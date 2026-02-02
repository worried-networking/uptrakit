mod cli;
mod db;
mod error;
mod lease_manager;
mod mqtt_client;
mod tenant_manager;

use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::error::AppError;
use crate::lease_manager::LeaseManager;
use crate::tenant_manager::TenantManager;

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

/// Generate a unique instance ID: `{hostname}-{uuid_v7_first_8_chars}`
fn generate_instance_id() -> String {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    let uuid_prefix = &uuid::Uuid::now_v7().to_string()[..8];
    format!("{host}-{uuid_prefix}")
}

async fn run(args: cli::Args) -> Result<(), rootcause::Report<AppError>> {
    let instance_id = generate_instance_id();
    tracing::info!(%instance_id, "starting uptrakit-mqtt service");

    // Connect to database
    tracing::info!(
        "connecting to database: {}",
        db::sanitize_url(&args.db_url)
    );
    let db_conn = db::connect(&args.db_url)
        .await
        .context(AppError::Database)?;
    tracing::info!("database connected");

    let poll_interval = Duration::from_secs(args.poll_interval);
    let heartbeat_interval = Duration::from_secs(args.heartbeat_interval);

    let lease_mgr = LeaseManager::new(
        db_conn,
        instance_id,
        args.max_tenants,
        args.lease_timeout,
    );

    let mut tenant_mgr = TenantManager::new();

    // Initial poll
    tenant_mgr.poll(&lease_mgr).await;

    let mut poll_ticker = tokio::time::interval(poll_interval);
    let mut heartbeat_ticker = tokio::time::interval(heartbeat_interval);

    // Skip the first immediate tick (we already polled above)
    poll_ticker.tick().await;
    heartbeat_ticker.tick().await;

    loop {
        tokio::select! {
            _ = poll_ticker.tick() => {
                tenant_mgr.poll(&lease_mgr).await;
            }
            _ = heartbeat_ticker.tick() => {
                if let Err(e) = lease_mgr.heartbeat().await {
                    tracing::error!(error = ?e, "heartbeat failed");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received shutdown signal");
                break;
            }
        }
    }

    // Graceful shutdown
    tracing::info!("shutting down MQTT clients");
    tenant_mgr.shutdown_all().await;

    tracing::info!("releasing all leases");
    if let Err(e) = lease_mgr.release_all().await {
        tracing::error!(error = ?e, "failed to release leases on shutdown");
    }

    tracing::info!("shutdown complete");
    Ok(())
}
