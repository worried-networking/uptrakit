mod cert_signer;
mod cli;
mod crl_manager;
mod db;
mod migration;
mod mtls_acceptor;
mod pki;
mod server;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use rootcause::{Report, prelude::*};
use thiserror::Error;
use tracing_subscriber::EnvFilter;

use uptrakit_web_api::AppState;
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

    // Initialize settings
    let (settings, reg_token) = Settings::load(&db_conn).await.context(AppError::Settings)?;
    if let Some(token) = reg_token {
        tracing::info!("==========================================================");
        tracing::info!("  No users found. Use this one-time registration token:");
        tracing::info!("  {}", token);
        tracing::info!("==========================================================");
    }

    // Resolve static directory for SPA serving
    let static_dir = resolve_static_dir(args.static_dir)?;

    // Validate TLS args
    if args.tls_cert.is_some() != args.tls_key.is_some() {
        return Err(report!(AppError::Config(
            "both --tls-cert and --tls-key must be provided together".into()
        )));
    }

    // Validate CA args
    if args.ca_cert.is_some() != args.ca_key.is_some() {
        return Err(report!(AppError::Config(
            "both --ca-cert and --ca-key must be provided together".into()
        )));
    }

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
        let mut state = pki::load_ca_state(&pki_path).context(AppError::Pki)?;

        // Auto-rotate if managed and within rotation window
        if state.managed && pki::should_rotate_ca(&state.active.cert_pem) {
            tracing::info!("CA certificate is within rotation window, rotating now");
            state = pki::rotate_ca(&pki_path).context(AppError::Pki)?;
        }

        state
    };

    let ca_snapshot = ca_state.to_snapshot().context(AppError::Pki)?;
    let (ca_tx, ca_rx) = tokio::sync::watch::channel(ca_snapshot.clone());

    // Resolve server certificate
    let server_cert = if let (Some(cert_path), Some(key_path)) = (&args.tls_cert, &args.tls_key) {
        pki::load_external_cert(cert_path, key_path).context(AppError::Pki)?
    } else {
        let mut cert = pki::load_or_generate_server_cert(&pki_path, &ca_state.active, &args.sans)
            .context(AppError::Pki)?;

        // Auto-renew if within renewal window
        if pki::should_renew_server_cert(&cert.cert_pem) {
            tracing::info!("server certificate is within renewal window, renewing now");
            cert = pki::renew_server_cert(&pki_path, &ca_state.active, &args.sans)
                .context(AppError::Pki)?;
        }

        cert
    };

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Create revocation notify channel
    let revocation_notify = Arc::new(tokio::sync::Notify::const_new());

    // Build initial CRLs from DB before server starts
    let initial_crls = crl_manager::build_initial_crls_der(&db_conn, &ca_snapshot)
        .await
        .context(AppError::Pki)?;

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

    let oidc_flow_store = uptrakit_web_api::auth::oidc_state::OidcFlowStore::new();
    let account_link_store = uptrakit_web_api::auth::oidc_state::AccountLinkStore::new();
    let oidc_token_exchange_store =
        uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore::new();
    let device_flow_store = uptrakit_web_api::auth::device_flow::DeviceFlowStore::new();

    let agent_connections = uptrakit_web_api::agent_connections::AgentConnectionRegistry::new();

    let app_state = Arc::new(AppState {
        ca_snapshot: ca_rx,
        trusted_proxies: args.trusted_proxies.into(),
        real_ip_header: args.real_ip_header,
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
        extra_sans: args.sans.into(),
    });

    // Spawn periodic cleanup for OIDC state stores (every 5 minutes)
    let oidc_cleanup_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            oidc_flow_store.cleanup_expired();
            account_link_store.cleanup_expired();
            oidc_token_exchange_store.cleanup_expired();
            device_flow_store.cleanup_expired();
        }
    });

    // Spawn CA rotation background task (managed CAs only, every 24h)
    let ca_rotation_handle = if ca_state.managed {
        let pki_for_task = pki_path.clone();
        let ca_tx_for_task = ca_tx;
        let crl_mgr_for_task = Arc::clone(&crl_manager);
        let conns_for_task = agent_connections.clone();

        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
            // Skip the first immediate tick
            interval.tick().await;

            loop {
                interval.tick().await;
                tracing::debug!("checking CA rotation status");

                let snapshot = ca_tx_for_task.borrow().clone();
                if !pki::should_rotate_ca(&snapshot.active_cert_pem) {
                    continue;
                }

                tracing::info!("CA certificate is within rotation window, rotating");
                match pki::rotate_ca(&pki_for_task) {
                    Ok(new_state) => {
                        match new_state.to_snapshot() {
                            Ok(new_snapshot) => {
                                // Update CRL manager with new CA material
                                if let Err(e) = crl_mgr_for_task.update_ca(&new_snapshot).await {
                                    tracing::error!(error = ?e, "failed to update CRL manager after CA rotation");
                                    continue;
                                }

                                // Broadcast CA bundle update to all connected agents
                                let payload = uptrakit_internal_wire::CaBundleUpdatedPayload {
                                    ca_bundle_pem: new_snapshot.bundle_pem.clone(),
                                };
                                conns_for_task.broadcast_ca_bundle_updated(payload).await;

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

                let extra_sans: Vec<String> = app_state_for_task.extra_sans.to_vec();
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

    // Start MQTT if configured
    #[cfg(feature = "mqtt")]
    let mqtt_handle = if let Some(host) = args.mqtt.mqtt_host {
        if args.mqtt.mqtt_password.is_some() && args.mqtt.mqtt_username.is_none() {
            return Err(report!(AppError::Config(
                "--mqtt-password requires --mqtt-username".into()
            )));
        }
        let config = uptrakit_mqtt::MqttConfig {
            host,
            port: args.mqtt.mqtt_port,
            client_id: args.mqtt.mqtt_client_id,
            username: args.mqtt.mqtt_username,
            password: args.mqtt.mqtt_password,
            topic_prefix: args.mqtt.mqtt_topic_prefix,
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
    };

    tokio::select! {
        result = server::run(server::ServerOptions {
            http_addr: args.http_addr,
            https_addr: args.https_addr,
            rustls_config,
            app_state,
            static_dir,
        }) => {
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
