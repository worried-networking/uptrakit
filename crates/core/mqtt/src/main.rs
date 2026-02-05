mod cli;
mod controller_client;
mod error;
mod identity;
mod mqtt_client;
mod tenant_manager;

use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_internal_wire::{
    ControllerMessage, EnrollPayload, MqttHeartbeatPayload, MqttRegisterPayload,
    RequestCertificatePayload, ServiceMessage,
};

use crate::controller_client::{ConnectionMode, ControllerConnection};
use crate::error::{AppError, Result};
use crate::identity::Identity;
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

    // Resolve application directories
    let app_dirs = args.resolve_dirs().context_to()?;
    app_dirs.ensure_dirs().context_to()?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());

    // Load or create identity (config for CA cert, state for service identity)
    let mut identity = Identity::new(app_dirs.config_dir(), app_dirs.state_dir());
    identity.load().await.context_to()?;

    // Phase 1: Ensure we have a CA certificate (TOFU)
    if identity.ca_cert_pem.is_none() {
        tracing::info!("fetching CA certificate from controller (TOFU)");
        let ca_pem = controller_client::fetch_ca_cert(&args.controller_url, args.insecure)
            .await
            .context_to()?;
        identity.save_ca_cert(&ca_pem).await.context_to()?;
        tracing::info!("CA certificate saved");
    }

    // Phase 2: Ensure we're enrolled
    if identity.is_fresh() {
        tracing::info!("enrolling with controller");
        enroll(&mut identity, &args).await?;
    }

    // Phase 3: Ensure we have a certificate
    if identity.is_enrolled_only() {
        tracing::info!("requesting certificate from controller");
        request_certificate(&mut identity, &args).await?;
    }

    // Phase 4: Run the main loop with authenticated connection
    tracing::info!("connecting to controller (authenticated)");
    run_authenticated(&identity, &args, &instance_id).await
}

/// Enroll with the controller (anonymous connection).
async fn enroll(identity: &mut Identity, args: &cli::Args) -> Result<()> {
    // Ensure we have a keypair for the CSR
    identity.ensure_keypair().await.context_to()?;

    let mut conn = ControllerConnection::connect(
        &args.controller_url,
        identity,
        ConnectionMode::Anonymous,
        args.insecure,
    )
    .await
    .context_to()?;

    // Send enrollment request
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());

    conn.send(ServiceMessage::Enroll(EnrollPayload {
        hostname,
        friendly_name: args.friendly_name_or_hostname(),
        enrollment_token: args.enrollment_token.clone(),
        service_type: "mqtt".to_string(),
        host_info: None,
    }))
    .await
    .context_to()?;

    // Wait for enrollment response
    loop {
        match conn.recv().await.context_to()? {
            Some(ControllerMessage::Enrolled(payload)) => {
                let service_id = uuid::Uuid::parse_str(&payload.service_id)
                    .map_err(|_| report!(AppError::Protocol("invalid service_id".into())))?;

                identity
                    .save_enrollment(service_id, &payload.enrollment_secret)
                    .await
                    .context_to()?;

                tracing::info!(%service_id, status = %payload.status, "enrolled successfully");

                if payload.status == "approved" {
                    // Auto-approved, can proceed to certificate request
                    break;
                } else {
                    // Need to wait for approval
                    tracing::info!("waiting for approval from controller...");
                }
            }
            Some(ControllerMessage::Approved(payload)) => {
                tracing::info!(service_id = %payload.service_id, "approved by controller");
                break;
            }
            Some(ControllerMessage::Rejected(payload)) => {
                return Err(report!(AppError::Protocol(format!(
                    "enrollment rejected: service_id={}",
                    payload.service_id
                ))));
            }
            Some(ControllerMessage::Error(payload)) => {
                return Err(report!(AppError::Protocol(format!(
                    "enrollment error: {} - {}",
                    payload.code, payload.message
                ))));
            }
            Some(msg) => {
                tracing::debug!(?msg, "ignoring unexpected message during enrollment");
            }
            None => {
                return Err(report!(AppError::Protocol(
                    "connection closed during enrollment".into()
                )));
            }
        }
    }

    conn.close().await.context_to()?;
    Ok(())
}

/// Request certificate from controller (enrolled connection).
async fn request_certificate(identity: &mut Identity, args: &cli::Args) -> Result<()> {
    let mut conn = ControllerConnection::connect(
        &args.controller_url,
        identity,
        ConnectionMode::Enrolled,
        args.insecure,
    )
    .await
    .context_to()?;

    // Generate and send CSR
    let service_id = identity
        .service_id
        .ok_or_else(|| report!(AppError::Identity(identity::IdentityError::NotEnrolled)))?;

    let csr_pem = identity.generate_csr(service_id).context_to()?;

    conn.send(ServiceMessage::RequestCertificate(
        RequestCertificatePayload { csr_pem },
    ))
    .await
    .context_to()?;

    // Wait for certificate
    loop {
        match conn.recv().await.context_to()? {
            Some(ControllerMessage::Certificate(payload)) => {
                identity
                    .save_certificate(&payload.cert_pem)
                    .await
                    .context_to()?;
                tracing::info!("certificate received and saved");
                break;
            }
            Some(ControllerMessage::Error(payload)) => {
                return Err(report!(AppError::Protocol(format!(
                    "certificate request error: {} - {}",
                    payload.code, payload.message
                ))));
            }
            Some(msg) => {
                tracing::debug!(
                    ?msg,
                    "ignoring unexpected message during certificate request"
                );
            }
            None => {
                return Err(report!(AppError::Protocol(
                    "connection closed during certificate request".into()
                )));
            }
        }
    }

    conn.close().await.context_to()?;
    Ok(())
}

/// Run the main loop with authenticated mTLS connection.
async fn run_authenticated(identity: &Identity, args: &cli::Args, instance_id: &str) -> Result<()> {
    let mut conn = ControllerConnection::connect(
        &args.controller_url,
        identity,
        ConnectionMode::Authenticated,
        false, // Never use insecure mode for authenticated connections
    )
    .await
    .context_to()?;

    // Register with controller
    conn.send(ServiceMessage::Register(MqttRegisterPayload {
        instance_id: instance_id.to_string(),
        max_tenants: args.max_tenants,
        active_tenants: vec![], // Empty on fresh start
    }))
    .await
    .context_to()?;

    let mut tenant_mgr = TenantManager::new();
    let heartbeat_interval = Duration::from_secs(args.heartbeat_interval);
    let mut heartbeat_ticker = tokio::time::interval(heartbeat_interval);

    // Skip the first immediate tick
    heartbeat_ticker.tick().await;

    loop {
        tokio::select! {
            msg = conn.recv() => {
                match msg.context_to()? {
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
                        // In a full implementation, would update identity.ca_cert_pem
                        let _ = payload;
                    }
                    Some(ControllerMessage::RequestCertRenewal(_)) => {
                        tracing::info!("certificate renewal requested");
                        // In a full implementation, would trigger certificate renewal
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
            _ = heartbeat_ticker.tick() => {
                let active = tenant_mgr.active_tenant_ids();
                if let Err(e) = conn.send(ServiceMessage::Heartbeat(MqttHeartbeatPayload {
                    active_tenants: active,
                })).await {
                    tracing::error!(error = ?e, "heartbeat failed");
                    break;
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

    tracing::info!("shutdown complete");
    Ok(())
}
