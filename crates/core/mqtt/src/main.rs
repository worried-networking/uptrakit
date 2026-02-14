mod cli;
mod mqtt_client;
mod tenant_manager;

use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_build_info::BuildInfo;
use uptrakit_internal_wire::{
    ControllerMessage, DisconnectReason, DisconnectingPayload, MqttClientStatusPayload,
    MqttRegisterPayload, PingPayload, ServiceMessage, now_millis,
};
use uptrakit_service_sdk::{
    AuthenticatedContext, ControllerConnection, LoopOutcome, ServiceConfig, ServiceEnrollmentInfo,
    ServiceHandler,
};

use crate::tenant_manager::TenantManager;

struct MqttHandler {
    max_tenants: u32,
    ping_interval: u64,
    instance_id: String,
    tenant_mgr: TenantManager,
    status_rx: tokio::sync::mpsc::UnboundedReceiver<mqtt_client::MqttClientStatusEvent>,
}

impl ServiceHandler for MqttHandler {
    fn config(&self) -> ServiceConfig {
        ServiceConfig {
            dir_name: "mqtt",
            service_label: "uptrakit-mqtt service",
        }
    }

    fn enrollment_info(&self) -> ServiceEnrollmentInfo {
        ServiceEnrollmentInfo {
            service_type: uptrakit_internal_wire::ServiceType::Mqtt,
        }
    }

    fn run_authenticated_loop<'a>(
        &'a mut self,
        ctx: AuthenticatedContext<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = uptrakit_service_sdk::Result<LoopOutcome>> + Send + 'a>,
    > {
        Box::pin(async move {
            run_mqtt_authenticated_loop(MqttLoopParams {
                host: ctx.host,
                port: ctx.port,
                tls_connector: ctx.tls_connector,
                max_tenants: self.max_tenants,
                ping_interval: self.ping_interval,
                instance_id: &self.instance_id,
                tenant_mgr: &mut self.tenant_mgr,
                status_rx: &mut self.status_rx,
            })
            .await
        })
    }
}

#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    if args.common.version {
        print_build_info();
        return;
    }

    let filter = match "uptrakit_mqtt=info".parse() {
        Ok(directive) => EnvFilter::from_default_env().add_directive(directive),
        Err(_) => EnvFilter::from_default_env(),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let instance_id = generate_instance_id();
    tracing::info!(%instance_id, "starting uptrakit-mqtt service");

    let (status_tx, status_rx) = tokio::sync::mpsc::unbounded_channel();
    let tenant_mgr = TenantManager::new(Some(status_tx));

    let mut handler = MqttHandler {
        max_tenants: args.max_tenants,
        ping_interval: args.ping_interval,
        instance_id,
        tenant_mgr,
        status_rx,
    };

    if let Err(e) = uptrakit_service_sdk::run_service_lifecycle(&args.common, &mut handler).await {
        if e.current_context().is_receive_closed() {
            tracing::info!("disconnected by controller");
        } else {
            tracing::error!(error = %e, "mqtt service failed");
            std::process::exit(1);
        }
    }
}

fn print_build_info() {
    let build_info = BuildInfo::current(
        "uptrakit-mqtt",
        env!("CARGO_PKG_VERSION"),
        option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
    );
    let output = build_info.render_human();
    print!("{output}");
}

/// Generate a unique instance ID: `{hostname}-{uuid_v7_first_8_chars}`
fn generate_instance_id() -> String {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    let uuid_prefix = &uuid::Uuid::now_v7().to_string()[..8];
    format!("{host}-{uuid_prefix}")
}

/// Parameters for [`run_mqtt_authenticated_loop`].
struct MqttLoopParams<'a> {
    host: &'a str,
    port: u16,
    tls_connector: tokio_rustls::TlsConnector,
    max_tenants: u32,
    ping_interval: u64,
    instance_id: &'a str,
    tenant_mgr: &'a mut TenantManager,
    status_rx: &'a mut tokio::sync::mpsc::UnboundedReceiver<mqtt_client::MqttClientStatusEvent>,
}

/// Authenticated event loop (mTLS connection) — service-specific logic for MQTT.
async fn run_mqtt_authenticated_loop(params: MqttLoopParams<'_>) -> uptrakit_service_sdk::Result<LoopOutcome> {
    let MqttLoopParams {
        host,
        port,
        tls_connector,
        max_tenants,
        ping_interval: ping_interval_secs,
        instance_id,
        tenant_mgr,
        status_rx,
    } = params;

    tracing::info!("connecting to controller (authenticated)");
    let mut conn = ControllerConnection::connect(host, port, &tls_connector, None).await?;

    // Register with controller
    conn.send(ServiceMessage::Register(MqttRegisterPayload {
        instance_id: instance_id.to_string(),
        max_tenants,
        active_mqtt_clients: tenant_mgr.active_mqtt_client_ids(),
        protocol_version: uptrakit_internal_wire::PROTOCOL_VERSION,
    }))
    .await?;

    let ping_interval = Duration::from_secs(ping_interval_secs);
    let mut ping_ticker = tokio::time::interval(ping_interval);

    // Skip the first immediate tick
    ping_ticker.tick().await;

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| report!(uptrakit_service_sdk::EnrollmentError::Io(e)))?;

    let outcome = loop {
        tokio::select! {
            biased;

            msg = conn.recv() => {
                match msg? {
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
                    .await?;
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
