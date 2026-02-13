mod cli;
mod error;
mod mqtt_client;
mod tenant_manager;

use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_internal_wire::{
    ControllerMessage, DisconnectReason, DisconnectingPayload, MqttClientStatusPayload,
    MqttRegisterPayload, PingPayload, ServiceMessage, now_millis,
};
use uptrakit_service_sdk::{ControllerConnection, ServiceIdentityState};

use crate::error::{AppError, Result};
use crate::tenant_manager::TenantManager;

/// Outcome of the authenticated event loop.
enum LoopOutcome {
    /// SIGINT/SIGTERM received — shut down cleanly.
    Shutdown,
    /// Certificate rotated — reload from disk and reconnect.
    Reconnect,
    /// Connection closed by controller — no special action.
    Disconnected,
}

#[tokio::main]
async fn main() {
    let filter = match "uptrakit_mqtt=info".parse() {
        Ok(directive) => EnvFilter::from_default_env().add_directive(directive),
        Err(_) => EnvFilter::from_default_env(),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let args = cli::Args::parse();

    if let Err(mut e) = run(args).await {
        if e.current_context_mut().is_receive_closed() {
            tracing::info!("disconnected by controller");
        } else {
            tracing::error!(error = %e, "mqtt service failed");
            std::process::exit(1);
        }
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
            uptrakit_service_sdk::EnrollmentError::Enrollment(s)
        ))
    })?;
    let base_url = args.common.base_url();
    let pki_addr = args.common.pki_addr();

    // Resolve application directories
    let app_dirs = args.common.resolve_dirs("mqtt").map_err(|e| {
        report!(AppError::Enrollment(
            uptrakit_service_sdk::EnrollmentError::Enrollment(e.to_string())
        ))
    })?;
    app_dirs.ensure_dirs().map_err(|e| {
        report!(AppError::Enrollment(
            uptrakit_service_sdk::EnrollmentError::Enrollment(format!(
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
    let ca_pem = uptrakit_service_sdk::ca::bootstrap_ca(
        &mut identity,
        base_url,
        args.common.tofu,
        args.common.tofu_fingerprint.as_deref(),
        args.common.ca_cert.as_deref(),
        pki_addr,
    )
    .await
    .context_to::<AppError>()?;

    // Check for existing certificate
    if identity.is_certified() {
        let cert_not_after_ts = identity.cert_not_after_ms();
        let cert_expired =
            cert_not_after_ts.is_some_and(|ts| uptrakit_internal_wire::now_millis() >= ts);

        if cert_expired {
            tracing::warn!("certificate expired, falling back to fresh enrollment");
            identity
                .clear_enrollment_state()
                .await
                .context_to::<AppError>()?;
            // Fall through to enrollment below
        } else {
            tracing::info!("loaded existing certificate from disk");
            match run_authenticated_with_reconnect(
                &host,
                port,
                ca_pem.as_deref(),
                &identity,
                &args,
                &instance_id,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(mut e) => {
                    if e.current_context_mut().is_cert_expired() {
                        tracing::warn!("certificate expired, falling back to enrollment");
                        identity
                            .clear_enrollment_state()
                            .await
                            .context_to::<AppError>()?;
                        // Fall through to enrollment below
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }

    // Enrollment with backoff loop
    let tls_connector = match ca_pem.as_deref() {
        Some(pem) => {
            uptrakit_service_sdk::tls::build_tls_connector(pem).context_to::<AppError>()?
        }
        None => uptrakit_service_sdk::tls::build_system_trust_tls_connector()
            .context_to::<AppError>()?,
    };

    let mut enrollment_backoff =
        uptrakit_service_sdk::Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    loop {
        match do_enrollment(&args, &host, port, &mut identity, &tls_connector).await {
            Ok(()) => break,
            Err(mut e) => {
                if e.current_context_mut().is_receive_closed() {
                    let delay = enrollment_backoff.next_delay();
                    tracing::info!("disconnected during enrollment, reconnecting in {delay:?}");
                    tokio::time::sleep(delay).await;
                    // Reload identity in case enrollment partially completed
                    identity.load().await.context_to::<AppError>()?;
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
        ca_pem.as_deref(),
        &identity,
        &args,
        &instance_id,
    )
    .await
}

/// Run enrollment using the shared SDK crate.
async fn do_enrollment(
    args: &cli::Args,
    host: &str,
    port: u16,
    identity: &mut ServiceIdentityState,
    tls_connector: &tokio_rustls::TlsConnector,
) -> Result<()> {
    if identity.is_enrolled_only() {
        // Resume: reconnect with Bearer header (existing service.json)
        tracing::info!("reconnecting with enrollment secret");
        uptrakit_service_sdk::ws::resume_enrollment(identity, host, port, tls_connector)
            .await
            .context_to::<AppError>()?;
    } else {
        // Fresh enrollment
        let hostname = args.common.hostname();
        let friendly_name = args.common.friendly_name_or_hostname();

        tracing::info!("enrolling via WebSocket");
        uptrakit_service_sdk::ws::run_enrollment(uptrakit_service_sdk::ws::EnrollmentParams {
            identity,
            host,
            port,
            tls_connector,
            hostname: &hostname,
            friendly_name: &friendly_name,
            enrollment_token: args.common.enrollment_token.as_deref(),
            service_type: uptrakit_internal_wire::ServiceType::Mqtt,
            host_info: None, // MQTT service doesn't collect host_info
        })
        .await
        .context_to::<AppError>()?;
    }

    tracing::info!("enrollment complete, certificate saved to disk");
    Ok(())
}

/// Enter the mTLS authenticated loop with automatic reconnection on cert rotation.
async fn run_authenticated_with_reconnect(
    host: &str,
    port: u16,
    ca_pem: Option<&[u8]>,
    identity: &ServiceIdentityState,
    args: &cli::Args,
    instance_id: &str,
) -> Result<()> {
    let (status_tx, mut status_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut tenant_mgr = TenantManager::new(Some(status_tx));

    let mut reconnect_backoff =
        uptrakit_service_sdk::Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    loop {
        let cert_pem = identity.cert_pem().ok_or_else(|| {
            report!(AppError::Enrollment(
                uptrakit_service_sdk::EnrollmentError::NotCertified
            ))
        })?;
        let key_pem = identity.key_pem().ok_or_else(|| {
            report!(AppError::Enrollment(
                uptrakit_service_sdk::EnrollmentError::NotCertified
            ))
        })?;

        let mtls_connector = match ca_pem {
            Some(pem) => uptrakit_service_sdk::tls::build_tls_connector_with_client_cert(
                pem, cert_pem, &key_pem,
            )
            .context_to::<AppError>()?,
            None => uptrakit_service_sdk::tls::build_system_trust_tls_connector_with_client_cert(
                cert_pem, &key_pem,
            )
            .context_to::<AppError>()?,
        };

        match run_authenticated_loop(AuthenticatedLoopParams {
            host,
            port,
            tls_connector: mtls_connector,
            args,
            instance_id,
            tenant_mgr: &mut tenant_mgr,
            status_rx: &mut status_rx,
        })
        .await?
        {
            LoopOutcome::Shutdown => return Ok(()),
            LoopOutcome::Reconnect => {
                // Certificate rotation is expected; reset backoff and reconnect quickly.
                reconnect_backoff.reset();
                tracing::info!("reconnecting with new certificate");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            LoopOutcome::Disconnected => {
                let delay = reconnect_backoff.next_delay();
                tracing::warn!("disconnected by controller, reconnecting in {delay:?}");
                tokio::time::sleep(delay).await;
                continue;
            }
        }
    }
}

/// Parameters for [`run_authenticated_loop`].
struct AuthenticatedLoopParams<'a> {
    host: &'a str,
    port: u16,
    tls_connector: tokio_rustls::TlsConnector,
    args: &'a cli::Args,
    instance_id: &'a str,
    tenant_mgr: &'a mut TenantManager,
    status_rx: &'a mut tokio::sync::mpsc::UnboundedReceiver<mqtt_client::MqttClientStatusEvent>,
}

/// Authenticated event loop (mTLS connection).
async fn run_authenticated_loop(params: AuthenticatedLoopParams<'_>) -> Result<LoopOutcome> {
    let AuthenticatedLoopParams {
        host,
        port,
        tls_connector,
        args,
        instance_id,
        tenant_mgr,
        status_rx,
    } = params;

    tracing::info!("connecting to controller (authenticated)");
    let mut conn = ControllerConnection::connect(host, port, &tls_connector, None)
        .await
        .context_to::<AppError>()?;

    // Register with controller
    conn.send(ServiceMessage::Register(MqttRegisterPayload {
        instance_id: instance_id.to_string(),
        max_tenants: args.max_tenants,
        active_mqtt_clients: tenant_mgr.active_mqtt_client_ids(),
        protocol_version: uptrakit_internal_wire::PROTOCOL_VERSION,
    }))
    .await
    .context_to::<AppError>()?;

    let ping_interval = Duration::from_secs(args.ping_interval);
    let mut ping_ticker = tokio::time::interval(ping_interval);

    // Skip the first immediate tick
    ping_ticker.tick().await;

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context_to::<AppError>()?;

    let outcome = loop {
        tokio::select! {
            biased;

            msg = conn.recv() => {
                match msg.context_to::<AppError>()? {
                    Some(ControllerMessage::Registered(payload)) => {
                        tracing::info!(instance_id = %payload.instance_id, "registered with controller");
                    }
                    Some(ControllerMessage::Pong(payload)) => {
                        let rtt = now_millis() - payload.service_ts;
                        tracing::trace!(rtt_ms = rtt, "received pong");
                    }
                    Some(ControllerMessage::TenantAssignments(payload)) => {
                        tracing::info!(count = payload.tenants.len(), "received tenant assignments");
                        tenant_mgr.apply_assignments(payload.tenants).await;
                    }
                    Some(ControllerMessage::TenantConfigUpdated(payload)) => {
                        tracing::info!(mqtt_client_id = %payload.tenant.mqtt_client_id, "mqtt client config updated");
                        tenant_mgr.reload_client(payload.tenant).await;
                    }
                    Some(ControllerMessage::TenantRevoked(payload)) => {
                        tracing::info!(mqtt_client_id = %payload.mqtt_client_id, reason = %payload.reason, "mqtt client revoked");
                        tenant_mgr.stop_client(&payload.mqtt_client_id).await;
                    }
                    Some(ControllerMessage::ServiceSettings(settings)) => {
                        tracing::trace!(
                            renewal_window_hours = settings.renewal_window_hours,
                            "received service settings"
                        );
                        if settings.protocol_version != uptrakit_internal_wire::PROTOCOL_VERSION {
                            tracing::warn!(
                                reported = settings.protocol_version,
                                expected = uptrakit_internal_wire::PROTOCOL_VERSION,
                                "controller protocol version mismatch"
                            );
                        }
                    }
                    Some(ControllerMessage::CaBundleUpdated(payload)) => {
                        tracing::info!("received CA bundle update from controller");
                        let _ = payload;
                    }
                    Some(ControllerMessage::RequestCertRenewal(_)) => {
                        tracing::info!("certificate renewal requested (not yet implemented for MQTT)");
                    }
                    Some(ControllerMessage::ServerRestarting(payload)) => {
                        tracing::info!(reason = %payload.reason, "controller is restarting");
                        // Connection will close, reconnect logic handles the rest
                    }
                    Some(_) => {
                        tracing::debug!("ignoring unrecognized message in authenticated loop");
                    }
                    None => {
                        // Connection closed — check close reason
                        match conn.close_reason() {
                            Some("certificate rotated") => {
                                tracing::info!("connection closed: certificate rotated");
                                break LoopOutcome::Reconnect;
                            }
                            Some("certificate revoked") => {
                                tracing::warn!("connection closed: certificate revoked");
                                break LoopOutcome::Disconnected;
                            }
                            Some(reason) => {
                                tracing::warn!(%reason, "connection closed by controller");
                                break LoopOutcome::Disconnected;
                            }
                            None => {
                                tracing::info!("connection closed by controller");
                                break LoopOutcome::Disconnected;
                            }
                        }
                    }
                }
            }
            status = status_rx.recv() => {
                let Some(status) = status else {
                    tracing::warn!("status channel closed");
                    break LoopOutcome::Disconnected;
                };
                conn.send_best_effort(ServiceMessage::MqttClientStatus(MqttClientStatusPayload {
                    mqtt_client_id: status.mqtt_client_id,
                    status: status.status,
                })).await;
            }
            _ = ping_ticker.tick() => {
                let service_ts = now_millis();
                tracing::trace!(service_ts, "sending ping");
                conn.send(ServiceMessage::Ping(PingPayload { service_ts }))
                    .await
                    .context_to::<AppError>()?;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received SIGINT, initiating graceful shutdown");
                break handle_graceful_shutdown(&mut conn, tenant_mgr).await;
            }
            _ = async {
                #[cfg(unix)]
                {
                    sigterm.recv().await;
                }
                #[cfg(not(unix))]
                {
                    futures_util::future::pending::<()>().await;
                }
            } => {
                tracing::info!("received SIGTERM, initiating graceful shutdown");
                break handle_graceful_shutdown(&mut conn, tenant_mgr).await;
            }
        }
    };

    // Best-effort close — the peer may have already disconnected.
    let _ = conn.close().await;

    Ok(outcome)
}

/// Handle graceful shutdown: send Disconnecting message with active MQTT client list.
async fn handle_graceful_shutdown(
    conn: &mut ControllerConnection,
    tenant_mgr: &mut TenantManager,
) -> LoopOutcome {
    // Notify controller with active MQTT client list
    let active = tenant_mgr.active_mqtt_client_ids();
    conn.send_best_effort(ServiceMessage::Disconnecting(DisconnectingPayload {
        reason: DisconnectReason::Shutdown,
        active_mqtt_clients: active,
    }))
    .await;

    tracing::info!("shutting down MQTT clients");
    tenant_mgr.shutdown_all().await;
    tracing::info!("shutdown complete");

    LoopOutcome::Shutdown
}
