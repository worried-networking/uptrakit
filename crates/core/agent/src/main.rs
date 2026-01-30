mod cli;
mod client;
mod enrollment;
mod error;
mod state;

use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;

use cli::Args;
use error::Error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("uptrakit_agent=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    if let Err(e) = run(&args).await {
        tracing::error!(error = %e, "agent failed");
        std::process::exit(1);
    }
}

async fn run(args: &Args) -> error::Result<()> {
    // Resolve data directory, create if needed
    let data_dir = args
        .resolve_data_dir()
        .map_err(|s| report!(Error::Enrollment(s)))?;
    std::fs::create_dir_all(&data_dir).context_to::<Error>()?;
    tracing::info!("data directory: {}", data_dir.display());

    // TOFU CA pinning: load from disk, or fetch from HTTP and persist
    let ca_pem = if let Some(cached) = state::load_ca_cert(&data_dir)? {
        tracing::info!("loaded CA certificate from disk");
        cached
    } else {
        tracing::info!("fetching CA certificate from controller");
        let pem = client::fetch_ca_certificate(&args.host, args.http_port).await?;
        state::save_ca_cert(&data_dir, &pem)?;
        tracing::info!("CA certificate saved to disk");
        pem
    };

    // Build TLS connector with the pinned CA
    let tls_connector = client::build_tls_connector(&ca_pem)?;

    // Enrollment: load agent.json or enroll
    let agent_state = if let Some(existing) = state::AgentState::load(&data_dir)? {
        tracing::info!(agent_id = %existing.agent_id, "loaded existing agent state");
        existing
    } else {
        tracing::info!("no agent state found, enrolling with controller");

        let system_hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let friendly_name = args
            .friendly_name
            .clone()
            .unwrap_or_else(|| system_hostname.clone());

        let resp = enrollment::enroll(
            &args.host,
            args.port,
            &tls_connector,
            &system_hostname,
            &friendly_name,
            args.enrollment_token.as_deref(),
        )
        .await?;

        tracing::info!(
            agent_id = %resp.agent_id,
            status = %resp.status,
            "enrollment response received"
        );

        let new_state = state::AgentState {
            agent_id: resp.agent_id,
            enrollment_secret: resp.enrollment_secret,
        };
        new_state.save(&data_dir)?;
        tracing::info!("agent state persisted");
        new_state
    };

    // Poll loop: check enrollment status until approved
    loop {
        let status_resp = enrollment::poll_status(
            &args.host,
            args.port,
            &tls_connector,
            &agent_state.enrollment_secret,
        )
        .await?;

        match status_resp.status.as_str() {
            "approved" => {
                tracing::info!(agent_id = %status_resp.agent_id, "enrollment approved");
                break;
            }
            "rejected" => {
                tracing::error!(agent_id = %status_resp.agent_id, "enrollment rejected");
                return Err(report!(Error::EnrollmentRejected));
            }
            "pending" => {
                tracing::info!(
                    agent_id = %status_resp.agent_id,
                    poll_interval_secs = args.enrollment_poll_interval,
                    "enrollment pending, waiting..."
                );
                tokio::time::sleep(Duration::from_secs(args.enrollment_poll_interval)).await;
            }
            other => {
                tracing::warn!(status = %other, "unknown enrollment status");
                tokio::time::sleep(Duration::from_secs(args.enrollment_poll_interval)).await;
            }
        }
    }

    // Request client certificate for mTLS, or load existing
    let cert_state = if let Some(existing) = state::AgentCertState::load(&data_dir)? {
        tracing::info!("loaded existing agent certificate from disk");
        existing
    } else {
        tracing::info!("requesting client certificate from controller");
        let cert_resp = enrollment::request_certificate(
            &args.host,
            args.port,
            &tls_connector,
            &agent_state.enrollment_secret,
        )
        .await?;
        tracing::info!(
            lifetime_days = cert_resp.lifetime_days,
            "received client certificate"
        );
        let cert_state = state::AgentCertState {
            cert_pem: cert_resp.cert_pem,
            key_pem: cert_resp.key_pem,
        };
        cert_state.save(&data_dir)?;
        tracing::info!("agent certificate saved to disk");
        cert_state
    };

    // Build mTLS connector with client certificate
    let mtls_connector = client::build_tls_connector_with_client_cert(
        &ca_pem,
        &cert_state.cert_pem,
        &cert_state.key_pem,
    )?;

    // Connect WebSocket with mTLS and run event loop
    client::run_event_loop(&args.host, args.port, mtls_connector).await?;

    Ok(())
}
