//! Controller startup phase functions.
//!
//! Each public function maps to a distinct startup phase extracted from the
//! monolithic `run()` function. Intermediate results are passed between phases
//! via explicit structs ([`ReconciledSettings`], [`ValidatedConfig`],
//! [`PkiRuntime`]).

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use ipnet::IpNet;
use rootcause::prelude::*;
#[cfg(feature = "nats")]
use uptrakit_web_api::MaskedUrl;
use uptrakit_web_api::SettingKey;
use uptrakit_web_api::settings::Settings;

use crate::AppError;

// ---------------------------------------------------------------------------
// Intermediate result types
// ---------------------------------------------------------------------------

/// Values produced by reconciling CLI / DB / default settings.
pub(crate) struct ReconciledSettings {
    pub extra_sans: Vec<String>,
    pub pki_addr: Option<String>,
    pub https_addr: SocketAddr,
    /// Raw (decrypted) NATS URL, or `None` when not configured.
    /// Populated only when the `nats` feature is enabled.
    #[cfg(feature = "nats")]
    pub nats_url: Option<String>,
}

/// Configuration values validated after reconciliation.
pub(crate) struct ValidatedConfig {
    pub static_dir: Option<PathBuf>,
    pub pki_http_port: Option<u16>,
}

/// All PKI and TLS runtime state needed by `AppState` and background tasks.
pub(crate) struct PkiRuntime {
    pub ca_managed: bool,
    pub pki_path: PathBuf,
    pub ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    pub ca_rx: tokio::sync::watch::Receiver<crate::pki::CaSnapshot>,
    pub ca_key_store: uptrakit_web_api::CaKeyStoreRef,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub revocation_notify: Arc<tokio::sync::Notify>,
    pub ca_rotation_trigger: Arc<tokio::sync::Notify>,
    pub crl_pem_cache: Arc<tokio::sync::RwLock<String>>,
    pub crl_manager: Arc<crate::crl_manager::CrlManager>,
    pub initial_ca_version: i64,
}

// ---------------------------------------------------------------------------
// Phase 1: Master key initialization
// ---------------------------------------------------------------------------

/// Initialize the global master encryption key from env var or file.
///
/// Returns the raw hex string wrapped in [`SecretString`] if a master key was
/// loaded, `None` otherwise. The [`SecretString`] zeroes the hex on drop so
/// the key material is not retained in memory beyond its needed lifetime.
pub(crate) fn init_master_key(
    args: &crate::cli::Args,
) -> crate::Result<Option<uptrakit_internal_wire::SecretString>> {
    let env_val = std::env::var("UPTRAKIT_MASTER_KEY").ok();
    let key_hex = read_master_key_hex(args.master_key_file.as_deref(), env_val.as_deref())?;

    match key_hex {
        Some(key_hex) => {
            if args.allow_plaintext_secrets {
                tracing::warn!(
                    "--allow-plaintext-secrets is enabled. This flag is for development only; \
                    encryption remains enabled because a master key was provided."
                );
            }
            // Warn when the key is supplied via environment variable rather than a file.
            // Environment variables are visible in /proc/pid/environ, container manifests,
            // and orchestration tooling — use --master-key-file with mode 0o600 in production.
            if args.master_key_file.is_none() && env_val.is_some() {
                tracing::warn!(
                    "master encryption key loaded from UPTRAKIT_MASTER_KEY environment variable. \
                     For production deployments, prefer --master-key-file with a file readable \
                     only by the service user (mode 0o600). Environment variables are visible in \
                     /proc/pid/environ, container inspection output, and orchestration manifests."
                );
            }
            let key_bytes = parse_master_key_hex(&key_hex)?;
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).context_to()?;
            tracing::info!("master encryption key initialized");
            // Transfer the hex string into SecretString which also zeroizes on drop.
            // We clone via Deref<Target=String> so the Zeroizing wrapper scrubs its copy.
            let hex_for_secret = (*key_hex).clone();
            Ok(Some(uptrakit_internal_wire::SecretString::new(
                hex_for_secret,
            )))
        }
        None => {
            if args.allow_plaintext_secrets {
                tracing::warn!(
                    "master encryption key not set; encryption at rest is disabled. \
                    This is for development only and is NOT safe for production."
                );
                uptrakit_crypto::enable_plaintext_mode();
            } else {
                bail!(AppError::Config(
                    "master encryption key is required: set UPTRAKIT_MASTER_KEY env var \
                     (64-char hex string) or pass --master-key-file <path>. \
                     For development only, pass --allow-plaintext-secrets to run without \
                     encryption at rest."
                        .into()
                ));
            }
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3: Database initialization
// ---------------------------------------------------------------------------

/// Database initialization result.
pub(crate) struct DatabaseInit {
    pub conn: sea_orm::DatabaseConnection,
    pub default_tenant: uptrakit_shared_db::entity::tenant::Model,
    /// The resolved database URL (for credential delivery to external services).
    pub url: String,
}

/// Connect to the database, run migrations, and load the default tenant.
pub(crate) async fn init_database(
    args: &crate::cli::Args,
    state_dir: &std::path::Path,
) -> crate::Result<DatabaseInit> {
    let db_config =
        crate::db::DbConfig::from_args(args.db_url.clone(), state_dir, args.db_max_connections)
            .context(AppError::Database)?;
    tracing::info!(
        max_connections = db_config.max_connections,
        "connecting to database: {}",
        crate::db::sanitize_url(&db_config.url)
    );
    let db_conn = crate::db::connect(&db_config)
        .await
        .context(AppError::Database)?;

    tracing::info!("running database migrations");
    crate::migration::run_migrations(&db_conn)
        .await
        .context(AppError::Database)?;
    tracing::info!("database initialized successfully");

    let default_tenant = {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        use uptrakit_shared_db::entity::{prelude::Tenant, tenant};

        Tenant::find()
            .filter(tenant::Column::IsDefault.eq(true))
            .filter(tenant::Column::DeactivatedAt.is_null())
            .one(&db_conn)
            .await
            .context(AppError::Database)?
            .ok_or_else(|| report!(AppError::Database))?
    };

    Ok(DatabaseInit {
        conn: db_conn,
        default_tenant,
        url: db_config.url,
    })
}

/// Initialize a separate database connection for audit log storage.
///
/// Connects to the provided URL, runs the standard migrations (extra
/// empty application tables in the audit database are harmless), and
/// returns the connection for use by the audit log backend.
pub(crate) async fn init_audit_database(
    url: &str,
    max_connections: u32,
) -> crate::Result<sea_orm::DatabaseConnection> {
    let db_config = crate::db::DbConfig {
        url: url.to_string(),
        max_connections,
    };
    tracing::info!(
        max_connections,
        "connecting to audit log database: {}",
        crate::db::sanitize_url(url)
    );
    let conn = crate::db::connect(&db_config)
        .await
        .context(AppError::Database)?;

    tracing::info!("running audit log database migrations");
    crate::migration::run_migrations(&conn)
        .await
        .context(AppError::Database)?;
    tracing::info!("audit log database initialized");

    Ok(conn)
}

// ---------------------------------------------------------------------------
// Phase 4: Master key verification (HA safety check)
// ---------------------------------------------------------------------------

/// Verify the master key matches existing encrypted data, or store a new
/// verification token if this is the first run.
pub(crate) async fn verify_master_key(db: &sea_orm::DatabaseConnection) -> crate::Result<()> {
    if !uptrakit_crypto::master_key_available() {
        return Ok(());
    }

    let stored_token = uptrakit_web_api::settings_store::load_global_setting(
        db,
        SettingKey::MasterKeyVerification,
    )
    .await
    .context(AppError::Settings)?;

    match stored_token {
        Some(value) => {
            if let Some(token_str) = value.as_str()
                && uptrakit_crypto::is_encrypted(token_str)
            {
                uptrakit_crypto::verify_key_verification_token(token_str).map_err(|_| {
                    report!(AppError::Config(
                        "master key mismatch: the current UPTRAKIT_MASTER_KEY cannot \
                             decrypt data encrypted by a previous instance. Ensure all \
                             controller instances use the same master key."
                            .into()
                    ))
                })?;
                tracing::info!("master key verification succeeded");
            }
        }
        None => {
            let token = uptrakit_crypto::create_key_verification_token().context_to()?;
            let inserted = uptrakit_web_api::settings_store::insert_global_setting_if_absent(
                db,
                SettingKey::MasterKeyVerification,
                serde_json::json!(token),
            )
            .await
            .context(AppError::Settings)?;

            if inserted {
                tracing::info!("master key verification token stored");
            } else {
                // Another instance raced and stored a token first — verify against it.
                let raced_value = uptrakit_web_api::settings_store::load_global_setting(
                    db,
                    SettingKey::MasterKeyVerification,
                )
                .await
                .context(AppError::Settings)?;
                if let Some(value) = raced_value
                    && let Some(token_str) = value.as_str()
                    && uptrakit_crypto::is_encrypted(token_str)
                {
                    uptrakit_crypto::verify_key_verification_token(token_str).map_err(|_| {
                        report!(AppError::Config(
                            "master key mismatch: another controller instance stored a \
                                 verification token first, and the current UPTRAKIT_MASTER_KEY \
                                 cannot decrypt it. Ensure all controller instances use the \
                                 same master key."
                                .into()
                        ))
                    })?;
                    tracing::info!(
                        "master key verification succeeded (raced with another instance)"
                    );
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 4c: Data key ring initialization (envelope encryption)
// ---------------------------------------------------------------------------

/// Initialize the global [`DataKeyRing`] for envelope encryption.
///
/// Loads existing DEKs from the `data_encryption_keys` table, or generates the
/// first DEK if the table is empty.  The ring enables `ENC:v3:` format which
/// encrypts data with a DEK rather than the KEK directly.
///
/// ## HA safety
///
/// On first start two controllers may race to insert the initial DEK.  The
/// `key_id` column has a UNIQUE constraint, so the loser's insert fails
/// with a DB error that is caught gracefully.  It then re-reads all rows
/// and uses whatever was committed.
pub(crate) async fn init_data_key_ring(db: &sea_orm::DatabaseConnection) -> crate::Result<()> {
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use uptrakit_shared_db::entity::data_encryption_key;

    if !uptrakit_crypto::master_key_available() {
        return Ok(());
    }

    let kek_fp = uptrakit_crypto::master_key_fingerprint().context_to()?;

    let rows = data_encryption_key::Entity::find()
        .all(db)
        .await
        .context(AppError::Database)?;

    if rows.is_empty() {
        // First start — generate the initial DEK.
        let dek = uptrakit_crypto::generate_data_key().context_to()?;
        let wrapped = uptrakit_crypto::wrap_data_key(&dek).context_to()?;

        let am = data_encryption_key::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            key_id: Set(dek.key_id.clone()),
            wrapped_key: Set(wrapped),
            kek_fingerprint: Set(kek_fp.clone()),
            status: Set("active".to_string()),
            created_at: Set(time::OffsetDateTime::now_utc()),
            retired_at: Set(None),
        };

        match am.insert(db).await {
            Ok(_) => {
                tracing::info!(key_id = %dek.key_id, "generated initial data encryption key");
            }
            Err(e) => {
                // HA race: another controller inserted first.  This is harmless —
                // fall through to load all rows below.
                tracing::debug!(
                    error = %e,
                    "initial DEK insert failed (likely HA race), will load existing keys"
                );
            }
        }

        // Re-read in case of HA race.
        let rows = data_encryption_key::Entity::find()
            .all(db)
            .await
            .context(AppError::Database)?;
        return build_and_init_ring(&rows, &kek_fp);
    }

    build_and_init_ring(&rows, &kek_fp)?;
    Ok(())
}

/// Unwrap all DEKs and initialize the global data key ring.
fn build_and_init_ring(
    rows: &[uptrakit_shared_db::entity::data_encryption_key::Model],
    kek_fp: &str,
) -> crate::Result<()> {
    use std::collections::HashMap;

    let mut keys = HashMap::new();
    let mut active_key_id: Option<String> = None;

    for row in rows {
        if row.kek_fingerprint != kek_fp {
            return Err(report!(AppError::Config(format!(
                "DEK '{}' was wrapped with KEK fingerprint '{}', but current KEK \
                 fingerprint is '{kek_fp}'. This indicates a master key mismatch. \
                 Use --rotate-master-key-file to rotate to a new master key.",
                row.key_id, row.kek_fingerprint,
            ))));
        }

        let dek = uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to unwrap DEK '{}': {e}",
                row.key_id
            )))
        })?;
        keys.insert(dek.key_id.clone(), dek.key);

        if row.status == "active" {
            active_key_id = Some(row.key_id.clone());
        }
    }

    let active = active_key_id.ok_or_else(|| {
        report!(AppError::Config(
            "no active DEK found in data_encryption_keys table".into()
        ))
    })?;

    let ring = uptrakit_crypto::DataKeyRing::new(keys, active.clone());
    uptrakit_crypto::init_data_key_ring(ring).context_to()?;
    tracing::info!(
        active_key_id = %active,
        count = rows.len(),
        "data key ring initialized"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Master key rotation
// ---------------------------------------------------------------------------

/// Rotate the master key (KEK) by re-wrapping all DEKs with a new KEK.
///
/// This is an O(1) operation regardless of data volume: only the DEK
/// wrappers in `data_encryption_keys` are updated, not the encrypted data
/// itself (since data is encrypted with DEKs, not the KEK directly).
///
/// After rotation, the operator must restart all controllers with
/// `--master-key-file` pointing to the new key file.
pub(crate) async fn rotate_master_key(
    db: &sea_orm::DatabaseConnection,
    new_key_path: &std::path::Path,
) -> crate::Result<()> {
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, TransactionTrait};
    use uptrakit_shared_db::entity::data_encryption_key;

    // Read and parse the new key.
    let new_key_hex = std::fs::read_to_string(new_key_path).map_err(|e| {
        report!(AppError::Config(format!(
            "failed to read --rotate-master-key-file {}: {e}",
            new_key_path.display()
        )))
    })?;
    let new_key_bytes = parse_master_key_hex(new_key_hex.trim())?;
    let new_kek = zeroize::Zeroizing::new(new_key_bytes);

    // Compute the new KEK fingerprint.
    let new_kek_fp = {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(new_kek.as_slice());
        uptrakit_shared_types::hex::encode(&hash[..8])
    };

    let current_kek_fp = uptrakit_crypto::master_key_fingerprint().context_to()?;
    if new_kek_fp == current_kek_fp {
        tracing::warn!(
            "new master key has the same fingerprint as the current one — no rotation needed"
        );
        return Ok(());
    }

    tracing::info!(current_kek_fp, new_kek_fp, "starting master key rotation");

    // Re-wrap all DEKs in a transaction.
    let txn = db.begin().await.context(AppError::Database)?;

    let rows = data_encryption_key::Entity::find()
        .all(&txn)
        .await
        .context(AppError::Database)?;

    if rows.is_empty() {
        tracing::warn!("no DEKs found in database — nothing to rotate");
        txn.commit().await.context(AppError::Database)?;
        return Ok(());
    }

    for row in &rows {
        // Unwrap with current KEK.
        let dek = uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to unwrap DEK '{}' with current KEK: {e}",
                row.key_id
            )))
        })?;

        // Re-wrap with new KEK.
        let new_wrapped = uptrakit_crypto::wrap_data_key_with(&new_kek, &dek).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to wrap DEK '{}' with new KEK: {e}",
                row.key_id
            )))
        })?;

        // Update the row.
        let mut am: data_encryption_key::ActiveModel = row.clone().into_active_model();
        am.wrapped_key = sea_orm::Set(new_wrapped);
        am.kek_fingerprint = sea_orm::Set(new_kek_fp.clone());
        am.update(&txn).await.context(AppError::Database)?;
    }

    // Update the master key verification token with the new KEK.
    let new_token = uptrakit_crypto::create_verification_token_with_key(&new_kek).context_to()?;
    uptrakit_web_api::settings_store::upsert_global_setting(
        &txn,
        SettingKey::MasterKeyVerification,
        serde_json::json!(new_token),
    )
    .await
    .context(AppError::Settings)?;

    txn.commit().await.context(AppError::Database)?;

    tracing::info!(
        dek_count = rows.len(),
        new_kek_fp,
        "master key rotation complete — restart all controllers with the new key file"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 6: Settings reconciliation
// ---------------------------------------------------------------------------

/// Reconcile all DB-managed global settings with CLI values and update the
/// in-memory [`Settings`] object.
pub(crate) async fn reconcile_all_settings(
    db: &sea_orm::DatabaseConnection,
    args: &crate::cli::Args,
    settings: &Settings,
    global_raw: &uptrakit_web_api::settings_store::RawSettings,
) -> crate::Result<ReconciledSettings> {
    let force = args.force_settings_override;

    // Network settings
    let trusted_proxies = reconcile_setting_vec::<IpNet>(crate::reconcile::ReconcileParams {
        db,
        key: SettingKey::TrustedProxies,
        raw: global_raw,
        cli_value: if args.trusted_proxies.is_empty() {
            None
        } else {
            Some(args.trusted_proxies.clone())
        },
        default_value: vec![],
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v.iter().map(|n| n.to_string()).collect::<Vec<_>>()),
            from_json: |v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str()?.parse::<IpNet>().ok())
                        .collect()
                })
            },
        },
    })
    .await
    .context(AppError::Settings)?;
    settings.set_trusted_proxies(trusted_proxies.clone()).await;
    for cidr in &trusted_proxies {
        warn_broad_trusted_proxy(cidr);
    }

    let real_ip_header = crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
        db,
        key: SettingKey::RealIpHeader,
        raw: global_raw,
        cli_value: args.real_ip_header.clone(),
        default_value: uptrakit_web_api::settings::DEFAULT_REAL_IP_HEADER.to_string(),
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| v.as_str().map(String::from),
        },
    })
    .await
    .context(AppError::Settings)?;
    settings.set_real_ip_header(real_ip_header).await;

    let forwarded_cert_info_header =
        crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
            db,
            key: SettingKey::ForwardedClientCertInfoHeader,
            raw: global_raw,
            cli_value: args.forwarded_client_cert_info_header.clone(),
            default_value: String::new(),
            force,
            convert: crate::reconcile::JsonConvert {
                to_json: |v| {
                    if v.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!(v)
                    }
                },
                from_json: |v| {
                    if v.is_null() {
                        Some(String::new())
                    } else {
                        v.as_str().map(String::from)
                    }
                },
            },
        })
        .await
        .context(AppError::Settings)?;
    let forwarded_cert_info_opt = if forwarded_cert_info_header.is_empty() {
        None
    } else {
        Some(forwarded_cert_info_header)
    };
    settings
        .set_forwarded_client_cert_info_header(forwarded_cert_info_opt.clone())
        .await;

    let forwarded_cert_pem_header =
        crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
            db,
            key: SettingKey::ForwardedClientCertPemHeader,
            raw: global_raw,
            cli_value: args.forwarded_client_cert_pem_header.clone(),
            default_value: String::new(),
            force,
            convert: crate::reconcile::JsonConvert {
                to_json: |v| {
                    if v.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!(v)
                    }
                },
                from_json: |v| {
                    if v.is_null() {
                        Some(String::new())
                    } else {
                        v.as_str().map(String::from)
                    }
                },
            },
        })
        .await
        .context(AppError::Settings)?;
    let forwarded_cert_pem_opt = if forwarded_cert_pem_header.is_empty() {
        None
    } else {
        Some(forwarded_cert_pem_header)
    };
    settings
        .set_forwarded_client_cert_pem_header(forwarded_cert_pem_opt.clone())
        .await;

    let pki_addr = crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
        db,
        key: SettingKey::PkiAddr,
        raw: global_raw,
        cli_value: args.pki_addr.clone(),
        default_value: String::new(),
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| {
                if v.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(v)
                }
            },
            from_json: |v| {
                if v.is_null() {
                    Some(String::new())
                } else {
                    v.as_str().map(String::from)
                }
            },
        },
    })
    .await
    .context(AppError::Settings)?;
    let pki_addr_opt = if pki_addr.is_empty() {
        None
    } else {
        Some(pki_addr)
    };
    settings.set_pki_addr(pki_addr_opt.clone()).await;

    // Warn if cert headers are configured but no trusted proxies
    if (forwarded_cert_info_opt.is_some() || forwarded_cert_pem_opt.is_some())
        && trusted_proxies.is_empty()
    {
        tracing::warn!(
            "forwarded client cert header(s) configured but no --trusted-proxy set; \
             cert headers will be stripped from all requests"
        );
    }

    let extra_sans = reconcile_setting_vec::<String>(crate::reconcile::ReconcileParams {
        db,
        key: SettingKey::ExtraSans,
        raw: global_raw,
        cli_value: if args.sans.is_empty() {
            None
        } else {
            Some(args.sans.clone())
        },
        default_value: vec![],
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
            },
        },
    })
    .await
    .context(AppError::Settings)?;
    settings.set_extra_sans(extra_sans.clone()).await;

    let https_addr = reconcile_socket_addr(
        db,
        SettingKey::HttpsAddr,
        global_raw,
        args.https_addr,
        uptrakit_web_api::settings::DEFAULT_HTTPS_ADDR
            .parse()
            .map_err(|e| {
                report!(AppError::Config(format!(
                    "invalid default HTTPS address constant: {e}"
                ),))
            })?,
        force,
    )
    .await?;
    settings.set_https_addr(https_addr).await;

    // NATS URL reconciliation (only when nats feature is enabled)
    #[cfg(feature = "nats")]
    let nats_url = {
        let nats_url_raw = crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
            db,
            key: SettingKey::NatsUrl,
            raw: global_raw,
            cli_value: args.nats_url.clone(),
            default_value: String::new(),
            force,
            convert: crate::reconcile::JsonConvert {
                to_json: |v| {
                    if v.is_empty() {
                        return serde_json::Value::Null;
                    }
                    uptrakit_crypto::encrypt_str(v, "uptrakit:settings:nats_url")
                        .map(|e| serde_json::json!(e))
                        .unwrap_or_else(|_| serde_json::json!(v))
                },
                from_json: |v| {
                    let s = v.as_str().filter(|s| !s.is_empty())?;
                    if uptrakit_crypto::is_encrypted(s) {
                        uptrakit_crypto::decrypt_str(s, "uptrakit:settings:nats_url")
                            .map_err(|e| {
                                tracing::warn!("failed to decrypt nats.url: {e}");
                            })
                            .ok()
                    } else {
                        Some(s.to_string())
                    }
                },
            },
        })
        .await
        .context(AppError::Settings)?;

        let nats_url_opt: Option<String> = if nats_url_raw.is_empty() {
            None
        } else {
            Some(nats_url_raw)
        };
        settings
            .set_nats_url(nats_url_opt.as_deref().map(MaskedUrl::new))
            .await;
        nats_url_opt
    };

    Ok(ReconciledSettings {
        extra_sans,
        pki_addr: pki_addr_opt,
        https_addr,
        #[cfg(feature = "nats")]
        nats_url,
    })
}

// ---------------------------------------------------------------------------
// Phase 7: OIDC bootstrap
// ---------------------------------------------------------------------------

/// Bootstrap an OIDC provider from CLI flags if all required flags are present.
pub(crate) async fn bootstrap_oidc(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    args: &crate::cli::Args,
) -> crate::Result<()> {
    let oidc = &args.oidc_bootstrap;
    let any_set = oidc.oidc_issuer_url.is_some()
        || oidc.oidc_client_id.is_some()
        || oidc.oidc_client_secret.is_some();

    if !any_set {
        return Ok(());
    }

    let issuer_url = oidc.oidc_issuer_url.as_deref().ok_or_else(|| {
        report!(AppError::Config(
            "--oidc-issuer-url is required when any OIDC bootstrap flag is set".into()
        ))
    })?;
    let client_id = oidc.oidc_client_id.as_deref().ok_or_else(|| {
        report!(AppError::Config(
            "--oidc-client-id is required with --oidc-issuer-url".into()
        ))
    })?;
    let client_secret = oidc.oidc_client_secret.as_deref().ok_or_else(|| {
        report!(AppError::Config(
            "--oidc-client-secret is required with --oidc-issuer-url".into()
        ))
    })?;

    let slug = oidc.oidc_provider_slug.as_deref().unwrap_or("sso");
    let name = oidc.oidc_provider_name.as_deref().unwrap_or("SSO");
    let scopes = oidc
        .oidc_scopes
        .as_deref()
        .unwrap_or("openid email profile groups");

    let force = args.force_settings_override;

    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use uptrakit_shared_db::entity::{oidc_provider, prelude::OidcProvider};

    let existing = OidcProvider::find()
        .filter(oidc_provider::Column::Slug.eq(slug))
        .filter(oidc_provider::Column::TenantId.eq(tenant_id))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context(AppError::Database)?;

    match existing {
        None => {
            use sea_orm::ActiveModelTrait;
            use sea_orm::Set;
            use time::OffsetDateTime;

            let now = OffsetDateTime::now_utc();
            let encrypted_secret = uptrakit_crypto::EncryptedString::new(
                client_secret.to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .context_to()?;
            let provider = oidc_provider::ActiveModel {
                id: Set(uuid::Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                name: Set(name.to_string()),
                slug: Set(slug.to_string()),
                logo_url: Set(None),
                issuer_url: Set(issuer_url.to_string()),
                client_id: Set(client_id.to_string()),
                client_secret: Set(encrypted_secret),
                scopes: Set(scopes.to_string()),
                auto_create_users: Set(true),
                role_claim_path: Set(None),
                role_mapping: Set(
                    uptrakit_shared_db::entity::oidc_provider::RoleMapping::default(),
                ),
                is_active: Set(true),
                created_at: Set(now),
                updated_at: Set(now),
                deactivated_at: Set(None),
            };
            provider.insert(db).await.context(AppError::Database)?;
            tracing::info!(slug = slug, name = name, "bootstrapped OIDC provider");
        }
        Some(existing_provider) if force => {
            use sea_orm::Set;
            use sea_orm::{ActiveModelTrait, IntoActiveModel};
            use time::OffsetDateTime;

            let encrypted_secret = uptrakit_crypto::EncryptedString::new(
                client_secret.to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .context_to()?;
            let mut model = existing_provider.into_active_model();
            model.issuer_url = Set(issuer_url.to_string());
            model.client_id = Set(client_id.to_string());
            model.client_secret = Set(encrypted_secret);
            model.is_active = Set(true);
            model.updated_at = Set(OffsetDateTime::now_utc());
            model.update(db).await.context(AppError::Database)?;
            tracing::info!(
                slug = slug,
                name = name,
                "force-updated bootstrapped OIDC provider"
            );
        }
        Some(_) => {
            tracing::info!(
                slug = slug,
                "OIDC provider already exists, skipping bootstrap \
                 (pass --force-settings-override to overwrite)"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 7b: Enrollment token bootstrap
// ---------------------------------------------------------------------------

/// Bootstrap enrollment tokens from CLI flags / environment variables.
///
/// Creates pre-hashed enrollment tokens named "bootstrap" at startup so that
/// services can auto-enroll using a shared secret (e.g. in docker-compose).
/// Idempotent: skips creation if an active token named "bootstrap" already
/// exists (not revoked, not expired, uses remaining).
pub(crate) async fn bootstrap_enrollment_tokens(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    args: &crate::cli::Args,
) -> crate::Result<()> {
    let eb = &args.enrollment_bootstrap;

    // Tenant enrollment token
    if let Some(ref token_value) = eb.bootstrap_enrollment_token {
        bootstrap_tenant_enrollment_token(db, tenant_id, token_value, eb).await?;
    }

    // System enrollment token
    if let Some(ref token_value) = eb.bootstrap_system_enrollment_token {
        bootstrap_system_enrollment_token(db, token_value, eb).await?;
    }

    Ok(())
}

/// Create a tenant enrollment token named "bootstrap" if none exists.
async fn bootstrap_tenant_enrollment_token(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    token_value: &str,
    eb: &crate::cli::EnrollmentBootstrapArgs,
) -> crate::Result<()> {
    use sea_orm::{ColumnTrait, EntityTrait, ExprTrait, QueryFilter, sea_query::Expr};
    use uptrakit_shared_db::entity::enrollment_token;

    let now = time::OffsetDateTime::now_utc();

    // Check for an existing active token named "bootstrap"
    let existing = enrollment_token::Entity::find()
        .filter(enrollment_token::Column::TenantId.eq(tenant_id))
        .filter(enrollment_token::Column::Name.eq("bootstrap"))
        .filter(enrollment_token::Column::RevokedAt.is_null())
        .filter(
            enrollment_token::Column::ExpiresAt
                .is_null()
                .or(enrollment_token::Column::ExpiresAt.gt(now)),
        )
        .filter(
            enrollment_token::Column::MaxUses
                .is_null()
                .or(Expr::col(enrollment_token::Column::CurrentUses)
                    .lt(Expr::col(enrollment_token::Column::MaxUses))),
        )
        .one(db)
        .await
        .context(AppError::Database)?;

    if existing.is_some() {
        tracing::info!("bootstrap enrollment token already exists, skipping");
        return Ok(());
    }

    let hash = uptrakit_web_api::auth::password::hash_password(token_value).map_err(|e| {
        report!(AppError::Config(format!(
            "failed to hash bootstrap enrollment token: {e}"
        )))
    })?;

    let expires_at = now + time::Duration::seconds(eb.bootstrap_enrollment_token_ttl as i64);

    use sea_orm::{ActiveModelTrait, Set};

    let model = enrollment_token::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        name: Set("bootstrap".to_string()),
        token_hash: Set(hash.expose_secret().to_string()),
        allowed_capabilities: Set(None),
        max_uses: Set(Some(eb.bootstrap_enrollment_token_max_uses as i32)),
        current_uses: Set(0),
        expires_at: Set(Some(expires_at)),
        created_at: Set(now),
        revoked_at: Set(None),
        created_by_user_id: Set(None),
    };
    model.insert(db).await.context(AppError::Database)?;

    tracing::info!(
        max_uses = eb.bootstrap_enrollment_token_max_uses,
        ttl_secs = eb.bootstrap_enrollment_token_ttl,
        "bootstrapped tenant enrollment token"
    );
    Ok(())
}

/// Create a system enrollment token named "bootstrap" if none exists.
async fn bootstrap_system_enrollment_token(
    db: &sea_orm::DatabaseConnection,
    token_value: &str,
    eb: &crate::cli::EnrollmentBootstrapArgs,
) -> crate::Result<()> {
    use sea_orm::{ColumnTrait, EntityTrait, ExprTrait, QueryFilter, sea_query::Expr};
    use uptrakit_shared_db::entity::system_enrollment_token;

    let now = time::OffsetDateTime::now_utc();

    // Check for an existing active token named "bootstrap"
    let existing = system_enrollment_token::Entity::find()
        .filter(system_enrollment_token::Column::Name.eq("bootstrap"))
        .filter(system_enrollment_token::Column::RevokedAt.is_null())
        .filter(
            system_enrollment_token::Column::ExpiresAt
                .is_null()
                .or(system_enrollment_token::Column::ExpiresAt.gt(now)),
        )
        .filter(
            system_enrollment_token::Column::MaxUses
                .is_null()
                .or(Expr::col(system_enrollment_token::Column::CurrentUses)
                    .lt(Expr::col(system_enrollment_token::Column::MaxUses))),
        )
        .one(db)
        .await
        .context(AppError::Database)?;

    if existing.is_some() {
        tracing::info!("bootstrap system enrollment token already exists, skipping");
        return Ok(());
    }

    let hash = uptrakit_web_api::auth::password::hash_password(token_value).map_err(|e| {
        report!(AppError::Config(format!(
            "failed to hash bootstrap system enrollment token: {e}"
        )))
    })?;

    let expires_at = now + time::Duration::seconds(eb.bootstrap_system_enrollment_token_ttl as i64);

    use sea_orm::{ActiveModelTrait, Set};

    let model = system_enrollment_token::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        name: Set("bootstrap".to_string()),
        token_hash: Set(hash.expose_secret().to_string()),
        max_uses: Set(Some(eb.bootstrap_system_enrollment_token_max_uses as i32)),
        current_uses: Set(0),
        expires_at: Set(Some(expires_at)),
        created_at: Set(now),
        revoked_at: Set(None),
        created_by_user_id: Set(None),
    };
    model.insert(db).await.context(AppError::Database)?;

    tracing::info!(
        max_uses = eb.bootstrap_system_enrollment_token_max_uses,
        ttl_secs = eb.bootstrap_system_enrollment_token_ttl,
        "bootstrapped system enrollment token"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 8: Configuration validation
// ---------------------------------------------------------------------------

/// Validate TLS, CA, SAN, and PKI HTTP args.  Resolve the static directory.
pub(crate) fn validate_configuration(
    args: &crate::cli::Args,
    reconciled: &ReconciledSettings,
) -> crate::Result<ValidatedConfig> {
    // If --static-dir is given explicitly, always resolve and use it (overrides embedded assets).
    // Without an explicit path: auto-detect only when embed-frontend is not compiled in.
    let static_dir = if args.static_dir.is_some() || !cfg!(feature = "embed-frontend") {
        resolve_static_dir(args.static_dir.clone())?
    } else {
        None
    };

    // Validate TLS args
    if args.tls_cert.is_some() != args.tls_key.is_some() {
        bail!(AppError::Config(
            "both --tls-cert and --tls-key must be provided together".into()
        ));
    }

    // --san only makes sense with managed (auto-generated) certificates
    if !reconciled.extra_sans.is_empty() && args.tls_cert.is_some() {
        bail!(AppError::Config(
            "--san cannot be used with --tls-cert/--tls-key; \
             SANs are only configurable for controller-managed certificates"
                .into()
        ));
    }

    // Validate CA args
    if args.ca_cert.is_some() != args.ca_key.is_some() {
        bail!(AppError::Config(
            "both --ca-cert and --ca-key must be provided together".into()
        ));
    }

    // Validate --pki-http
    let pki_http_port: Option<u16> = if let Some(mode) = args.pki_http {
        let pki_url = reconciled.pki_addr.as_deref().ok_or_else(|| {
            report!(AppError::Config(
                "--pki-http requires --pki-addr to be set".into()
            ))
        })?;
        match mode {
            crate::cli::PkiHttpMode::Listener => {
                let parsed: url::Url = pki_url.parse().map_err(|e| {
                    report!(AppError::Config(format!(
                        "--pki-addr URL is not valid: {e}"
                    )))
                })?;
                let port = parsed.port_or_known_default().ok_or_else(|| {
                    report!(AppError::Config(
                        "--pki-addr URL must have an explicit or default port".into()
                    ))
                })?;
                Some(port)
            }
            crate::cli::PkiHttpMode::External => None,
        }
    } else {
        if let Some(ref url) = reconciled.pki_addr
            && url.starts_with("http://")
        {
            tracing::warn!(
                "--pki-addr uses http:// scheme but --pki-http is not set; \
                 the controller is NOT serving PKI endpoints over plain HTTP. \
                 Add --pki-http listener to start the HTTP listener, or \
                 --pki-http external if PKI HTTP is handled by a reverse proxy."
            );
        }
        None
    };

    Ok(ValidatedConfig {
        static_dir,
        pki_http_port,
    })
}

// ---------------------------------------------------------------------------
// Phase 9: PKI + TLS initialization
// ---------------------------------------------------------------------------

/// Initialize the entire PKI subsystem: CA state, server certificate,
/// CRL manager, and TLS configuration.
pub(crate) async fn init_pki_runtime(
    args: &crate::cli::Args,
    db: &sea_orm::DatabaseConnection,
    config_dir: &std::path::Path,
    reconciled: &ReconciledSettings,
) -> crate::Result<PkiRuntime> {
    use crate::pki;

    let pki_path = pki::pki_dir(config_dir).context(AppError::Pki)?;

    // Load CA state
    let ca_state = if let (Some(ca_cert_path), Some(ca_key_path)) = (&args.ca_cert, &args.ca_key) {
        // External CA — not managed
        let ca = pki::load_external_ca(ca_cert_path, ca_key_path).context(AppError::Pki)?;
        let trusted = vec![
            pki::bundle_from_pem(ca.cert_pem.clone(), ca.key_pem.clone()).context(AppError::Pki)?,
        ];
        pki::CaState {
            active: ca,
            previous: None,
            trusted,
            managed: false,
        }
    } else {
        let mut state = pki::load_or_init_managed_ca(db, reconciled.pki_addr.as_deref())
            .await
            .context(AppError::Pki)?;

        if pki::should_rotate_ca(&state.active.cert_pem) {
            tracing::info!("CA certificate is within rotation window, rotating now");
            let active_fp = pki::ca_fingerprint(&state.active.cert_pem).context(AppError::Pki)?;
            let rotation = pki::rotate_managed_ca(db, reconciled.pki_addr.as_deref(), &active_fp)
                .await
                .context(AppError::Pki)?;
            state = rotation.state;
        }

        state
    };

    // Validate CA extensions match pki_addr (managed CAs only)
    if ca_state.managed {
        pki::validate_ca_pki_addr(&ca_state.active.cert_pem, reconciled.pki_addr.as_deref())
            .context(AppError::Pki)?;
    }

    let (ca_snapshot, ca_initial_key_store) = ca_state
        .to_snapshot(reconciled.pki_addr.clone())
        .context(AppError::Pki)?;
    let (ca_tx, ca_rx) = tokio::sync::watch::channel(ca_snapshot.clone());
    let ca_key_store: uptrakit_web_api::CaKeyStoreRef =
        Arc::new(tokio::sync::RwLock::new(ca_initial_key_store));

    // Resolve server certificate (using reconciled extra_sans)
    let server_cert = if let (Some(cert_path), Some(key_path)) = (&args.tls_cert, &args.tls_key) {
        pki::load_external_cert(cert_path, key_path).context(AppError::Pki)?
    } else {
        let mut cert =
            pki::load_or_generate_server_cert(&pki_path, &ca_state.active, &reconciled.extra_sans)
                .await
                .context(AppError::Pki)?;

        // Check if the existing cert needs SAN regeneration
        if pki::server_cert_needs_san_update(&cert.cert_pem, &reconciled.extra_sans)
            .context(AppError::Pki)?
        {
            if pki::cert_signed_by_ca(&cert.cert_pem, &ca_state.active.cert_pem)
                .context(AppError::Pki)?
            {
                tracing::info!(
                    "server certificate SANs do not match configured values, regenerating"
                );
                cert = pki::renew_server_cert(&pki_path, &ca_state.active, &reconciled.extra_sans)
                    .await
                    .context(AppError::Pki)?;
            } else {
                bail!(AppError::Config(
                    "The server certificate does not include the requested SANs and was signed by \
                     a different CA than the currently active one.\n\n\
                     To fix this:\n  \
                     1. Restart the controller without the --san flag(s) that are not yet in the certificate\n  \
                     2. Regenerate the server certificate via POST /api/v1/settings/renew-server-certificate or the UI\n  \
                     3. Restart the controller with the desired --san flag(s)"
                        .into()
                ));
            }
        }

        // Auto-renew if within renewal window
        if pki::should_renew_server_cert(&cert.cert_pem) {
            tracing::info!("server certificate is within renewal window, renewing now");
            cert = pki::renew_server_cert(&pki_path, &ca_state.active, &reconciled.extra_sans)
                .await
                .context(AppError::Pki)?;
        }

        cert
    };

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let revocation_notify = Arc::new(tokio::sync::Notify::const_new());
    let ca_rotation_trigger = Arc::new(tokio::sync::Notify::const_new());

    // Build initial CRLs — tries DB cache first, generates fresh if missing/stale.
    let crl_pem_cache = Arc::new(tokio::sync::RwLock::new(String::new()));
    let (initial_crls, initial_crl_pem, starting_crl_number) = {
        let ks = ca_key_store.read().await;
        crate::crl_manager::build_initial_crls(db, &ca_snapshot, &ks)
            .await
            .context(AppError::Pki)?
    };
    *crl_pem_cache.write().await = initial_crl_pem;

    // Build initial server config with CRLs
    let initial_server_config = pki::build_rustls_config_with_client_auth_and_crls(
        &server_cert.cert_pem,
        &server_cert.key_pem,
        &ca_snapshot.bundle_pem,
        initial_crls,
    )
    .context(AppError::Pki)?;

    let rustls_config =
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(initial_server_config));

    // Create CRL manager
    let crl_manager = Arc::new({
        let ks = ca_key_store.read().await;
        crate::crl_manager::CrlManager::new(
            crate::crl_manager::CrlManagerConfig {
                server_cert_pem: server_cert.cert_pem.clone(),
                server_key_pem: server_cert.key_pem.clone(),
                db: db.clone(),
                rustls_config: rustls_config.clone(),
                revocation_notify: Arc::clone(&revocation_notify),
                crl_pem_cache: Arc::clone(&crl_pem_cache),
            },
            &ca_snapshot,
            &ks,
            starting_crl_number,
        )
        .context(AppError::Pki)?
    });

    let initial_ca_version = if ca_state.managed {
        pki::load_ca_version(db).await.context(AppError::Pki)?
    } else {
        0
    };

    Ok(PkiRuntime {
        ca_managed: ca_state.managed,
        pki_path,
        ca_tx,
        ca_rx,
        ca_key_store,
        rustls_config,
        revocation_notify,
        ca_rotation_trigger,
        crl_pem_cache,
        crl_manager,
        initial_ca_version,
    })
}

// ---------------------------------------------------------------------------
// Phase 10: JWT initialization
// ---------------------------------------------------------------------------

/// Migrate file-based JWT key (if present) and load or generate the DB-stored JWT signing key.
pub(crate) async fn init_jwt(
    db: &sea_orm::DatabaseConnection,
    state_dir: &std::path::Path,
) -> crate::Result<uptrakit_web_api::auth::jwt::JwtManager> {
    uptrakit_web_api::settings_store::migrate_file_jwt_key(db, state_dir)
        .await
        .context(AppError::Config("JWT key migration failed".into()))?;

    let jwt_secret = uptrakit_web_api::settings_store::load_or_generate_jwt_key(db)
        .await
        .context(AppError::Config("JWT key initialization failed".into()))?;

    let jwt_manager = uptrakit_web_api::auth::jwt::JwtManager::from_secret(&jwt_secret);
    tracing::info!("JWT signing key initialized from database");
    Ok(jwt_manager)
}

// ---------------------------------------------------------------------------
// Private helpers (moved from main.rs)
// ---------------------------------------------------------------------------

pub(crate) fn read_master_key_hex(
    master_key_file: Option<&std::path::Path>,
    env_val: Option<&str>,
) -> crate::Result<Option<zeroize::Zeroizing<String>>> {
    if let Some(key_file) = master_key_file {
        let contents = std::fs::read_to_string(key_file).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to read --master-key-file {}: {e}",
                key_file.display()
            )))
        })?;
        return Ok(Some(zeroize::Zeroizing::new(contents.trim().to_string())));
    }

    if let Some(env_val) = env_val {
        return Ok(Some(zeroize::Zeroizing::new(env_val.trim().to_string())));
    }

    Ok(None)
}

pub(crate) fn parse_master_key_hex(key_hex: &str) -> crate::Result<[u8; 32]> {
    let bytes = uptrakit_shared_types::hex::decode(key_hex).map_err(|e| {
        report!(AppError::Config(format!(
            "master key must be a 64-character hex string: {e}"
        )))
    })?;
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
        report!(AppError::Config(format!(
            "master key must be exactly 32 bytes (64 hex chars), got {} bytes",
            v.len()
        )))
    })?;
    Ok(key_bytes)
}

/// Resolve the static directory for SPA serving.
///
/// If `--static-dir` is given, validates that it contains `index.html`.
/// Otherwise, auto-detects by probing `frontend/build` and `frontend`
/// relative to the current working directory.
///
/// This function is always compiled so that `--static-dir` can override the
/// embedded frontend assets even when the `embed-frontend` feature is active.
fn resolve_static_dir(explicit: Option<PathBuf>) -> crate::Result<Option<PathBuf>> {
    if let Some(dir) = explicit {
        let index = dir.join("index.html");
        if !index.is_file() {
            bail!(AppError::Config(format!(
                "--static-dir {}: missing index.html",
                dir.display()
            )));
        }
        tracing::info!("serving static files from {}", dir.display());
        return Ok(Some(dir));
    }

    for candidate in ["frontend/build", "frontend"] {
        let dir = PathBuf::from(candidate);
        if dir.join("index.html").is_file() {
            tracing::info!("auto-detected static files in {}", dir.display());
            return Ok(Some(dir));
        }
    }

    Ok(None)
}

/// Check whether a trusted proxy CIDR is overly broad and emit a warning.
///
/// Broad CIDRs in the trusted-proxy list effectively trust a large portion of the
/// internet to set the `X-Forwarded-For` header, which undermines IP-based rate
/// limiting and audit logging.
fn warn_broad_trusted_proxy(cidr: &IpNet) {
    let prefix = cidr.prefix_len();
    if prefix == 0 {
        tracing::warn!(
            cidr = %cidr,
            "trusted proxy CIDR has prefix length /0 — this trusts ALL IP addresses to set \
             forwarded headers, effectively disabling IP-based security controls"
        );
    } else if is_broad_trusted_proxy(cidr) {
        tracing::warn!(
            cidr = %cidr,
            "trusted proxy CIDR is very broad — consider narrowing to specific proxy subnets"
        );
    }
}

/// Returns `true` when the CIDR is broad enough to warrant a security warning.
///
/// Thresholds: IPv4 /8 or less, IPv6 /32 or less, or /0 for either family.
fn is_broad_trusted_proxy(cidr: &IpNet) -> bool {
    let prefix = cidr.prefix_len();
    match cidr {
        IpNet::V4(_) => prefix <= 8,
        IpNet::V6(_) => prefix <= 32,
    }
}

/// Wrapper for `&[T]` that implements Display for logging in reconciliation.
struct DisplayVec<'a, T: fmt::Display>(&'a [T]);

impl<T: fmt::Display> fmt::Display for DisplayVec<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "[]")
        } else {
            let items: Vec<String> = self.0.iter().map(|i| i.to_string()).collect();
            write!(f, "[{}]", items.join(", "))
        }
    }
}

/// Reconcile a `Vec<T>` global setting.  Empty CLI vec is treated as "not provided".
async fn reconcile_setting_vec<T>(
    params: crate::reconcile::ReconcileParams<'_, Vec<T>>,
) -> crate::reconcile::Result<Vec<T>>
where
    T: PartialEq + Clone + fmt::Display + 'static,
{
    let crate::reconcile::ReconcileParams {
        db,
        key,
        raw,
        cli_value,
        default_value,
        force,
        convert,
    } = params;
    let db_key = key.as_str();
    let db_value = raw.get(db_key).and_then(convert.from_json);

    match (db_value, cli_value) {
        (Some(db_val), Some(cli_val)) if db_val != cli_val => {
            if force {
                tracing::info!(key = db_key, cli = %DisplayVec(&cli_val), db = %DisplayVec(&db_val), "force-overriding DB setting with CLI value");
                uptrakit_web_api::settings_store::upsert_global_setting(
                    db,
                    key,
                    (convert.to_json)(&cli_val),
                )
                .await
                .map_err(|e| {
                    tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                    rootcause::report!(crate::reconcile::ReconcileError)
                })?;
                Ok(cli_val)
            } else {
                tracing::warn!(
                    key = db_key,
                    cli = %DisplayVec(&cli_val),
                    db = %DisplayVec(&db_val),
                    "CLI value differs from DB; using DB value (pass --force-settings-override to overwrite)"
                );
                Ok(db_val)
            }
        }
        (Some(db_val), _) => {
            tracing::debug!(key = db_key, value = %DisplayVec(&db_val), "using DB value");
            Ok(db_val)
        }
        (None, Some(cli_val)) => {
            tracing::info!(key = db_key, value = %DisplayVec(&cli_val), "seeding DB setting from CLI");
            uptrakit_web_api::settings_store::upsert_global_setting(
                db,
                key,
                (convert.to_json)(&cli_val),
            )
            .await
            .map_err(|e| {
                tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                rootcause::report!(crate::reconcile::ReconcileError)
            })?;
            Ok(cli_val)
        }
        (None, None) => {
            tracing::info!(key = db_key, value = %DisplayVec(&default_value), "seeding DB setting from default");
            uptrakit_web_api::settings_store::upsert_global_setting(
                db,
                key,
                (convert.to_json)(&default_value),
            )
            .await
            .map_err(|e| {
                tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                rootcause::report!(crate::reconcile::ReconcileError)
            })?;
            Ok(default_value)
        }
    }
}

/// Reconcile a `SocketAddr` global setting.
async fn reconcile_socket_addr(
    db: &sea_orm::DatabaseConnection,
    key: SettingKey,
    raw: &uptrakit_web_api::settings_store::RawSettings,
    cli_value: Option<SocketAddr>,
    default_value: SocketAddr,
    force: bool,
) -> crate::Result<SocketAddr> {
    crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
        db,
        key,
        raw,
        cli_value,
        default_value,
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v.to_string()),
            from_json: |v| v.as_str().and_then(|s| s.parse().ok()),
        },
    })
    .await
    .context(AppError::Settings)
}

// ---------------------------------------------------------------------------
// Tests (moved from main.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{parse_master_key_hex, read_master_key_hex};
    use std::io::Write;

    #[test]
    fn missing_key_returns_none() {
        let result = read_master_key_hex(None, None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn env_key_is_trimmed() {
        let result = read_master_key_hex(None, Some("  deadbeef  "));
        assert!(matches!(result, Ok(Some(ref value)) if value.as_str() == "deadbeef"));
    }

    #[test]
    fn file_key_is_trimmed() {
        let file = tempfile::NamedTempFile::new();
        assert!(file.is_ok());
        let mut file = match file {
            Ok(file) => file,
            Err(_) => return,
        };
        assert!(file.write_all(b"  0123  ").is_ok());
        let result = read_master_key_hex(Some(file.path()), None);
        assert!(matches!(result, Ok(Some(ref value)) if value.as_str() == "0123"));
    }

    #[test]
    fn read_master_key_hex_returns_zeroizing() {
        let result = read_master_key_hex(None, Some("abcdef")).unwrap().unwrap();
        assert_eq!(result.as_str(), "abcdef");
    }

    #[test]
    fn parse_master_key_rejects_invalid_hex() {
        let result = parse_master_key_hex("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_rejects_invalid_length() {
        let result = parse_master_key_hex("aa");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_accepts_valid_length() {
        let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_master_key_hex(key_hex);
        assert!(matches!(result, Ok(bytes) if bytes.len() == 32));
    }

    // ── Broad trusted proxy detection ────────────────────────────────

    #[test]
    fn broad_proxy_ipv4_slash_zero() {
        let cidr: ipnet::IpNet = "0.0.0.0/0".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn broad_proxy_ipv6_slash_zero() {
        let cidr: ipnet::IpNet = "::/0".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn broad_proxy_ipv4_slash_eight() {
        let cidr: ipnet::IpNet = "10.0.0.0/8".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn narrow_proxy_ipv4_slash_sixteen() {
        let cidr: ipnet::IpNet = "192.168.0.0/16".parse().unwrap();
        assert!(!super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn narrow_proxy_ipv4_slash_thirtytwo() {
        let cidr: ipnet::IpNet = "127.0.0.1/32".parse().unwrap();
        assert!(!super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn broad_proxy_ipv6_slash_thirtytwo() {
        let cidr: ipnet::IpNet = "2001:db8::/32".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn narrow_proxy_ipv6_slash_fortyeight() {
        let cidr: ipnet::IpNet = "2001:db8:abcd::/48".parse().unwrap();
        assert!(!super::is_broad_trusted_proxy(&cidr));
    }
}
