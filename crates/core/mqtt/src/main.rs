mod cli;
mod controller_client;
mod error;
mod mqtt_client;
mod tenant_manager;

use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_enrollment::ServiceIdentityState;
use uptrakit_internal_wire::{
    ControllerMessage, DisconnectReason, DisconnectingPayload, MqttRegisterPayload, PingPayload,
    ServiceMessage, now_millis,
};

use crate::controller_client::ControllerConnection;
use crate::error::{AppError, Result};
use crate::tenant_manager::TenantManager;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

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

async fn run(args: cli::Args) -> Result<()> {
    let instance_id = generate_instance_id();
    tracing::info!(%instance_id, "starting uptrakit-mqtt service");

    // Parse URL early
    let (host, port) = args.common.parsed_url().map_err(|s| {
        report!(AppError::Enrollment(
            uptrakit_enrollment::EnrollmentError::Enrollment(s)
        ))
    })?;
    let base_url = args.common.base_url();
    let pki_addr = args.common.pki_addr();

    // Resolve application directories
    let app_dirs = args.common.resolve_dirs("mqtt").context_to::<AppError>()?;
    app_dirs.ensure_dirs().map_err(|e| {
        report!(AppError::Enrollment(
            uptrakit_enrollment::EnrollmentError::Enrollment(format!(
                "failed to create directories: {e}"
            ))
        ))
    })?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());

    // Create and load identity state
    let mut identity = ServiceIdentityState::new(app_dirs.config_dir(), app_dirs.state_dir());
    identity.load().await.context_to::<AppError>()?;

    // --force-enroll: clear existing enrollment state (preserves CA cert)
    if args.common.force_enroll {
        tracing::info!("--force-enroll: clearing existing enrollment state");
        identity
            .clear_enrollment_state()
            .await
            .context_to::<AppError>()?;
    }

    // CA bootstrap: cached → --ca-cert file → --pki-addr → --tofu TOFU → system trust
    let ca_pem = uptrakit_enrollment::ca::bootstrap_ca(
        &mut identity,
        base_url,
        args.common.tofu,
        args.common.ca_cert.as_deref(),
        pki_addr,
    )
    .await
    .context_to::<AppError>()?;

    // Enrollment (if not yet certified)
    if !identity.is_certified() {
        let tls_connector = match ca_pem.as_deref() {
            Some(pem) => {
                uptrakit_enrollment::tls::build_tls_connector(pem).context_to::<AppError>()?
            }
            None => uptrakit_enrollment::tls::build_system_trust_tls_connector()
                .context_to::<AppError>()?,
        };

        if identity.is_enrolled_only() {
            tracing::info!("resuming enrollment (have service_id, awaiting certificate)");
            uptrakit_enrollment::ws::resume_enrollment(&mut identity, &host, port, &tls_connector)
                .await
                .context_to::<AppError>()?;
        } else {
            tracing::info!("starting fresh enrollment");
            let hostname = args.common.hostname();
            let friendly_name = args.common.friendly_name_or_hostname();

            uptrakit_enrollment::ws::run_enrollment(
                &mut identity,
                &host,
                port,
                &tls_connector,
                &hostname,
                &friendly_name,
                args.common.enrollment_token.as_deref(),
                "mqtt",
                None, // MQTT service doesn't collect host_info
            )
            .await
            .context_to::<AppError>()?;
        }

        tracing::info!("enrollment complete, certificate saved to disk");
    }

    // Run the main authenticated loop
    tracing::info!("connecting to controller (authenticated)");
    run_authenticated(&identity, &args, &instance_id, ca_pem.as_deref()).await
}

/// Run the main loop with authenticated mTLS connection.
async fn run_authenticated(
    identity: &ServiceIdentityState,
    args: &cli::Args,
    instance_id: &str,
    ca_pem: Option<&[u8]>,
) -> Result<()> {
    let cert_pem = identity.cert_pem().ok_or_else(|| {
        report!(AppError::Enrollment(
            uptrakit_enrollment::EnrollmentError::NotCertified
        ))
    })?;
    let key_pem = identity.key_pem().ok_or_else(|| {
        report!(AppError::Enrollment(
            uptrakit_enrollment::EnrollmentError::NotCertified
        ))
    })?;

    let client_config = match ca_pem {
        Some(pem) => uptrakit_enrollment::tls::build_mtls_client_config(pem, cert_pem, &key_pem)
            .context_to::<AppError>()?,
        None => {
            // System trust mTLS — build a ClientConfig with webpki roots and client cert.
            use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

            let root_store =
                rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

            let client_certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| {
                    report!(AppError::Enrollment(
                        uptrakit_enrollment::EnrollmentError::Tls(e.to_string())
                    ))
                })?;

            let client_key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(|e| {
                report!(AppError::Enrollment(
                    uptrakit_enrollment::EnrollmentError::Tls(e.to_string())
                ))
            })?;

            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_client_auth_cert(client_certs, client_key)
                .map_err(|e| {
                    report!(AppError::Enrollment(
                        uptrakit_enrollment::EnrollmentError::Rustls(e)
                    ))
                })?
        }
    };

    let controller_url = args.common.base_url();
    let mut conn = ControllerConnection::connect(controller_url, client_config)
        .await
        .context_to::<AppError>()?;

    // Register with controller
    conn.send(ServiceMessage::Register(MqttRegisterPayload {
        instance_id: instance_id.to_string(),
        max_tenants: args.max_tenants,
        active_tenants: vec![], // Empty on fresh start
    }))
    .await
    .context_to::<AppError>()?;

    let mut tenant_mgr = TenantManager::new();
    let ping_interval = Duration::from_secs(args.ping_interval);
    let mut ping_ticker = tokio::time::interval(ping_interval);

    // Skip the first immediate tick
    ping_ticker.tick().await;

    loop {
        tokio::select! {
            msg = conn.recv() => {
                match msg.context_to::<AppError>()? {
                    Some(ControllerMessage::Registered(payload)) => {
                        tracing::info!(instance_id = %payload.instance_id, "registered with controller");
                    }
                    Some(ControllerMessage::TenantAssignments(payload)) => {
                        tracing::info!(count = payload.tenants.len(), "received tenant assignments");
                        tenant_mgr.apply_assignments(payload.tenants).await;
                    }
                    Some(ControllerMessage::TenantConfigUpdated(payload)) => {
                        tracing::info!(tenant_id = %payload.tenant.tenant_id, "tenant config updated");
                        tenant_mgr.reload_tenant(payload.tenant).await;
                    }
                    Some(ControllerMessage::TenantRevoked(payload)) => {
                        tracing::info!(tenant_id = %payload.tenant_id, reason = %payload.reason, "tenant revoked");
                        tenant_mgr.stop_tenant(&payload.tenant_id).await;
                    }
                    Some(ControllerMessage::CaBundleUpdated(payload)) => {
                        tracing::info!("CA bundle updated");
                        let _ = payload;
                    }
                    Some(ControllerMessage::RequestCertRenewal(_)) => {
                        tracing::info!("certificate renewal requested");
                    }
                    Some(ControllerMessage::Pong(payload)) => {
                        let rtt = now_millis() - payload.agent_ts;
                        tracing::trace!(rtt_ms = rtt, "pong received");
                    }
                    Some(ControllerMessage::ServerRestarting(payload)) => {
                        tracing::info!(reason = %payload.reason, "server restarting, preparing for reconnect");
                    }
                    Some(msg) => {
                        tracing::debug!(?msg, "ignoring unexpected message");
                    }
                    None => {
                        tracing::warn!("connection closed by controller");
                        break;
                    }
                }
            }
            _ = ping_ticker.tick() => {
                if let Err(e) = conn.send(ServiceMessage::Ping(PingPayload {
                    agent_ts: now_millis(),
                })).await {
                    tracing::error!(error = ?e, "ping failed");
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received shutdown signal");
                break;
            }
        }
    }

    // Graceful shutdown: notify controller with active tenant list
    let active = tenant_mgr.active_tenant_ids();
    let _ = conn
        .send(ServiceMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Shutdown,
            active_tenants: active,
        }))
        .await;

    tracing::info!("shutting down MQTT clients");
    tenant_mgr.shutdown_all().await;

    tracing::info!("shutdown complete");
    Ok(())
}
