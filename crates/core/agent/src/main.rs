mod cli;
mod client;
mod error;
mod state;

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

    // If we already have a certificate, skip straight to mTLS event loop
    if let Some(existing_cert) = state::AgentCertState::load(&data_dir)? {
        tracing::info!("loaded existing agent certificate from disk");
        let mtls_connector = client::build_tls_connector_with_client_cert(
            &ca_pem,
            &existing_cert.cert_pem,
            &existing_cert.key_pem,
        )?;
        client::run_authenticated_loop(&args.host, args.port, mtls_connector).await?;
        return Ok(());
    }

    // Enrollment via WebSocket
    let cert_payload = if let Some(existing) = state::AgentState::load(&data_dir)? {
        // Reconnect with Bearer header
        tracing::info!(agent_id = %existing.agent_id, "reconnecting with enrollment secret");
        let auth_header = format!("Bearer {}", existing.enrollment_secret);
        let mut ws =
            client::connect_ws(&args.host, args.port, &tls_connector, Some(&auth_header)).await?;

        // Wait for approval (controller pushes immediately if already approved)
        client::wait_for_approval(&mut ws).await?;

        // Request certificate
        let cert = client::request_certificate_ws(&mut ws).await?;
        tracing::info!(
            lifetime_days = cert.lifetime_days,
            "received client certificate"
        );
        cert
    } else {
        // First-time enrollment
        tracing::info!("no agent state found, enrolling via WebSocket");
        let mut ws = client::connect_ws(&args.host, args.port, &tls_connector, None).await?;

        let system_hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let friendly_name = args
            .friendly_name
            .clone()
            .unwrap_or_else(|| system_hostname.clone());

        let enrolled = client::send_enroll(
            &mut ws,
            &system_hostname,
            &friendly_name,
            args.enrollment_token.as_deref(),
        )
        .await?;

        tracing::info!(
            agent_id = %enrolled.agent_id,
            status = %enrolled.status,
            "enrollment response received"
        );

        // Persist agent state
        let agent_state = state::AgentState {
            agent_id: enrolled.agent_id,
            enrollment_secret: enrolled.enrollment_secret,
        };
        agent_state.save(&data_dir)?;
        tracing::info!("agent state persisted");

        // Wait for approval (may come immediately if auto-approved)
        client::wait_for_approval(&mut ws).await?;

        // Request certificate
        let cert = client::request_certificate_ws(&mut ws).await?;
        tracing::info!(
            lifetime_days = cert.lifetime_days,
            "received client certificate"
        );
        cert
    };

    // Save certificate
    let cert_state = state::AgentCertState {
        cert_pem: cert_payload.cert_pem,
        key_pem: cert_payload.key_pem,
    };
    cert_state.save(&data_dir)?;
    tracing::info!("agent certificate saved to disk");

    // Build mTLS connector and enter authenticated event loop
    let mtls_connector = client::build_tls_connector_with_client_cert(
        &ca_pem,
        &cert_state.cert_pem,
        &cert_state.key_pem,
    )?;
    client::run_authenticated_loop(&args.host, args.port, mtls_connector).await?;

    Ok(())
}
