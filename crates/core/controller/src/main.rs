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

use clap::Parser;
use ipnet::IpNet;
use rootcause::{Report, prelude::*};
use thiserror::Error;
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

async fn run(args: cli::Args) -> Result<(), Report<AppError>> {
    // Resolve data directory
    let data_dir = args
        .resolve_data_dir()
        .map_err(|s| report!(AppError::Config(s)))?;
    std::fs::create_dir_all(&data_dir)
        .context_transform(|e| AppError::Config(format!("failed to create data directory: {e}")))?;
    tracing::info!("data directory: {}", data_dir.display());

    // Initialize database
    let db_config = db::DbConfig::from_args(args.db_url, &data_dir).context(AppError::Database)?;
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
    let trusted_proxies = reconcile_setting_vec::<IpNet>(
        &db_conn,
        default_tenant_id,
        SettingKey::TrustedProxies,
        &raw_settings,
        if args.trusted_proxies.is_empty() {
            None
        } else {
            Some(args.trusted_proxies)
        },
        vec![],
        force,
        reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v.iter().map(|n| n.to_string()).collect::<Vec<_>>()),
            from_json: |v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str()?.parse::<IpNet>().ok())
                        .collect()
                })
            },
        },
    )
    .await
    .context(AppError::Settings)?;
    settings.set_trusted_proxies(trusted_proxies.clone()).await;

    let real_ip_header = reconcile::reconcile_setting(
        &db_conn,
        default_tenant_id,
        SettingKey::RealIpHeader,
        &raw_settings,
        args.real_ip_header,
        uptrakit_web_api::settings::DEFAULT_REAL_IP_HEADER.to_string(),
        force,
        reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| v.as_str().map(String::from),
        },
    )
    .await
    .context(AppError::Settings)?;
    settings.set_real_ip_header(real_ip_header).await;

    let forwarded_cert_info_header = reconcile::reconcile_setting(
        &db_conn,
        default_tenant_id,
        SettingKey::ForwardedClientCertInfoHeader,
        &raw_settings,
        args.forwarded_client_cert_info_header,
        String::new(), // empty = disabled
        force,
        reconcile::JsonConvert {
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
    )
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

    let forwarded_cert_pem_header = reconcile::reconcile_setting(
        &db_conn,
        default_tenant_id,
        SettingKey::ForwardedClientCertPemHeader,
        &raw_settings,
        args.forwarded_client_cert_pem_header,
        String::new(), // empty = disabled
        force,
        reconcile::JsonConvert {
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
    )
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

    let pki_addr = reconcile::reconcile_setting(
        &db_conn,
        default_tenant_id,
        SettingKey::PkiAddr,
        &raw_settings,
        args.pki_addr.clone(),
        String::new(), // empty = not set
        force,
        reconcile::JsonConvert {
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
    )
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

    let extra_sans = reconcile_setting_vec::<String>(
        &db_conn,
        default_tenant_id,
        SettingKey::ExtraSans,
        &raw_settings,
        if args.sans.is_empty() {
            None
        } else {
            Some(args.sans)
        },
        vec![],
        force,
        reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
            },
        },
    )
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

    // MQTT settings
    #[cfg(feature = "mqtt")]
    {
        let mqtt_host = reconcile::reconcile_setting(
            &db_conn,
            default_tenant_id,
            SettingKey::MqttHost,
            &raw_settings,
            args.mqtt.mqtt_host.clone(),
            String::new(), // empty = disabled
            force,
            reconcile::JsonConvert {
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
        )
        .await
        .context(AppError::Settings)?;
        let mqtt_host_opt = if mqtt_host.is_empty() {
            None
        } else {
            Some(mqtt_host)
        };

        let mqtt_port = reconcile_u16(
            &db_conn,
            default_tenant_id,
            SettingKey::MqttPort,
            &raw_settings,
            args.mqtt.mqtt_port,
            uptrakit_web_api::settings::DEFAULT_MQTT_PORT,
            force,
        )
        .await?;

        let mqtt_client_id = reconcile::reconcile_setting(
            &db_conn,
            default_tenant_id,
            SettingKey::MqttClientId,
            &raw_settings,
            args.mqtt.mqtt_client_id.clone(),
            uptrakit_web_api::settings::DEFAULT_MQTT_CLIENT_ID.to_string(),
            force,
            reconcile::JsonConvert {
                to_json: |v| serde_json::json!(v),
                from_json: |v| v.as_str().map(String::from),
            },
        )
        .await
        .context(AppError::Settings)?;

        let mqtt_username = reconcile::reconcile_setting(
            &db_conn,
            default_tenant_id,
            SettingKey::MqttUsername,
            &raw_settings,
            args.mqtt.mqtt_username.clone(),
            String::new(),
            force,
            reconcile::JsonConvert {
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
        )
        .await
        .context(AppError::Settings)?;
        let mqtt_username_opt = if mqtt_username.is_empty() {
            None
        } else {
            Some(mqtt_username)
        };

        let mqtt_password = reconcile::reconcile_setting(
            &db_conn,
            default_tenant_id,
            SettingKey::MqttPassword,
            &raw_settings,
            args.mqtt.mqtt_password.clone(),
            String::new(),
            force,
            reconcile::JsonConvert {
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
        )
        .await
        .context(AppError::Settings)?;
        let mqtt_password_opt = if mqtt_password.is_empty() {
            None
        } else {
            Some(mqtt_password)
        };

        let mqtt_topic_prefix = reconcile::reconcile_setting(
            &db_conn,
            default_tenant_id,
            SettingKey::MqttTopicPrefix,
            &raw_settings,
            args.mqtt.mqtt_topic_prefix.clone(),
            uptrakit_web_api::settings::DEFAULT_MQTT_TOPIC_PREFIX.to_string(),
            force,
            reconcile::JsonConvert {
                to_json: |v| serde_json::json!(v),
                from_json: |v| v.as_str().map(String::from),
            },
        )
        .await
        .context(AppError::Settings)?;

        settings
            .set_mqtt(uptrakit_web_api::settings::MqttSettings {
                host: mqtt_host_opt.clone(),
                port: mqtt_port,
                client_id: mqtt_client_id,
                username: mqtt_username_opt.clone(),
                password: mqtt_password_opt.clone(),
                topic_prefix: mqtt_topic_prefix,
            })
            .await;
    }

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
                        client_secret: Set(client_secret.to_string()),
                        scopes: Set(scopes.to_string()),
                        auto_create_users: Set(true),
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
                    model.client_secret = Set(client_secret.to_string());
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

    // Initialize PKI
    let pki_path = pki::pki_dir(&data_dir).context(AppError::Pki)?;

    // Load CA state
    let ca_state = if let (Some(ca_cert_path), Some(ca_key_path)) = (&args.ca_cert, &args.ca_key) {
        // External CA — not managed
        let ca = pki::load_external_ca(ca_cert_path, ca_key_path).context(AppError::Pki)?;
        pki::CaState {
            active: ca,
            previous: None,
            managed: false,
        }
    } else {
        let mut state =
            pki::load_ca_state(&pki_path, pki_addr_opt.as_deref()).context(AppError::Pki)?;

        // Auto-rotate if managed and within rotation window
        if state.managed && pki::should_rotate_ca(&state.active.cert_pem) {
            tracing::info!("CA certificate is within rotation window, rotating now");
            state = pki::rotate_ca(&pki_path, pki_addr_opt.as_deref()).context(AppError::Pki)?;
        }

        state
    };

    // Validate CA extensions match pki_addr (managed CAs only)
    if ca_state.managed {
        pki::validate_ca_pki_addr(
            &ca_state.active.cert_pem,
            pki_addr_opt.as_deref(),
            &pki_path,
        )
        .context(AppError::Pki)?;
    }

    let ca_snapshot = ca_state
        .to_snapshot(pki_addr_opt.clone())
        .context(AppError::Pki)?;
    let (ca_tx, ca_rx) = tokio::sync::watch::channel(ca_snapshot.clone());

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

    // Build initial CRLs from DB before server starts
    let crl_pem_cache = Arc::new(tokio::sync::RwLock::new(String::new()));
    let (initial_crls, initial_crl_pem) = crl_manager::build_initial_crls(&db_conn, &ca_snapshot)
        .await
        .context(AppError::Pki)?;
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
    let crl_manager = Arc::new(
        crl_manager::CrlManager::new(
            crl_manager::CrlManagerConfig {
                server_cert_pem: server_cert.cert_pem.clone(),
                server_key_pem: server_cert.key_pem.clone(),
                db: db_conn.clone(),
                rustls_config: rustls_config.clone(),
                revocation_notify: Arc::clone(&revocation_notify),
                crl_pem_cache: Arc::clone(&crl_pem_cache),
            },
            &ca_snapshot,
        )
        .context(AppError::Pki)?,
    );

    // Spawn CRL manager background task
    let crl_handle = tokio::spawn(Arc::clone(&crl_manager).run());

    // Create agent certificate signer (reads from watch receiver)
    let cert_signer = Arc::new(cert_signer::RcgenAgentCertSigner::new(ca_rx.clone()));

    // Initialize JWT signing key
    let jwt_manager = uptrakit_web_api::auth::jwt::JwtManager::load_or_generate(&data_dir)
        .context(AppError::Config("JWT initialization failed".into()))?;
    tracing::info!("JWT signing key initialized");

    let oidc_flow_store = uptrakit_web_api::auth::oidc_state::OidcFlowStore::new(db_conn.clone());
    let account_link_store =
        uptrakit_web_api::auth::oidc_state::AccountLinkStore::new(db_conn.clone());
    let oidc_token_exchange_store =
        uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore::new(db_conn.clone());
    let device_flow_store =
        uptrakit_web_api::auth::device_flow::DeviceFlowStore::new(db_conn.clone());

    let agent_connections = uptrakit_web_api::agent_connections::AgentConnectionRegistry::new();

    let app_state = Arc::new(AppState {
        ca_snapshot: ca_rx,
        db: db_conn,
        settings,
        cert_signer,
        agent_connections: agent_connections.clone(),
        revocation_notify,
        oidc_flow_store: oidc_flow_store.clone(),
        account_link_store: account_link_store.clone(),
        jwt: Arc::new(jwt_manager),
        oidc_token_exchange_store: oidc_token_exchange_store.clone(),
        device_flow_store: device_flow_store.clone(),
        pki_path: pki_path.clone(),
        rustls_config: rustls_config.clone(),
        crl_pem_cache,
        ca_rotation_trigger: Arc::clone(&ca_rotation_trigger),
        default_tenant_id,
    });

    // Spawn periodic cleanup for auth state stores (every 5 minutes)
    let oidc_cleanup_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            oidc_flow_store.cleanup_expired().await;
            account_link_store.cleanup_expired().await;
            oidc_token_exchange_store.cleanup_expired().await;
            device_flow_store.cleanup_expired().await;
        }
    });

    // Spawn CA rotation background task (managed CAs only, every 24h or on API trigger)
    let ca_rotation_handle = if ca_state.managed {
        let pki_for_task = pki_path.clone();
        let ca_tx_for_task = ca_tx;
        let crl_mgr_for_task = Arc::clone(&crl_manager);
        let conns_for_task = agent_connections.clone();
        let settings_for_rotation = app_state.settings.clone();
        let trigger = Arc::clone(&ca_rotation_trigger);

        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
            // Skip the first immediate tick
            interval.tick().await;

            loop {
                // Wait for either the periodic timer or an API-triggered rotation
                let forced = tokio::select! {
                    _ = interval.tick() => false,
                    () = trigger.notified() => true,
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

                let current_pki_addr = settings_for_rotation.pki_addr().await;
                match pki::rotate_ca(&pki_for_task, current_pki_addr.as_deref()) {
                    Ok(new_state) => {
                        let rotation_pki_addr = current_pki_addr.clone();
                        match new_state.to_snapshot(rotation_pki_addr) {
                            Ok(new_snapshot) => {
                                // Update CRL manager with new CA material
                                if let Err(e) = crl_mgr_for_task.update_ca(&new_snapshot).await {
                                    tracing::error!(error = ?e, "failed to update CRL manager after CA rotation");
                                    continue;
                                }

                                // Broadcast CA bundle update to all connected agents
                                let ca_payload = uptrakit_internal_wire::CaBundleUpdatedPayload {
                                    ca_bundle_pem: new_snapshot.bundle_pem.clone(),
                                };
                                conns_for_task.broadcast_ca_bundle_updated(ca_payload).await;

                                // Request all agents to renew their certificates
                                let renewal_payload =
                                    uptrakit_internal_wire::RequestCertRenewalPayload {
                                        reason: "CA rotation".to_string(),
                                    };
                                conns_for_task
                                    .broadcast_request_cert_renewal(renewal_payload)
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
    let server_cert_renewal_handle = if args.tls_cert.is_none() {
        // Only auto-renew when using internally-generated server certs
        let pki_for_task = pki_path;
        let crl_mgr_for_task = Arc::clone(&crl_manager);
        let app_state_for_task = Arc::clone(&app_state);

        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
            // Skip the first immediate tick
            interval.tick().await;

            loop {
                interval.tick().await;
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

                // Get current active CA from watch channel
                let snapshot = app_state_for_task.ca_snapshot.borrow().clone();

                // Build a temporary CaBundle for renewal
                let ca_key = match rcgen::KeyPair::from_pem(&snapshot.active_key_pem) {
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
                    key_pem: snapshot.active_key_pem.clone(),
                    issuer: ca_issuer,
                };

                let extra_sans = app_state_for_task.settings.extra_sans().await;
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

    // Start MQTT if configured (read from reconciled settings)
    #[cfg(feature = "mqtt")]
    let mqtt_handle = {
        let mqtt_settings = app_state.settings.mqtt().await;
        if let Some(host) = mqtt_settings.host {
            if mqtt_settings.password.is_some() && mqtt_settings.username.is_none() {
                return Err(report!(AppError::Config(
                    "MQTT password requires a username".into()
                )));
            }
            let config = uptrakit_mqtt::MqttConfig {
                host,
                port: mqtt_settings.port,
                client_id: mqtt_settings.client_id,
                username: mqtt_settings.username,
                password: mqtt_settings.password,
                topic_prefix: mqtt_settings.topic_prefix,
            };
            tracing::info!("starting MQTT client: {config:?}");
            match uptrakit_mqtt::start(config).await {
                Ok(handle) => Some(handle),
                Err(e) => {
                    tracing::warn!("MQTT startup failed: {e}");
                    None
                }
            }
        } else {
            None
        }
    };

    tokio::select! {
        result = server::run(server::ServerOptions {
            https_addr,
            rustls_config,
            app_state: Arc::clone(&app_state),
            static_dir,
        }) => {
            result.context(AppError::Server)?;
        }
        result = async {
            match pki_http_port {
                Some(port) => {
                    let addr = SocketAddr::from(([0, 0, 0, 0], port));
                    server::run_pki_http(addr, app_state).await
                }
                None => std::future::pending().await,
            }
        } => {
            result.context(AppError::Server)?;
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received shutdown signal");
        }
    }

    crl_handle.abort();
    oidc_cleanup_handle.abort();
    if let Some(h) = ca_rotation_handle {
        h.abort();
    }
    if let Some(h) = server_cert_renewal_handle {
        h.abort();
    }

    #[cfg(feature = "mqtt")]
    if let Some(handle) = mqtt_handle {
        handle.shutdown().await;
    }

    Ok(())
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
#[allow(clippy::too_many_arguments)]
async fn reconcile_setting_vec<T>(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    key: SettingKey,
    raw: &uptrakit_web_api::settings_store::RawSettings,
    cli_value: Option<Vec<T>>,
    default_value: Vec<T>,
    force: bool,
    convert: reconcile::JsonConvert<Vec<T>>,
) -> Result<Vec<T>, Report<AppError>>
where
    T: PartialEq + Clone + fmt::Display + 'static,
{
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
) -> Result<SocketAddr, Report<AppError>> {
    reconcile::reconcile_setting(
        db,
        tenant_id,
        key,
        raw,
        cli_value,
        default_value,
        force,
        reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v.to_string()),
            from_json: |v| v.as_str().and_then(|s| s.parse().ok()),
        },
    )
    .await
    .context(AppError::Settings)
}

/// Reconcile a `u16` setting.
#[cfg(feature = "mqtt")]
async fn reconcile_u16(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    key: SettingKey,
    raw: &uptrakit_web_api::settings_store::RawSettings,
    cli_value: Option<u16>,
    default_value: u16,
    force: bool,
) -> Result<u16, Report<AppError>> {
    reconcile::reconcile_setting(
        db,
        tenant_id,
        key,
        raw,
        cli_value,
        default_value,
        force,
        reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| v.as_u64().and_then(|n| u16::try_from(n).ok()),
        },
    )
    .await
    .context(AppError::Settings)
}

/// Resolve the static directory for SPA serving.
///
/// If `--static-dir` is given, validates that it contains `index.html`.
/// Otherwise, auto-detects by probing `frontend/build` and `frontend`
/// relative to the current working directory.
fn resolve_static_dir(explicit: Option<PathBuf>) -> Result<Option<PathBuf>, Report<AppError>> {
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
