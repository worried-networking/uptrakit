mod cli;
mod client;
mod error;
mod host_info;
mod state;
mod update;
mod version_check;

use std::path::Path;
use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_directories::AppDirs;
use uptrakit_internal_wire::now_millis;

use cli::Args;
use client::LoopOutcome;
use error::Error;

#[tokio::main]
async fn main() {
    let filter = match "uptrakit_agent=info".parse() {
        Ok(directive) => EnvFilter::from_default_env().add_directive(directive),
        Err(_) => EnvFilter::from_default_env(),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

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
    // Parse URL early
    let (host, port) = args
        .parsed_url()
        .map_err(|s| report!(Error::Enrollment(s)))?;
    let base_url = args.url.trim_end_matches('/');
    let pki_addr = args.pki_addr.as_deref();

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Resolve application directories, create if needed
    let app_dirs = args
        .resolve_dirs()
        .map_err(|s| report!(Error::Enrollment(s)))?;
    app_dirs
        .ensure_dirs()
        .map_err(|e| report!(Error::Enrollment(format!("failed to create directories: {e}"))))?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());

    // Shortcuts for config (CA cert) and state (agent data) directories
    let config_dir = app_dirs.config_dir();
    let state_dir = app_dirs.state_dir();

    // --force-enroll: delete all existing state (keep CA cert for TOFU)
    if args.force_enroll {
        tracing::info!("--force-enroll: clearing existing state");
        state::AgentCertState::delete(state_dir)?;
        state::AgentState::delete(state_dir)?;
        state::delete_agent_key(state_dir)?;
    }

    // CA bootstrap: cached → --ca-cert file → --pki-addr → --tofu TOFU → system trust
    // CA cert is stored in config directory
    let ca_pem: Option<Vec<u8>> = if let Some(cached) = state::load_ca_cert(config_dir)? {
        tracing::info!("loaded CA certificate from disk");
        if args.tofu {
            tracing::warn!("--tofu ignored: CA already cached");
        }
        Some(cached)
    } else if let Some(ref ca_path) = args.ca_cert {
        tracing::info!("loading CA certificate from {}", ca_path.display());
        let pem = std::fs::read(ca_path)
            .map_err(|e| report!(Error::CaCertFile(format!("{}: {e}", ca_path.display()))))?;
        state::save_ca_cert(config_dir, &pem)?;
        tracing::info!("CA certificate saved to disk");
        Some(pem)
    } else if let Some(pki) = pki_addr {
        tracing::info!("fetching CA certificate from --pki-addr {pki}");
        let pem = client::fetch_ca_certificate(pki, client::TlsMode::SystemTrust).await?;
        state::save_ca_cert(config_dir, &pem)?;
        tracing::info!("CA certificate saved to disk");
        Some(pem)
    } else if args.tofu {
        tracing::info!("TOFU: fetching CA (accepting any server certificate)");
        let pem = client::fetch_ca_certificate(base_url, client::TlsMode::TrustOnFirstUse).await?;
        state::save_ca_cert(config_dir, &pem)?;
        tracing::info!("CA certificate saved to disk");
        Some(pem)
    } else {
        tracing::info!("using system root certificates");
        None
    };

    // Check for existing certificate (stored in state directory)
    if let Some(existing_cert) = state::AgentCertState::load(state_dir)? {
        let cert_not_after_ts = existing_cert.cert_not_after_ms();
        let cert_expired = cert_not_after_ts.is_some_and(|ts| now_millis() >= ts);

        if cert_expired {
            tracing::warn!("certificate expired, falling back to fresh enrollment");
            state::AgentCertState::delete(state_dir)?;
            state::AgentState::delete(state_dir)?;
            // Fall through to enrollment below
        } else {
            tracing::info!("loaded existing agent certificate from disk");
            match run_authenticated_with_reconnect(
                &host,
                port,
                base_url,
                pki_addr,
                &app_dirs,
                ca_pem.as_deref(),
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(mut e) => {
                    if e.current_context_mut().is_cert_expired() {
                        tracing::warn!("certificate expired, falling back to enrollment");
                        state::AgentCertState::delete(state_dir)?;
                        state::AgentState::delete(state_dir)?;
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
    let tls_connector = match ca_pem.as_deref() {
        Some(pem) => client::build_tls_connector(pem)?,
        None => client::build_system_trust_tls_connector()?,
    };
    loop {
        match do_enrollment(args, &host, port, state_dir, &tls_connector).await {
            Ok(()) => break,
            Err(mut e) => {
                if e.current_context_mut().is_receive_closed() {
                    tracing::info!("disconnected during enrollment, reconnecting");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                return Err(e);
            }
        }
    }

    // Enter mTLS loop with reconnect
    run_authenticated_with_reconnect(
        &host,
        port,
        base_url,
        pki_addr,
        &app_dirs,
        ca_pem.as_deref(),
    )
    .await
}

/// Consolidates the existing enrollment logic (bearer reconnect + fresh enroll).
/// On success, cert and key are saved to disk before returning.
async fn do_enrollment(
    args: &Args,
    host: &str,
    port: u16,
    data_dir: &Path,
    tls_connector: &tokio_rustls::TlsConnector,
) -> error::Result<()> {
    if let Some(existing) = state::AgentState::load(data_dir)? {
        // Reconnect with Bearer header (existing agent.json)
        tracing::info!(agent_id = %existing.agent_id, "reconnecting with enrollment secret");
        let auth_header = format!("Bearer {}", existing.enrollment_secret);
        let mut ws = client::connect_ws(host, port, tls_connector, Some(&auth_header)).await?;

        // Wait for approval (controller pushes immediately if already approved)
        client::wait_for_approval(&mut ws).await?;

        // Generate new keypair + CSR for certificate request
        let (key_pem, csr_pem) = client::generate_keypair_and_csr(&existing.agent_id)?;
        state::save_agent_key(data_dir, &key_pem)?;

        // Request certificate
        let cert = client::request_certificate_ws(&mut ws, &csr_pem).await?;
        tracing::info!(
            not_after = %cert.not_after,
            "received client certificate"
        );
        save_cert_from_payload(data_dir, &cert.cert_pem)?;
        Ok(())
    } else {
        // Fresh enrollment — controller generates agent_id
        let system_hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let friendly_name = args
            .friendly_name
            .clone()
            .unwrap_or_else(|| system_hostname.clone());
        let host_info = host_info::collect_host_info();
        tracing::info!(machine_id = %host_info.machine_id, "collected host info");

        tracing::info!("enrolling via WebSocket");
        let mut ws = client::connect_ws(host, port, tls_connector, None).await?;

        let enrolled = client::send_enroll(
            &mut ws,
            &system_hostname,
            &friendly_name,
            args.enrollment_token.as_deref(),
            host_info,
        )
        .await?;

        tracing::info!(
            service_id = %enrolled.service_id,
            status = %enrolled.status,
            "enrollment response received"
        );

        // Persist agent state using service_id from controller response
        let agent_state = state::AgentState {
            agent_id: enrolled.service_id.clone(),
            enrollment_secret: enrolled.enrollment_secret,
        };
        agent_state.save(data_dir)?;
        tracing::info!("agent state persisted");

        // Wait for approval (may come immediately if auto-approved)
        client::wait_for_approval(&mut ws).await?;

        // Generate new keypair + CSR for certificate request
        let (cert_key_pem, cert_csr_pem) = client::generate_keypair_and_csr(&agent_state.agent_id)?;
        state::save_agent_key(data_dir, &cert_key_pem)?;

        // Request certificate
        let cert = client::request_certificate_ws(&mut ws, &cert_csr_pem).await?;
        tracing::info!(
            not_after = %cert.not_after,
            "received client certificate"
        );
        save_cert_from_payload(data_dir, &cert.cert_pem)?;
        Ok(())
    }
}

/// Save certificate PEM to disk.
/// The key is already saved separately.
fn save_cert_from_payload(data_dir: &Path, cert_pem: &str) -> error::Result<()> {
    // Load the key that was saved earlier
    let key_pem = state::load_agent_key(data_dir)?
        .ok_or_else(|| report!(Error::Enrollment("no agent key found on disk".to_string())))?;

    let cert_state = state::AgentCertState {
        cert_pem: cert_pem.to_string(),
        key_pem,
    };
    cert_state.save(data_dir)?;
    tracing::info!("agent certificate saved to disk");
    Ok(())
}

/// Enter the mTLS authenticated loop with automatic reconnection on cert rotation.
async fn run_authenticated_with_reconnect(
    host: &str,
    port: u16,
    base_url: &str,
    pki_addr: Option<&str>,
    app_dirs: &AppDirs,
    ca_pem: Option<&[u8]>,
) -> error::Result<()> {
    let config_dir = app_dirs.config_dir();
    let state_dir = app_dirs.state_dir();

    loop {
        let cert_state = state::AgentCertState::load(state_dir)?
            .ok_or_else(|| report!(Error::NoCertificates))?;
        let cert_not_after_ts = cert_state.cert_not_after_ms();

        let mtls_connector = match ca_pem {
            Some(pem) => client::build_tls_connector_with_client_cert(
                pem,
                &cert_state.cert_pem,
                &cert_state.key_pem,
            )?,
            None => client::build_system_trust_tls_connector_with_client_cert(
                &cert_state.cert_pem,
                &cert_state.key_pem,
            )?,
        };

        match client::run_authenticated_loop(
            host,
            port,
            base_url,
            pki_addr,
            ca_pem,
            mtls_connector,
            cert_not_after_ts,
            config_dir,
            state_dir,
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
            LoopOutcome::Restart => {
                tracing::info!("restart requested, exiting for external restart");
                return Ok(());
            }
        }
    }
}
