mod cert_signer;
mod cli;
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

    // Build rustls config with optional client auth (mTLS)
    let rustls_config = pki::build_rustls_config_with_client_auth(
        &server_cert.cert_pem,
        &server_cert.key_pem,
        &ca.cert_pem,
    )
    .context(AppError::Pki)?;

    // Create agent certificate signer
    let cert_signer = Arc::new(cert_signer::RcgenAgentCertSigner::new(
        ca.cert_pem.clone(),
        ca.key_pem.clone(),
    ));

    let app_state = Arc::new(AppState {
        ca_pem: ca.cert_pem,
        trusted_proxies: args.trusted_proxies.into(),
        real_ip_header: args.real_ip_header,
        db: db_conn,
        settings,
        cert_signer,
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
            tls_config: rustls_config,
            app_state,
            static_dir,
        }) => {
            result.context(AppError::Server)?;
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received shutdown signal");
        }
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
