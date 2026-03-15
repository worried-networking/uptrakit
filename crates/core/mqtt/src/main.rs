/// Abort the current publish batch on first error.
///
/// When a publish or subscribe operation fails (typically due to a broker
/// connection timeout), there is no point continuing with the remaining
/// operations in the batch — the data will be automatically republished
/// on the next `SoftwareStates` push or broker reconnect.  Aborting early
/// prevents the service event loop from being blocked for
/// `N × OPERATION_TIMEOUT` seconds, keeping signal handling responsive.
macro_rules! publish_or_abort {
    ($expr:expr, $client_id:expr, $what:expr) => {
        if let Err(e) = $expr {
            tracing::warn!(
                error = %e,
                mqtt_client_id = %$client_id,
                concat!("failed to ", $what, "; aborting remaining publishes for this client"),
            );
            return;
        }
    };
}

/// Best-effort variant of `publish_or_abort!` for cleanup operations.
///
/// Cleanup must continue even if individual operations fail (e.g. broker
/// connection down for one topic). Logs a warning on failure and continues
/// with the remaining cleanup operations.
macro_rules! publish_best_effort {
    ($expr:expr, $client_id:expr, $what:expr) => {
        if let Err(e) = $expr {
            tracing::warn!(
                error = %e,
                mqtt_client_id = %$client_id,
                concat!("failed to ", $what, " (cleanup, continuing)"),
            );
        }
    };
}

mod cli;
mod client_manager;
mod ha_discovery;
mod mqtt_client;
mod state_publisher;
mod tenant_manager;

use clap::Parser;
use rootcause::prelude::*;
use std::collections::BTreeSet;

use uptrakit_internal_wire::{
    Capability, ControllerMessage, DisconnectingPayload, MqttClientStatusPayload,
    MqttRegisterPayload, ServiceMessage,
};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, default_resolve_shutdown,
};

use crate::tenant_manager::TenantManager;

/// Capacity of the bounded MQTT service-event channel.
///
/// Events are ephemeral status/reconnect notifications. Dropping events when the
/// channel is full results in at most a missing MQTT publish that auto-recovers on
/// the next state push from the controller. 512 is generous for typical deployments
/// (tens of MQTT clients) while bounding memory growth under backpressure.
const MQTT_EVENT_CHANNEL_CAPACITY: usize = 512;

struct MqttHandler {
    max_tenants: u32,
    instance_id: String,
    tenant_mgr: TenantManager,
    event_rx: tokio::sync::mpsc::Receiver<crate::mqtt_client::MqttServiceEvent>,
}

#[async_trait::async_trait]
impl ServiceHandler for MqttHandler {
    const DIR_NAME: &'static str = "mqtt";
    const SERVICE_LABEL: &'static str = "uptrakit-mqtt service";
    const SERVICE_APP_NAME: &'static str = env!("CARGO_PKG_NAME");

    type ServiceEvent = Option<crate::mqtt_client::MqttServiceEvent>;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        conn.send(ServiceMessage::Register(MqttRegisterPayload {
            instance_id: self.instance_id.clone(),
            max_tenants: self.max_tenants,
            active_mqtt_clients: self.tenant_mgr.active_mqtt_client_ids(),
            capabilities: mqtt_capabilities(),
        }))
        .await
        .context_to::<LoopError>()?;
        Ok(())
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match msg {
            ControllerMessage::Registered(payload) => {
                tracing::info!(instance_id = %payload.instance_id, "registered with controller");
                Ok(None)
            }
            ControllerMessage::TenantAssignments(payload) => {
                tracing::info!(count = payload.tenants.len(), "received tenant assignments");
                self.tenant_mgr.apply_assignments(payload.tenants).await;
                Ok(None)
            }
            ControllerMessage::TenantConfigUpdated(payload) => {
                tracing::info!(mqtt_client_id = %payload.tenant.mqtt_client_id, "mqtt client config updated");
                self.tenant_mgr.reload_client(payload.tenant).await;
                Ok(None)
            }
            ControllerMessage::TenantRevoked(payload) => {
                tracing::info!(mqtt_client_id = %payload.mqtt_client_id, reason = %payload.reason, "mqtt client revoked");
                self.tenant_mgr.stop_client(&payload.mqtt_client_id).await;
                Ok(None)
            }
            ControllerMessage::SoftwareStates(payload) => {
                tracing::debug!(
                    tenant_id = %payload.tenant_id,
                    items = payload.items.len(),
                    "received SoftwareStates"
                );
                self.tenant_mgr.update_software_states(payload).await;
                Ok(None)
            }
            ControllerMessage::HostConnectivityUpdated(payload) => {
                tracing::debug!(
                    tenant_id = %payload.tenant_id,
                    updates = payload.updates.len(),
                    "received HostConnectivityUpdated"
                );
                self.tenant_mgr
                    .handle_host_connectivity_updated(payload.tenant_id, payload.updates)
                    .await;
                Ok(None)
            }
            _ => {
                tracing::debug!("ignoring unrecognized message in authenticated loop");
                Ok(None)
            }
        }
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.event_rx.recv().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        use crate::mqtt_client::MqttServiceEvent;
        match event {
            Some(MqttServiceEvent::Status(status)) => {
                conn.send_best_effort(ServiceMessage::MqttClientStatus(MqttClientStatusPayload {
                    mqtt_client_id: status.mqtt_client_id,
                    status: status.status,
                }))
                .await;
                Ok(None)
            }
            Some(MqttServiceEvent::Reconnected(id)) => {
                self.tenant_mgr.handle_reconnected(&id).await;
                Ok(None)
            }
            Some(MqttServiceEvent::HaOnline(id)) => {
                self.tenant_mgr.handle_ha_online(&id).await;
                Ok(None)
            }
            Some(MqttServiceEvent::Command {
                mqtt_client_id,
                topic,
            }) => {
                if let Some(payload) = self
                    .tenant_mgr
                    .resolve_host_security_batch_update_trigger(mqtt_client_id, &topic)
                {
                    conn.send_best_effort(ServiceMessage::MqttTriggerHostBatchUpdate(payload))
                        .await;
                } else if let Some(payload) = self
                    .tenant_mgr
                    .resolve_host_batch_update_trigger(mqtt_client_id, &topic)
                {
                    conn.send_best_effort(ServiceMessage::MqttTriggerHostBatchUpdate(payload))
                        .await;
                } else if let Some(payload) = self
                    .tenant_mgr
                    .resolve_update_trigger(mqtt_client_id, &topic)
                {
                    conn.send_best_effort(ServiceMessage::MqttTriggerUpdate(payload))
                        .await;
                } else {
                    tracing::debug!(
                        %mqtt_client_id,
                        %topic,
                        "received MQTT command on unknown topic, ignoring"
                    );
                }
                Ok(None)
            }
            None => {
                tracing::warn!("event channel closed");
                Ok(Some(LoopOutcome::Disconnected))
            }
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        mqtt_capabilities()
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        _shutdown_timeout: std::time::Duration,
    ) -> LoopOutcome {
        let (reason, outcome) = default_resolve_shutdown(cause);

        // Notify controller with active MQTT client list.
        let active = self.tenant_mgr.active_mqtt_client_ids();
        conn.send_best_effort(ServiceMessage::Disconnecting(DisconnectingPayload {
            reason,
            active_mqtt_clients: active,
        }))
        .await;

        tracing::info!("shutting down MQTT clients");
        self.tenant_mgr.shutdown_all().await;
        tracing::info!("shutdown complete");

        outcome
    }
}

/// Capabilities advertised by the MQTT service.
///
/// `SystemService` marks this service as global infrastructure (routed to the
/// `system_services` table instead of the per-tenant `services` table).
fn mqtt_capabilities() -> BTreeSet<Capability> {
    [
        Capability::SystemService,
        Capability::MqttBridge,
        Capability::GracefulShutdown,
    ]
    .into_iter()
    .collect()
}

#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            "uptrakit-mqtt",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    init_tracing("uptrakit_mqtt", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    let instance_id = generate_instance_id();
    tracing::info!(%instance_id, "starting uptrakit-mqtt service");

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(MQTT_EVENT_CHANNEL_CAPACITY);
    let tenant_mgr = TenantManager::new(Some(event_tx));

    let mut handler = MqttHandler {
        // Convert Option<NonZeroU32> → u32 using 0 as the wire sentinel for "unlimited".
        max_tenants: args.max_tenants.map_or(0, |n| n.get()),
        instance_id,
        tenant_mgr,
        event_rx,
    };

    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-mqtt",
        &args.common,
        &mut handler,
    )
    .await;
}

/// Generate a unique instance ID: `{hostname}-{uuid_v7_first_8_chars}`
fn generate_instance_id() -> String {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    let uuid_str = uuid::Uuid::now_v7().to_string();
    // UUID v7 format is `xxxxxxxx-xxxx-...`, so the first 8 chars are always
    // ASCII hex digits. Using `.get()` for defence-in-depth.
    let uuid_prefix = uuid_str.get(..8).unwrap_or(&uuid_str);
    format!("{host}-{uuid_prefix}")
}

/// Initialize `tracing_subscriber` with a verbosity-aware filter.
fn init_tracing(own_module: &str, verbosity: u8) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    if verbosity > 3 {
        eprintln!(
            "warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)"
        );
    }

    let directive = match verbosity {
        0 => format!("{own_module}=info"),
        1 => format!("{own_module}=debug"),
        2 => "uptrakit=debug".to_string(),
        _ => "uptrakit=trace".to_string(),
    };
    let mut filter = EnvFilter::from_default_env();
    if let Ok(d) = directive.parse() {
        filter = filter.add_directive(d);
    }
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .init();
}
