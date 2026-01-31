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

    // Initialize PKI
    let pki_path = pki::pki_dir(&data_dir).context(AppError::Pki)?;
    let ca = pki::load_or_generate_ca(&pki_path).context(AppError::Pki)?;

    // Resolve server certificate
    let server_cert = if let (Some(cert_path), Some(key_path)) = (&args.tls_cert, &args.tls_key) {
        pki::load_external_cert(cert_path, key_path).context(AppError::Pki)?
    } else {
        pki::load_or_generate_server_cert(&pki_path, &ca, &args.sans).context(AppError::Pki)?
    };

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Create revocation notify channel
    let revocation_notify = Arc::new(tokio::sync::Notify::const_new());

    // Build initial CRL from DB before server starts
    let initial_crl = crl_manager::build_initial_crl_der(&db_conn, &ca.cert_pem, &ca.key_pem)
        .await
        .context(AppError::Pki)?;

    // Build initial server config WITH CRL
    let initial_server_config = pki::build_rustls_config_with_client_auth_and_crl(
        &server_cert.cert_pem,
        &server_cert.key_pem,
        &ca.cert_pem,
        initial_crl,
    )
    .context(AppError::Pki)?;

    let rustls_config =
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(initial_server_config));

    // Create CRL manager with the real RustlsConfig handle for hot-reloads
    let crl_manager = Arc::new(
        crl_manager::CrlManager::new(crl_manager::CrlManagerConfig {
            ca_cert_pem: ca.cert_pem.clone(),
            ca_key_pem: ca.key_pem.clone(),
            server_cert_pem: server_cert.cert_pem.clone(),
            server_key_pem: server_cert.key_pem.clone(),
            db: db_conn.clone(),
            rustls_config: rustls_config.clone(),
            revocation_notify: Arc::clone(&revocation_notify),
        })
        .context(AppError::Pki)?,
    );

    // Spawn CRL manager background task
    let crl_handle = tokio::spawn(Arc::clone(&crl_manager).run());

    // Create agent certificate signer
    let cert_signer = Arc::new(cert_signer::RcgenAgentCertSigner::new(
        ca.cert_pem.clone(),
        ca.key_pem.clone(),
    ));

    // Initialize JWT signing key
    let jwt_manager = uptrakit_web_api::auth::jwt::JwtManager::load_or_generate(&data_dir)
        .map_err(|e| report!(AppError::Config(format!("JWT initialization failed: {e}"))))?;
    tracing::info!("JWT signing key initialized");

    let oidc_flow_store = uptrakit_web_api::auth::oidc_state::OidcFlowStore::new();
    let account_link_store = uptrakit_web_api::auth::oidc_state::AccountLinkStore::new();
    let oidc_token_exchange_store =
        uptrakit_web_api::auth::oidc_state::OidcTokenExchangeStore::new();

    let app_state = Arc::new(AppState {
        ca_pem: ca.cert_pem,
        trusted_proxies: args.trusted_proxies.into(),
        real_ip_header: args.real_ip_header,
        db: db_conn,
        settings,
        cert_signer,
        agent_connections: uptrakit_web_api::agent_connections::AgentConnectionRegistry::new(),
        revocation_notify,
        oidc_flow_store: oidc_flow_store.clone(),
        account_link_store: account_link_store.clone(),
        jwt: Arc::new(jwt_manager),
        oidc_token_exchange_store: oidc_token_exchange_store.clone(),
    });

    // Spawn periodic cleanup for OIDC state stores (every 5 minutes)
    let oidc_cleanup_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            oidc_flow_store.cleanup_expired();
            account_link_store.cleanup_expired();
            oidc_token_exchange_store.cleanup_expired();
        }
    });

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
