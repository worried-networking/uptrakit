mod cert_signer;
mod cli;
mod crl_manager;
mod db;
mod migration;
mod mtls_acceptor;
mod pki;
mod reconcile;
mod server;

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use ipnet::IpNet;
use rootcause::prelude::*;
use thiserror::Error;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use uptrakit_web_api::AppState;
use uptrakit_web_api::SettingKey;
use uptrakit_web_api::settings::Settings;

#[derive(Debug, Error)]
enum AppError {
    #[error("{0}")]
    Config(String),

    #[error("database initialization failed")]
    Database,

    #[error("settings initialization failed")]
    Settings,

    #[error("PKI initialization failed")]
    Pki,

    #[error("server error")]
    Server,
}

type Result<T> = std::result::Result<T, rootcause::Report<AppError>>;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = cli::Args::parse();

    if let Err(report) = run(args).await {
        eprintln!("Error: {report:?}");
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

async fn run(args: cli::Args) -> Result<()> {
    // Initialize master encryption key (required for credential encryption at rest)
    {
        let env_val = std::env::var("UPTRAKIT_MASTER_KEY").ok();
        let key_hex = read_master_key_hex(args.master_key_file.as_deref(), env_val.as_deref())
            .map_err(|e| report!(AppError::Config(e)))?;

        match key_hex {
            Some(key_hex) => {
                if args.allow_plaintext_secrets {
                    tracing::warn!(
                        "--allow-plaintext-secrets is enabled. This flag is for development only; \
                        encryption remains enabled because a master key was provided."
                    );
                }
                let key_bytes =
                    parse_master_key_hex(&key_hex).map_err(|e| report!(AppError::Config(e)))?;
                uptrakit_shared_db::crypto::init_master_key(key_bytes).map_err(|e| {
                    report!(AppError::Config(format!(
                        "failed to initialize master key: {e}"
                    )))
                })?;
                tracing::info!("master encryption key initialized");
            }
            None => {
                if args.allow_plaintext_secrets {
                    tracing::warn!(
                        "master encryption key not set; encryption at rest is disabled. \
                        This is for development only and is NOT safe for production."
                    );
                } else {
                    return Err(report!(AppError::Config(
                        "master encryption key is required: set UPTRAKIT_MASTER_KEY env var \
                         (64-char hex string) or pass --master-key-file <path>. \
                         For development only, pass --allow-plaintext-secrets to run without \
                         encryption at rest."
                            .into()
                    )));
                }
            }
        }
    }

    // Resolve application directories (config and state)
    let app_dirs = args.resolve_dirs().map_err(|e| {
        report!(AppError::Config(format!(
            "failed to resolve directories: {e}"
        )))
    })?;
    app_dirs.ensure_dirs().map_err(|e| {
        report!(AppError::Config(format!(
            "failed to create directories: {e}"
        )))
    })?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());

    // Initialize database (state directory for SQLite DB)
    let db_config =
        db::DbConfig::from_args(args.db_url, app_dirs.state_dir()).context(AppError::Database)?;
    tracing::info!(
        "connecting to database: {}",
        db::sanitize_url(&db_config.url)
    );
    let db_conn = db::connect(&db_config.url)
        .await
        .context(AppError::Database)?;

    tracing::info!("running database migrations");
    migration::run_migrations(&db_conn)
        .await
        .context(AppError::Database)?;

    tracing::info!("database initialized successfully");

    // Load the default tenant (seeded by the initial migration)
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
    let default_tenant_id = default_tenant.id;
    tracing::info!(%default_tenant_id, "loaded default tenant");

    // Initialize settings (bulk-loads all values from DB in a single query)
    let (settings, raw_settings, reg_token) = Settings::load(&db_conn, default_tenant_id)
        .await
        .context(AppError::Settings)?;
    if let Some(token) = reg_token {
        tracing::info!("==========================================================");
        tracing::info!("  No users found. Use this one-time registration token:");
        tracing::info!("  {}", token);
        tracing::info!("==========================================================");
    }

    // --- Reconcile DB-managed settings with CLI values ---
    let force = args.force_settings_override;

    // Network settings
    let trusted_proxies = reconcile_setting_vec::<IpNet>(reconcile::ReconcileParams {
        db: &db_conn,
        tenant_id: default_tenant_id,
        key: SettingKey::TrustedProxies,
        raw: &raw_settings,
        cli_value: if args.trusted_proxies.is_empty() {
            None
        } else {
            Some(args.trusted_proxies)
        },
        default_value: vec![],
        force,
        convert: reconcile::JsonConvert {
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

    let real_ip_header = reconcile::reconcile_setting(reconcile::ReconcileParams {
        db: &db_conn,
        tenant_id: default_tenant_id,
        key: SettingKey::RealIpHeader,
        raw: &raw_settings,
        cli_value: args.real_ip_header,
        default_value: uptrakit_web_api::settings::DEFAULT_REAL_IP_HEADER.to_string(),
        force,
        convert: reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| v.as_str().map(String::from),
        },
    })
    .await
    .context(AppError::Settings)?;
    settings.set_real_ip_header(real_ip_header).await;

    let forwarded_cert_info_header = reconcile::reconcile_setting(reconcile::ReconcileParams {
        db: &db_conn,
        tenant_id: default_tenant_id,
        key: SettingKey::ForwardedClientCertInfoHeader,
        raw: &raw_settings,
        cli_value: args.forwarded_client_cert_info_header,
        default_value: String::new(), // empty = disabled
        force,
        convert: reconcile::JsonConvert {
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

    let forwarded_cert_pem_header = reconcile::reconcile_setting(reconcile::ReconcileParams {
        db: &db_conn,
        tenant_id: default_tenant_id,
        key: SettingKey::ForwardedClientCertPemHeader,
        raw: &raw_settings,
        cli_value: args.forwarded_client_cert_pem_header,
        default_value: String::new(), // empty = disabled
        force,
        convert: reconcile::JsonConvert {
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

    let pki_addr = reconcile::reconcile_setting(reconcile::ReconcileParams {
        db: &db_conn,
        tenant_id: default_tenant_id,
        key: SettingKey::PkiAddr,
        raw: &raw_settings,
        cli_value: args.pki_addr.clone(),
        default_value: String::new(), // empty = not set
        force,
        convert: reconcile::JsonConvert {
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

    let extra_sans = reconcile_setting_vec::<String>(reconcile::ReconcileParams {
        db: &db_conn,
        tenant_id: default_tenant_id,
        key: SettingKey::ExtraSans,
        raw: &raw_settings,
        cli_value: if args.sans.is_empty() {
            None
        } else {
            Some(args.sans)
        },
        default_value: vec![],
        force,
        convert: reconcile::JsonConvert {
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
        &db_conn,
        default_tenant_id,
        SettingKey::HttpsAddr,
        &raw_settings,
        args.https_addr,
        uptrakit_web_api::settings::DEFAULT_HTTPS_ADDR
            .parse()
            .expect("valid default HTTPS addr"),
        force,
    )
    .await?;
    settings.set_https_addr(https_addr).await;

    // --- Bootstrap OIDC provider from CLI flags ---
    {
        let oidc = &args.oidc_bootstrap;
        let any_set = oidc.oidc_issuer_url.is_some()
            || oidc.oidc_client_id.is_some()
            || oidc.oidc_client_secret.is_some();

        if any_set {
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

            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            use uptrakit_shared_db::entity::{oidc_provider, prelude::OidcProvider};

            let existing = OidcProvider::find()
                .filter(oidc_provider::Column::Slug.eq(slug))
                .filter(oidc_provider::Column::TenantId.eq(default_tenant_id))
                .filter(oidc_provider::Column::DeletedAt.is_null())
                .one(&db_conn)
                .await
                .context(AppError::Database)?;

            match existing {
                None => {
                    use sea_orm::ActiveModelTrait;
                    use sea_orm::Set;
                    use time::OffsetDateTime;

                    let now = OffsetDateTime::now_utc();
                    let provider = oidc_provider::ActiveModel {
                        id: Set(uuid::Uuid::now_v7()),
                        tenant_id: Set(default_tenant_id),
                        name: Set(name.to_string()),
                        slug: Set(slug.to_string()),
                        logo_url: Set(None),
                        issuer_url: Set(issuer_url.to_string()),
                        client_id: Set(client_id.to_string()),
                        client_secret: Set(uptrakit_shared_db::crypto::EncryptedString::new(
                            client_secret.to_string(),
                        )),
                        scopes: Set(scopes.to_string()),
                        auto_create_users: Set(true),
                        email_verified_trusted: Set(false),
                        role_claim_path: Set(None),
                        role_mapping: Set(
                            uptrakit_shared_db::entity::oidc_provider::RoleMapping::default(),
                        ),
                        is_active: Set(true),
                        created_at: Set(now),
                        updated_at: Set(now),
                        deleted_at: Set(None),
                    };
                    provider
                        .insert(&db_conn)
                        .await
                        .context(AppError::Database)?;
                    tracing::info!(slug = slug, name = name, "bootstrapped OIDC provider");
                }
                Some(existing_provider) if force => {
                    use sea_orm::Set;
                    use sea_orm::{ActiveModelTrait, IntoActiveModel};
                    use time::OffsetDateTime;

                    let mut model = existing_provider.into_active_model();
                    model.issuer_url = Set(issuer_url.to_string());
                    model.client_id = Set(client_id.to_string());
                    model.client_secret = Set(uptrakit_shared_db::crypto::EncryptedString::new(
                        client_secret.to_string(),
                    ));
                    model.is_active = Set(true);
                    model.updated_at = Set(OffsetDateTime::now_utc());
                    model.update(&db_conn).await.context(AppError::Database)?;
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
        }
    }

    // Resolve static directory for SPA serving
    let static_dir = resolve_static_dir(args.static_dir)?;

    // Validate TLS args
    if args.tls_cert.is_some() != args.tls_key.is_some() {
        return Err(report!(AppError::Config(
            "both --tls-cert and --tls-key must be provided together".into()
        )));
    }

    // --san only makes sense with managed (auto-generated) certificates
    if !extra_sans.is_empty() && args.tls_cert.is_some() {
        return Err(report!(AppError::Config(
            "--san cannot be used with --tls-cert/--tls-key; \
             SANs are only configurable for controller-managed certificates"
                .into()
        )));
    }

    // Validate CA args
    if args.ca_cert.is_some() != args.ca_key.is_some() {
        return Err(report!(AppError::Config(
            "both --ca-cert and --ca-key must be provided together".into()
        )));
    }

    // Validate --pki-http
    let pki_http_port: Option<u16> = if let Some(mode) = args.pki_http {
        let pki_url = pki_addr_opt.as_deref().ok_or_else(|| {
            report!(AppError::Config(
                "--pki-http requires --pki-addr to be set".into()
            ))
        })?;
        match mode {
            cli::PkiHttpMode::Listener => {
                let parsed: url::Url = pki_url.parse().expect("already validated");
                let port = parsed.port_or_known_default().ok_or_else(|| {
                    report!(AppError::Config(
                        "--pki-addr URL must have an explicit or default port".into()
                    ))
                })?;
                Some(port)
            }
            cli::PkiHttpMode::External => None,
        }
    } else {
        // Warn if pki_addr has http:// but no --pki-http is set
        if let Some(ref url) = pki_addr_opt
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

    // Initialize PKI (config directory for CA and TLS certificates)
    let pki_path = pki::pki_dir(app_dirs.config_dir()).context(AppError::Pki)?;

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
        let mut state =
            pki::load_or_init_managed_ca(&db_conn, default_tenant_id, pki_addr_opt.as_deref())
                .await
                .context(AppError::Pki)?;

        if pki::should_rotate_ca(&state.active.cert_pem) {
            tracing::info!("CA certificate is within rotation window, rotating now");
            let active_fp = pki::ca_fingerprint(&state.active.cert_pem).context(AppError::Pki)?;
            let rotation = pki::rotate_managed_ca(
                &db_conn,
                default_tenant_id,
                pki_addr_opt.as_deref(),
                &active_fp,
            )
            .await
            .context(AppError::Pki)?;
            state = rotation.state;
        }

        state
    };

    // Validate CA extensions match pki_addr (managed CAs only)
    if ca_state.managed {
        pki::validate_ca_pki_addr(&ca_state.active.cert_pem, pki_addr_opt.as_deref())
            .context(AppError::Pki)?;
    }

    let (ca_snapshot, ca_initial_key_store) = ca_state
        .to_snapshot(pki_addr_opt.clone())
        .context(AppError::Pki)?;
    let (ca_tx, ca_rx) = tokio::sync::watch::channel(ca_snapshot.clone());
    let ca_key_store: uptrakit_web_api::CaKeyStoreRef =
        Arc::new(tokio::sync::RwLock::new(ca_initial_key_store));

    // Resolve server certificate (using reconciled extra_sans)
    let server_cert = if let (Some(cert_path), Some(key_path)) = (&args.tls_cert, &args.tls_key) {
        pki::load_external_cert(cert_path, key_path).context(AppError::Pki)?
    } else {
        let mut cert = pki::load_or_generate_server_cert(&pki_path, &ca_state.active, &extra_sans)
            .context(AppError::Pki)?;

        // Check if the existing cert needs SAN regeneration
        if pki::server_cert_needs_san_update(&cert.cert_pem, &extra_sans).context(AppError::Pki)? {
            if pki::cert_signed_by_ca(&cert.cert_pem, &ca_state.active.cert_pem)
                .context(AppError::Pki)?
            {
                tracing::info!(
                    "server certificate SANs do not match configured values, regenerating"
                );
                cert = pki::renew_server_cert(&pki_path, &ca_state.active, &extra_sans)
                    .context(AppError::Pki)?;
            } else {
                return Err(report!(AppError::Config(
                    "The server certificate does not include the requested SANs and was signed by \
                     a different CA than the currently active one.\n\n\
                     To fix this:\n  \
                     1. Restart the controller without the --san flag(s) that are not yet in the certificate\n  \
                     2. Regenerate the server certificate via POST /api/v1/settings/renew-server-certificate or the UI\n  \
                     3. Restart the controller with the desired --san flag(s)"
                        .into()
                )));
            }
        }

        // Auto-renew if within renewal window
        if pki::should_renew_server_cert(&cert.cert_pem) {
            tracing::info!("server certificate is within renewal window, renewing now");
            cert = pki::renew_server_cert(&pki_path, &ca_state.active, &extra_sans)
                .context(AppError::Pki)?;
        }

        cert
    };

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Create revocation notify channel
    let revocation_notify = Arc::new(tokio::sync::Notify::const_new());

    // Create CA rotation trigger (used by the rotate-ca API endpoint)
    let ca_rotation_trigger = Arc::new(tokio::sync::Notify::const_new());

    // Read initial revocation version for CRL version-gated polling
    let initial_revocation_version =
        uptrakit_web_api::settings_store::get_revocation_version(&db_conn, default_tenant_id)
            .await
            .context(AppError::Settings)?;

    // Build initial CRLs from DB before server starts
    let crl_pem_cache = Arc::new(tokio::sync::RwLock::new(String::new()));
    let (initial_crls, initial_crl_pem) = {
        let ks = ca_key_store.read().await;
        crl_manager::build_initial_crls(&db_conn, &ca_snapshot, &ks)
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
        crl_manager::CrlManager::new(
            crl_manager::CrlManagerConfig {
                server_cert_pem: server_cert.cert_pem.clone(),
                server_key_pem: server_cert.key_pem.clone(),
                db: db_conn.clone(),
                rustls_config: rustls_config.clone(),
                revocation_notify: Arc::clone(&revocation_notify),
                crl_pem_cache: Arc::clone(&crl_pem_cache),
                default_tenant_id,
                initial_revocation_version,
            },
            &ca_snapshot,
            &ks,
        )
        .context(AppError::Pki)?
    });

    // Create CancellationToken for graceful shutdown of background tasks
    let shutdown_token = CancellationToken::new();

    // Create axum_server Handle for graceful shutdown
    let server_handle = axum_server::Handle::new();

    // Spawn CRL manager background task
    let crl_shutdown_token = shutdown_token.child_token();
    let crl_handle = tokio::spawn(Arc::clone(&crl_manager).run(Some(crl_shutdown_token)));

    // Create agent certificate signer (reads from watch receiver and key store)
    let cert_signer = Arc::new(cert_signer::RcgenAgentCertSigner::new(
        ca_rx.clone(),
        Arc::clone(&ca_key_store),
    ));

    // Migrate file-based JWT key to DB if it exists (backwards compatibility)
    uptrakit_web_api::settings_store::migrate_file_jwt_key(
        &db_conn,
        default_tenant_id,
        app_dirs.state_dir(),
    )
    .await
    .context(AppError::Config("JWT key migration failed".into()))?;
    // Load or generate JWT signing key from DB (HA-safe: all instances share the same key)
    let jwt_secret =
        uptrakit_web_api::settings_store::load_or_generate_jwt_key(&db_conn, default_tenant_id)
            .await
            .context(AppError::Config("JWT key initialization failed".into()))?;
    let jwt_manager = uptrakit_web_api::auth::jwt::JwtManager::from_secret(&jwt_secret);
    tracing::info!("JWT signing key initialized from database");

    let oidc_flow_store = uptrakit_web_api::auth::oidc_state::OidcFlowStore::new(db_conn.clone());
    let account_link_store =
        uptrakit_web_api::auth::oidc_state::AccountLinkStore::new(db_conn.clone());
    let oidc_token_exchange_store =
        uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore::new(db_conn.clone());
    let oidc_registration_store =
        uptrakit_web_api::auth::oidc_state::OidcRegistrationStore::new(db_conn.clone());
    let device_flow_store =
        uptrakit_web_api::auth::device_flow::DeviceFlowStore::new(db_conn.clone());
    let rate_limit_store = uptrakit_web_api::auth::rate_limit::RateLimitStore::new(db_conn.clone());

    let service_connections =
        uptrakit_web_api::service_connections::ServiceConnectionRegistry::new();

    let controller_id = uuid::Uuid::now_v7();
    let notification_service = uptrakit_web_api::notification_service::NotificationService::new(
        db_conn.clone(),
        service_connections.clone(),
        controller_id,
    );

    let token_denylist = Arc::new(uptrakit_web_api::auth::token_denylist::TokenDenylist::new());

    let app_state = Arc::new(AppState {
        ca_snapshot: ca_rx,
        ca_key_store: Arc::clone(&ca_key_store),
        db: db_conn.clone(),
        settings,
        cert_signer,
        service_connections: service_connections.clone(),
        revocation_notify,
        oidc_flow_store: oidc_flow_store.clone(),
        account_link_store: account_link_store.clone(),
        jwt: Arc::new(jwt_manager),
        oidc_token_exchange_store: oidc_token_exchange_store.clone(),
        oidc_registration_store: oidc_registration_store.clone(),
        device_flow_store: device_flow_store.clone(),
        rate_limit_store: rate_limit_store.clone(),
        pki_path: pki_path.clone(),
        rustls_config: rustls_config.clone(),
        crl_pem_cache,
        ca_rotation_trigger: Arc::clone(&ca_rotation_trigger),
        default_tenant_id,
        controller_id,
        notification_service: notification_service.clone(),
        token_denylist: Arc::clone(&token_denylist),
    });

    // Spawn periodic cleanup for auth state stores (every 5 minutes)
    let oidc_cleanup_token = shutdown_token.child_token();
    let oidc_cleanup_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    oidc_flow_store.cleanup_expired().await;
                    account_link_store.cleanup_expired().await;
                    oidc_token_exchange_store.cleanup_expired().await;
                    oidc_registration_store.cleanup_expired().await;
                    device_flow_store.cleanup_expired().await;
                    rate_limit_store.cleanup_expired().await;
                    token_denylist.purge_expired().await;
                }
                _ = oidc_cleanup_token.cancelled() => {
                    tracing::debug!("auth state cleanup task shutting down");
                    break;
                }
            }
        }
    });

    // Spawn periodic settings version check (every 30s, for cross-instance cache invalidation)
    let settings_reload_token = shutdown_token.child_token();
    let settings_reload_handle = {
        let settings = app_state.settings.clone();
        let db = app_state.db.clone();
        let tid = default_tenant_id;
        let token = settings_reload_token;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            // Skip the first immediate tick — settings were just loaded
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        match settings.check_version_and_reload(&db, tid).await {
                            Ok(true) => tracing::info!("settings reloaded from database (version changed)"),
                            Ok(false) => tracing::debug!("settings version unchanged"),
                            Err(e) => tracing::warn!(error = ?e, "periodic settings version check failed"),
                        }
                    }
                    _ = token.cancelled() => {
                        tracing::debug!("settings reload task shutting down");
                        break;
                    }
                }
            }
        })
    };

    let initial_ca_version = if ca_state.managed {
        pki::load_ca_version(&app_state.db, default_tenant_id)
            .await
            .context(AppError::Pki)?
    } else {
        0
    };

    let ca_reload_token = shutdown_token.child_token();
    let ca_reload_handle = if ca_state.managed {
        let db = app_state.db.clone();
        let ca_tx_for_task = ca_tx.clone();
        let crl_mgr_for_task = Arc::clone(&crl_manager);
        let ca_key_store_for_reload = Arc::clone(&ca_key_store);
        let tenant_id = default_tenant_id;
        let settings_for_task = app_state.settings.clone();
        let token = ca_reload_token;

        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            let mut cached_version = initial_ca_version;
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = token.cancelled() => {
                        tracing::debug!("CA reload task shutting down");
                        break;
                    }
                }

                let Ok(db_version) = pki::load_ca_version(&db, tenant_id).await else {
                    continue;
                };

                if db_version == cached_version {
                    continue;
                }

                let state = match pki::load_managed_ca_state(&db, tenant_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = ?e, "failed to reload CA state from database");
                        continue;
                    }
                };

                let pki_addr = settings_for_task.pki_addr();
                let (snapshot, new_key_store) = match state.to_snapshot(pki_addr) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = ?e, "failed to build CA snapshot after reload");
                        continue;
                    }
                };

                if let Err(e) = crl_mgr_for_task.update_ca(&snapshot, &new_key_store).await {
                    tracing::error!(error = ?e, "failed to update CRL manager after CA reload");
                    continue;
                }

                // Update the shared key store
                *ca_key_store_for_reload.write().await = new_key_store;

                let _ = ca_tx_for_task.send(snapshot);

                if let Err(e) = crl_mgr_for_task.reload_tls_config().await {
                    tracing::error!(error = ?e, "failed to reload TLS after CA reload");
                }

                cached_version = db_version;
            }
        }))
    } else {
        None
    };
    // Spawn event poller for cross-controller notification delivery
    let event_poller_token = shutdown_token.child_token();
    let event_poller = uptrakit_web_api::event_poller::EventPoller::new(
        app_state.db.clone(),
        service_connections.clone(),
        controller_id,
    );
    let event_poller_handle = tokio::spawn(event_poller.run(event_poller_token));

    // Spawn CA rotation background task (managed CAs only, every 24h or on API trigger)
    let ca_rotation_token = shutdown_token.child_token();
    let ca_rotation_handle = if ca_state.managed {
        let ca_tx_for_task = ca_tx.clone();
        let crl_mgr_for_task = Arc::clone(&crl_manager);
        let ca_key_store_for_rotation = Arc::clone(&ca_key_store);
        let notifications_for_task = notification_service.clone();
        let settings_for_rotation = app_state.settings.clone();
        let db_for_rotation = app_state.db.clone();
        let tenant_id = default_tenant_id;
        let trigger = Arc::clone(&ca_rotation_trigger);
        let token = ca_rotation_token;

        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(24 * 3600));
            // Skip the first immediate tick
            interval.tick().await;

            loop {
                // Wait for either the periodic timer, an API-triggered rotation, or shutdown
                let forced = tokio::select! {
                    _ = interval.tick() => false,
                    () = trigger.notified() => true,
                    _ = token.cancelled() => {
                        tracing::debug!("CA rotation task shutting down");
                        return;
                    }
                };

                if !forced {
                    tracing::debug!("checking CA rotation status");
                    let snapshot = ca_tx_for_task.borrow().clone();
                    if !pki::should_rotate_ca(&snapshot.active_cert_pem) {
                        continue;
                    }
                    tracing::info!("CA certificate is within rotation window, rotating");
                } else {
                    tracing::info!("CA rotation triggered via API");
                }

                let current_pki_addr = settings_for_rotation.pki_addr();
                let snapshot = ca_tx_for_task.borrow().clone();
                let expected_fp = snapshot.active_fingerprint.clone();

                match pki::rotate_managed_ca(
                    &db_for_rotation,
                    tenant_id,
                    current_pki_addr.as_deref(),
                    &expected_fp,
                )
                .await
                {
                    Ok(rotation) => {
                        if !rotation.rotated {
                            tracing::info!(
                                "CA rotation skipped (another controller already rotated)"
                            );
                            continue;
                        }

                        let rotation_pki_addr = current_pki_addr.clone();
                        match rotation.state.to_snapshot(rotation_pki_addr) {
                            Ok((new_snapshot, new_key_store)) => {
                                // Update CRL manager with new CA material
                                if let Err(e) = crl_mgr_for_task
                                    .update_ca(&new_snapshot, &new_key_store)
                                    .await
                                {
                                    tracing::error!(error = ?e, "failed to update CRL manager after CA rotation");
                                    continue;
                                }

                                // Update the shared key store
                                *ca_key_store_for_rotation.write().await = new_key_store;

                                // Broadcast CA bundle update to all connected services
                                let ca_payload = uptrakit_internal_wire::CaBundleUpdatedPayload {
                                    ca_bundle_pem: new_snapshot.bundle_pem.clone(),
                                };
                                notifications_for_task
                                    .broadcast(
                                        uptrakit_internal_wire::ControllerMessage::CaBundleUpdated(
                                            ca_payload,
                                        ),
                                    )
                                    .await;

                                // Request all services to renew their certificates
                                let renewal_payload =
                                    uptrakit_internal_wire::RequestCertRenewalPayload {
                                        reason: "CA rotation".to_string(),
                                    };
                                notifications_for_task
                                    .broadcast(uptrakit_internal_wire::ControllerMessage::RequestCertRenewal(renewal_payload))
                                    .await;

                                // Publish new snapshot via watch channel
                                let _ = ca_tx_for_task.send(new_snapshot);

                                // Trigger CRL rebuild
                                if let Err(e) = crl_mgr_for_task.reload_tls_config().await {
                                    tracing::error!(error = ?e, "failed to reload TLS after CA rotation");
                                }

                                tracing::info!("CA rotation completed successfully");
                            }
                            Err(e) => {
                                tracing::error!(error = ?e, "failed to build snapshot after CA rotation");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "CA rotation failed");
                    }
                }
            }
        }))
    } else {
        None
    };

    // Spawn server cert renewal background task (every 24h)
    let server_cert_renewal_token = shutdown_token.child_token();
    let server_cert_renewal_handle = if args.tls_cert.is_none() {
        // Only auto-renew when using internally-generated server certs
        let pki_for_task = pki_path;
        let crl_mgr_for_task = Arc::clone(&crl_manager);
        let ca_key_store_for_renewal = Arc::clone(&ca_key_store);
        let app_state_for_task = Arc::clone(&app_state);
        let token = server_cert_renewal_token;

        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(24 * 3600));
            // Skip the first immediate tick
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = token.cancelled() => {
                        tracing::debug!("server cert renewal task shutting down");
                        return;
                    }
                }
                tracing::debug!("checking server certificate renewal status");

                // Read current server cert from disk
                let cert_path = pki_for_task.join("server.crt");
                let Ok(cert_pem) = std::fs::read_to_string(&cert_path) else {
                    continue;
                };

                if !pki::should_renew_server_cert(&cert_pem) {
                    continue;
                }

                tracing::info!("server certificate is within renewal window, renewing");

                // Get current active CA from watch channel and key store
                let snapshot = app_state_for_task.ca_snapshot.borrow().clone();
                let key_store = ca_key_store_for_renewal.read().await;

                // Build a temporary CaBundle for renewal
                let ca_key = match rcgen::KeyPair::from_pem(&key_store.active_key_pem) {
                    Ok(k) => k,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to parse CA key for server cert renewal");
                        continue;
                    }
                };
                let ca_issuer = match rcgen::Issuer::from_ca_cert_pem(
                    &snapshot.active_cert_pem,
                    ca_key,
                ) {
                    Ok(i) => i,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to create CA issuer for server cert renewal");
                        continue;
                    }
                };

                let ca_bundle = pki::CaBundle {
                    cert_pem: snapshot.active_cert_pem.clone(),
                    key_pem: key_store.active_key_pem.to_string(),
                    issuer: ca_issuer,
                };
                // Drop the key store lock before proceeding with I/O
                drop(key_store);

                let extra_sans = app_state_for_task.settings.extra_sans();
                match pki::renew_server_cert(&pki_for_task, &ca_bundle, &extra_sans) {
                    Ok(new_cert) => {
                        // Update CRL manager's server cert
                        crl_mgr_for_task
                            .update_server_cert(new_cert.cert_pem.clone(), new_cert.key_pem.clone())
                            .await;

                        // Reload TLS config
                        if let Err(e) = crl_mgr_for_task.reload_tls_config().await {
                            tracing::error!(error = ?e, "failed to reload TLS after server cert renewal");
                        }

                        tracing::info!("server certificate auto-renewed successfully");
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "server certificate renewal failed");
                    }
                }
            }
        }))
    } else {
        None
    };

    // Set up signal handlers
    let mut sigterm = signal(SignalKind::terminate()).context_transform(|e| {
        AppError::Config(format!("failed to set up SIGTERM handler: {e}"))
    })?;
    let mut sigint = signal(SignalKind::interrupt())
        .context_transform(|e| AppError::Config(format!("failed to set up SIGINT handler: {e}")))?;
    let mut sigusr1 = signal(SignalKind::user_defined1()).context_transform(|e| {
        AppError::Config(format!("failed to set up SIGUSR1 handler: {e}"))
    })?;

    // Spawn the HTTPS server
    let server_options = server::ServerOptions {
        https_addr,
        rustls_config,
        app_state: Arc::clone(&app_state),
        static_dir,
        handle: server_handle.clone(),
        enable_reuseport: args.reuseport,
    };
    let server_task = tokio::spawn(server::run(server_options));

    // If taking over, signal old process after we're ready
    if let Some(old_pid) = args.takeover_from {
        // Wait briefly for server to start accepting connections
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Signal old process to begin graceful shutdown
        match nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(old_pid as i32),
            nix::sys::signal::Signal::SIGUSR1,
        ) {
            Ok(()) => tracing::info!(pid = old_pid, "sent SIGUSR1 to old process"),
            Err(e) => tracing::warn!(pid = old_pid, error = %e, "failed to signal old process"),
        }
    }

    // Spawn PKI HTTP server if needed
    let pki_http_task = if let Some(port) = pki_http_port {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let app_state_for_pki = Arc::clone(&app_state);
        Some(tokio::spawn(server::run_pki_http(addr, app_state_for_pki)))
    } else {
        None
    };

    // Main event loop - wait for shutdown signal
    let mut server_task = server_task;
    let shutdown_reason = tokio::select! {
        result = &mut server_task => {
            match result {
                Ok(Ok(())) => {
                    tracing::info!("server task exited normally");
                    "server exit"
                }
                Ok(Err(e)) => {
                    tracing::error!(error = ?e, "server error");
                    return Err(e).context(AppError::Server)?;
                }
                Err(e) => {
                    tracing::error!(error = %e, "server task panicked");
                    "server panic"
                }
            }
        }
        _ = sigterm.recv() => {
            tracing::info!("received SIGTERM, initiating graceful shutdown");
            "SIGTERM"
        }
        _ = sigint.recv() => {
            tracing::info!("received SIGINT, initiating graceful shutdown");
            "SIGINT"
        }
        _ = sigusr1.recv() => {
            tracing::info!("received SIGUSR1 (new process ready), initiating graceful shutdown");
            "SIGUSR1 (takeover)"
        }
    };

    // Graceful shutdown sequence
    tracing::info!(reason = shutdown_reason, "beginning graceful shutdown");

    // 1. Stop accepting new connections immediately via the server handle
    let shutdown_timeout = Duration::from_secs(args.shutdown_timeout_secs);
    server_handle.graceful_shutdown(Some(shutdown_timeout));

    // 2. Scatter ServerRestarting notifications over 5 seconds to avoid thundering herd
    let connected_count = service_connections.connection_count().await;
    if connected_count > 0 {
        tracing::info!(
            connected_agents = connected_count,
            "sending server restarting notifications"
        );
        let scatter_duration = Duration::from_secs(5);
        service_connections
            .broadcast_server_restarting_scattered(
                uptrakit_internal_wire::ServerRestartingPayload {
                    reason: "controller restarting".to_string(),
                },
                scatter_duration,
            )
            .await;

        // Wait for notifications to be scattered before proceeding
        tokio::time::sleep(scatter_duration).await;
    }

    // 3. Cancel background tasks (they check CancellationToken)
    shutdown_token.cancel();

    // 4. Wait for background task handles to complete gracefully
    tracing::debug!("waiting for background tasks to complete");

    // Wait for CRL manager
    crl_handle.abort();

    // Wait for event poller
    let _ = tokio::time::timeout(Duration::from_secs(5), event_poller_handle).await;

    // Wait for cleanup task
    let _ = tokio::time::timeout(Duration::from_secs(5), oidc_cleanup_handle).await;

    // Wait for settings reload task
    let _ = tokio::time::timeout(Duration::from_secs(5), settings_reload_handle).await;

    // Wait for CA reload task
    if let Some(h) = ca_reload_handle {
        let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
    }

    // Wait for CA rotation task
    if let Some(h) = ca_rotation_handle {
        let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
    }

    // Wait for server cert renewal task
    if let Some(h) = server_cert_renewal_handle {
        let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
    }

    // Wait for PKI HTTP server
    if let Some(task) = pki_http_task {
        task.abort();
    }

    tracing::info!("graceful shutdown complete");
    Ok(())
}

fn read_master_key_hex(
    master_key_file: Option<&std::path::Path>,
    env_val: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    if let Some(key_file) = master_key_file {
        let contents = std::fs::read_to_string(key_file).map_err(|e| {
            format!(
                "failed to read --master-key-file {}: {e}",
                key_file.display()
            )
        })?;
        return Ok(Some(contents.trim().to_string()));
    }

    if let Some(env_val) = env_val {
        return Ok(Some(env_val.trim().to_string()));
    }

    Ok(None)
}

fn parse_master_key_hex(key_hex: &str) -> std::result::Result<[u8; 32], String> {
    let bytes = hex::decode(key_hex)
        .map_err(|e| format!("master key must be a 64-character hex string: {e}"))?;
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
        format!(
            "master key must be exactly 32 bytes (64 hex chars), got {} bytes",
            v.len()
        )
    })?;
    Ok(key_bytes)
}

// --- Reconciliation helpers ---

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

/// Reconcile a `Vec<T>` setting. Empty CLI vec is treated as "not provided".
async fn reconcile_setting_vec<T>(params: reconcile::ReconcileParams<'_, Vec<T>>) -> Result<Vec<T>>
where
    T: PartialEq + Clone + fmt::Display + 'static,
{
    let reconcile::ReconcileParams {
        db,
        tenant_id,
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
                uptrakit_web_api::settings_store::upsert_setting(
                    db,
                    tenant_id,
                    key,
                    (convert.to_json)(&cli_val),
                )
                .await
                .context(AppError::Settings)?;
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
            uptrakit_web_api::settings_store::upsert_setting(
                db,
                tenant_id,
                key,
                (convert.to_json)(&cli_val),
            )
            .await
            .context(AppError::Settings)?;
            Ok(cli_val)
        }
        (None, None) => {
            tracing::info!(key = db_key, value = %DisplayVec(&default_value), "seeding DB setting from default");
            uptrakit_web_api::settings_store::upsert_setting(
                db,
                tenant_id,
                key,
                (convert.to_json)(&default_value),
            )
            .await
            .context(AppError::Settings)?;
            Ok(default_value)
        }
    }
}

/// Reconcile a `SocketAddr` setting.
async fn reconcile_socket_addr(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    key: SettingKey,
    raw: &uptrakit_web_api::settings_store::RawSettings,
    cli_value: Option<SocketAddr>,
    default_value: SocketAddr,
    force: bool,
) -> Result<SocketAddr> {
    reconcile::reconcile_setting(reconcile::ReconcileParams {
        db,
        tenant_id,
        key,
        raw,
        cli_value,
        default_value,
        force,
        convert: reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v.to_string()),
            from_json: |v| v.as_str().and_then(|s| s.parse().ok()),
        },
    })
    .await
    .context(AppError::Settings)
}

/// Resolve the static directory for SPA serving.
///
/// If `--static-dir` is given, validates that it contains `index.html`.
/// Otherwise, auto-detects by probing `frontend/build` and `frontend`
/// relative to the current working directory.
fn resolve_static_dir(explicit: Option<PathBuf>) -> Result<Option<PathBuf>> {
    if let Some(dir) = explicit {
        let index = dir.join("index.html");
        if !index.is_file() {
            return Err(report!(AppError::Config(format!(
                "--static-dir {}: missing index.html",
                dir.display()
            ))));
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
        assert!(matches!(result, Ok(Some(ref value)) if value == "deadbeef"));
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
        assert!(matches!(result, Ok(Some(ref value)) if value == "0123"));
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
}
