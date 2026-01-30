mod cli;
mod client;
mod error;
mod state;

use std::path::Path;
use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_internal_wire::{CertificatePayload, now_millis};

use cli::Args;
use client::LoopOutcome;
use error::Error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("uptrakit_agent=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    if let Err(mut e) = run(&args).await {
        if e.current_context_mut().is_receive_closed() {
            tracing::info!("disconnected by controller");
        } else {
            tracing::error!(error = %e, "agent failed");
            std::process::exit(1);
        }
    }
}

async fn run(args: &Args) -> error::Result<()> {
    // Resolve data directory, create if needed
    let data_dir = args
        .resolve_data_dir()
        .map_err(|s| report!(Error::Enrollment(s)))?;
    std::fs::create_dir_all(&data_dir).context_to::<Error>()?;
    tracing::info!("data directory: {}", data_dir.display());

    // --force-enroll: delete all existing state (keep CA cert for TOFU)
    if args.force_enroll {
        tracing::info!("--force-enroll: clearing existing state");
        state::AgentCertState::delete(&data_dir)?;
        state::AgentState::delete(&data_dir)?;
        state::delete_cert_not_after_ts(&data_dir)?;
    }

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

    // Check for existing certificate
    if let Some(_existing_cert) = state::AgentCertState::load(&data_dir)? {
        let cert_not_after_ts = state::load_cert_not_after_ts(&data_dir)?;
        let cert_expired = cert_not_after_ts.is_some_and(|ts| now_millis() >= ts);

        if cert_expired {
            tracing::warn!("certificate expired, falling back to fresh enrollment");
            state::AgentCertState::delete(&data_dir)?;
            state::delete_cert_not_after_ts(&data_dir)?;
            state::AgentState::delete(&data_dir)?;
            // Fall through to enrollment below
        } else {
            tracing::info!("loaded existing agent certificate from disk");
            match run_authenticated_with_reconnect(args, &data_dir, &ca_pem).await {
                Ok(()) => return Ok(()),
                Err(mut e) => {
                    if e.current_context_mut().is_cert_expired() {
                        tracing::warn!("certificate expired, falling back to enrollment");
                        state::AgentCertState::delete(&data_dir)?;
                        state::delete_cert_not_after_ts(&data_dir)?;
                        state::AgentState::delete(&data_dir)?;
                        // Fall through to enrollment below
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }

    // Enrollment (existing agent.json OR fresh enrollment)
    // Retry on disconnect — e.g. a merge transfers our identity while we wait.
    let tls_connector = client::build_tls_connector(&ca_pem)?;
    let cert_payload = loop {
        match do_enrollment(args, &data_dir, &tls_connector).await {
            Ok(cert) => break cert,
            Err(mut e) => {
                if e.current_context_mut().is_receive_closed() {
                    tracing::info!("disconnected during enrollment, reconnecting");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                return Err(e);
            }
        }
    };

    // Save certificate
    let cert_state = state::AgentCertState {
        cert_pem: cert_payload.cert_pem,
        key_pem: cert_payload.key_pem,
    };
    cert_state.save(&data_dir)?;
    let not_after_ms =
        cert_payload.not_after.unix_timestamp() * 1000 + i64::from(cert_payload.not_after.millisecond());
    state::save_cert_not_after_ts(&data_dir, not_after_ms)?;
    tracing::info!("agent certificate saved to disk");

    // Enter mTLS loop with reconnect
    run_authenticated_with_reconnect(args, &data_dir, &ca_pem).await
}

/// Consolidates the existing enrollment logic (bearer reconnect + fresh enroll).
async fn do_enrollment(
    args: &Args,
    data_dir: &Path,
    tls_connector: &tokio_rustls::TlsConnector,
) -> error::Result<CertificatePayload> {
    if let Some(existing) = state::AgentState::load(data_dir)? {
        // Reconnect with Bearer header (existing agent.json)
        tracing::info!(agent_id = %existing.agent_id, "reconnecting with enrollment secret");
        let auth_header = format!("Bearer {}", existing.enrollment_secret);
        let mut ws =
            client::connect_ws(&args.host, args.port, tls_connector, Some(&auth_header)).await?;

        // Wait for approval (controller pushes immediately if already approved)
        client::wait_for_approval(&mut ws).await?;

        // Request certificate
        let cert = client::request_certificate_ws(&mut ws).await?;
        tracing::info!(
            not_after = %cert.not_after,
            "received client certificate"
        );
        Ok(cert)
    } else {
        // Fresh enrollment
        tracing::info!("no agent state found, enrolling via WebSocket");
        let mut ws = client::connect_ws(&args.host, args.port, tls_connector, None).await?;

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
        agent_state.save(data_dir)?;
        tracing::info!("agent state persisted");

        // Wait for approval (may come immediately if auto-approved)
        client::wait_for_approval(&mut ws).await?;

        // Request certificate
        let cert = client::request_certificate_ws(&mut ws).await?;
        tracing::info!(
            not_after = %cert.not_after,
            "received client certificate"
        );
        Ok(cert)
    }
}

/// Enter the mTLS authenticated loop with automatic reconnection on cert rotation.
async fn run_authenticated_with_reconnect(
    args: &Args,
    data_dir: &Path,
    ca_pem: &[u8],
) -> error::Result<()> {
    loop {
        let cert_state =
            state::AgentCertState::load(data_dir)?.ok_or_else(|| report!(Error::NoCertificates))?;
        let cert_not_after_ts = state::load_cert_not_after_ts(data_dir)?;

        let mtls_connector = client::build_tls_connector_with_client_cert(
            ca_pem,
            &cert_state.cert_pem,
            &cert_state.key_pem,
        )?;

        match client::run_authenticated_loop(
            &args.host,
            args.port,
            mtls_connector,
            cert_not_after_ts,
            data_dir,
        )
        .await?
        {
            LoopOutcome::Shutdown => return Ok(()),
            LoopOutcome::Reconnect => {
                tracing::info!("reconnecting with new certificate");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            LoopOutcome::Disconnected => {
                tracing::warn!("disconnected by controller");
                return Ok(());
            }
        }
    }
}
