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
mod extension;
mod ha_discovery;
mod mqtt_client;
mod state_publisher;
mod tenant_manager;
mod types;

use base64::Engine as _;
use clap::Parser;
use rootcause::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use uptrakit_internal_wire::{
    Capability, ControllerMessage, DisconnectingPayload, RegisterPayload, ServiceMessage,
    WorkloadClaimPayload,
    payloads::{ServiceConfigEntry, ServiceConfigUpdatedPayload},
};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceConfigProxy, ServiceHandler,
    ServiceIdentityState, ShutdownCause, default_resolve_shutdown,
};

use crate::client_manager::ParsedMqttClientConfig;
use crate::tenant_manager::TenantManager;

/// Capacity of the bounded MQTT service-event channel.
///
/// Events are ephemeral status/reconnect notifications. Dropping events when the
/// channel is full results in at most a missing MQTT publish that auto-recovers on
/// the next state push from the controller. 512 is generous for typical deployments
/// (tens of MQTT clients) while bounding memory growth under backpressure.
const MQTT_EVENT_CHANNEL_CAPACITY: usize = 512;

/// Prefix for client config store keys.
///
/// Each MQTT client is stored under `"clients.{uuid}"` in the service config store.
const CONFIG_KEY_PREFIX: &str = "clients.";

struct MqttHandler {
    tenant_mgr: TenantManager,
    event_rx: tokio::sync::mpsc::Receiver<crate::mqtt_client::MqttServiceEvent>,
    /// In-memory snapshot of all parsed MQTT client configs.
    ///
    /// Updated on `ServiceConfigDelivery` and `ServiceConfigUpdated`.
    configs: Vec<ParsedMqttClientConfig>,
    /// Config keys that the controller has granted to this instance.
    ///
    /// Only clients whose config key is in this set should be started.
    /// Updated on `WorkloadClaimResult`.
    granted_keys: BTreeSet<String>,
    /// Correlates `StoreServiceConfig` / `DeleteServiceConfig` requests with ACKs.
    config_proxy: ServiceConfigProxy,
    /// Private key used to decrypt ECIES-encrypted sensitive extension params.
    private_key_der: Option<Vec<u8>>,
    /// Base64-encoded uncompressed P-256 public key for extension param encryption.
    encryption_public_key: Option<String>,
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
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        self.private_key_der = identity.private_key_pkcs8_der();
        self.encryption_public_key = identity
            .public_key_raw()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));

        // Declare capabilities immediately so the controller can set session
        // flags correctly even on first connect (before DB has stored caps).
        conn.send(ServiceMessage::Register(RegisterPayload::new(
            mqtt_capabilities(),
        )))
        .await
        .context_to::<LoopError>()?;

        Ok(())
    }

    async fn on_settings(
        &mut self,
        _settings: &uptrakit_internal_wire::ServiceSettingsPayload,
        conn: &mut ControllerConnection,
    ) {
        // Register UI extensions only when the agreed capability set includes
        // UiExtensions. The controller refreshes its gating flags from the
        // Register message before delivering ServiceSettings, so the controller
        // has already updated its flags by the time ExtensionRegister is received.
        if conn
            .agreed_capabilities()
            .contains(&Capability::UiExtensions)
        {
            let register_payload =
                extension::build_register_payload(self.encryption_public_key.clone());
            if let Err(e) = conn
                .send(ServiceMessage::ExtensionRegister(register_payload))
                .await
            {
                tracing::warn!(error = %e, "failed to register UI extensions");
            }

            let actions_payload = uptrakit_internal_wire::extension::ExtensionActionsPayload::new(
                extension::build_actions(),
            );
            if let Err(e) = conn
                .send(ServiceMessage::ExtensionActionsRegister(actions_payload))
                .await
            {
                tracing::warn!(error = %e, "failed to register extension actions");
            }
        }
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match msg {
            ControllerMessage::ServiceConfigDelivery(payload) => {
                tracing::info!(
                    count = payload.entries.len(),
                    "received ServiceConfigDelivery"
                );
                let parsed = parse_client_configs(payload.entries);
                self.configs = parsed.clone();
                // Send WorkloadClaim with the full desired config set.
                self.send_workload_claim(conn).await;
                // Apply configs for any already-granted keys (empty on first connect,
                // populated on reconnect if WorkloadClaimResult arrives before this).
                self.apply_granted_configs().await;
                Ok(None)
            }
            ControllerMessage::ServiceConfigUpdated(payload) => {
                tracing::debug!("received ServiceConfigUpdated");
                self.apply_config_update(payload).await;
                // Re-claim with updated config set.
                self.send_workload_claim(conn).await;
                self.apply_granted_configs().await;
                Ok(None)
            }
            ControllerMessage::WorkloadClaimResult(payload) => {
                tracing::info!(
                    granted = payload.granted.len(),
                    rejected = payload.rejected.len(),
                    "received WorkloadClaimResult"
                );
                self.granted_keys = payload.granted;
                // Stop clients for rejected/revoked keys.
                for key in &payload.rejected {
                    if let Some(client_id) = parse_client_key(key) {
                        self.tenant_mgr.stop_client(&client_id).await;
                    }
                }
                // Apply configs for granted keys.
                self.apply_granted_configs().await;
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

    fn on_service_config_ack(
        &self,
        ack: uptrakit_internal_wire::payloads::ServiceConfigAckPayload,
    ) {
        self.config_proxy.complete(&ack.request_id.clone(), ack);
    }

    async fn on_extension_request(
        &mut self,
        request: uptrakit_internal_wire::extension::ExtensionRequestPayload,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        let request_id = request.request_id.clone();

        // List action — read-only, no config proxy needed.
        if let Some(response) = extension::handle_list_action(&request, &self.configs) {
            return conn
                .send(ServiceMessage::ExtensionResponse(response))
                .await
                .map_err(|e| {
                    report!(LoopError::Other(format!(
                        "failed to send extension response: {e}"
                    )))
                });
        }
        if request.action_id == extension::ACTION_GET {
            if let Some(response) = extension::handle_get_action(&request, &self.configs) {
                return conn
                    .send(ServiceMessage::ExtensionResponse(response))
                    .await
                    .map_err(|e| {
                        report!(LoopError::Other(format!(
                            "failed to send extension response: {e}"
                        )))
                    });
            }
            return extension::send_error_response(
                conn,
                request_id,
                "missing or invalid MQTT client id",
            )
            .await;
        }

        // Write actions — create, edit, delete.
        match request.action_id.as_str() {
            extension::ACTION_CREATE => {
                self.handle_create_client(request, conn).await?;
            }
            extension::ACTION_EDIT => {
                self.handle_edit_client(request, conn).await?;
            }
            extension::ACTION_DELETE => {
                self.handle_delete_client(request, conn).await?;
            }
            _ => {
                extension::send_error_response(
                    conn,
                    request_id,
                    format!("unknown action: {}", request.action_id),
                )
                .await?;
            }
        }
        Ok(())
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
                tracing::debug!(
                    mqtt_client_id = %status.mqtt_client_id,
                    status = %status.status,
                    "MQTT client connection status changed"
                );
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
                    conn.send_best_effort(ServiceMessage::ServiceTriggerHostBatchUpdate(payload))
                        .await;
                } else if let Some(payload) = self
                    .tenant_mgr
                    .resolve_host_batch_update_trigger(mqtt_client_id, &topic)
                {
                    conn.send_best_effort(ServiceMessage::ServiceTriggerHostBatchUpdate(payload))
                        .await;
                } else if let Some(payload) = self
                    .tenant_mgr
                    .resolve_update_trigger(mqtt_client_id, &topic)
                {
                    conn.send_best_effort(ServiceMessage::ServiceTriggerUpdate(payload))
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

        conn.send_best_effort(ServiceMessage::Disconnecting(DisconnectingPayload::new(
            reason,
        )))
        .await;

        tracing::info!("shutting down MQTT clients");
        self.tenant_mgr.shutdown_all().await;
        tracing::info!("shutdown complete");

        outcome
    }
}

impl MqttHandler {
    /// Apply an incremental config update from the controller.
    async fn apply_config_update(&mut self, payload: ServiceConfigUpdatedPayload) {
        // Parse newly changed entries.
        let changed = parse_client_configs(payload.changed);

        // Collect deleted UUIDs and update the in-memory snapshot.
        let mut deleted_ids: Vec<Uuid> = Vec::new();
        for deleted_key in &payload.deleted {
            if let Some(mqtt_client_id) = parse_client_key(&deleted_key.key) {
                self.configs.retain(|c| c.mqtt_client_id != mqtt_client_id);
                deleted_ids.push(mqtt_client_id);
            }
        }
        for config in &changed {
            let id = config.mqtt_client_id;
            if let Some(existing) = self.configs.iter_mut().find(|c| c.mqtt_client_id == id) {
                *existing = config.clone();
            } else {
                self.configs.push(config.clone());
            }
        }

        // Stop deleted clients.
        for mqtt_client_id in deleted_ids {
            self.tenant_mgr.stop_client(&mqtt_client_id).await;
            // Remove deleted keys from granted set.
            self.granted_keys
                .remove(&format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}"));
        }
    }

    /// Compute the desired config claims from the current in-memory configs.
    ///
    /// Returns a `BTreeMap<config_key, tenant_id>` for all enabled configs with
    /// a valid (non-nil) tenant_id.
    fn compute_desired_claims(&self) -> BTreeMap<String, Uuid> {
        self.configs
            .iter()
            .filter(|c| c.enabled && c.tenant_id != Uuid::nil())
            .map(|c| {
                (
                    format!("{CONFIG_KEY_PREFIX}{}", c.mqtt_client_id),
                    c.tenant_id,
                )
            })
            .collect()
    }

    /// Send a `WorkloadClaim` with the full desired config set to the controller.
    async fn send_workload_claim(&self, conn: &mut ControllerConnection) {
        let claims = self.compute_desired_claims();
        tracing::info!(keys = claims.len(), "sending WorkloadClaim");
        if let Err(e) = conn
            .send(ServiceMessage::WorkloadClaim(WorkloadClaimPayload::new(
                claims,
            )))
            .await
        {
            tracing::warn!(error = %e, "failed to send WorkloadClaim");
        }
    }

    /// Apply only configs whose keys are in `granted_keys` to the tenant manager.
    ///
    /// Configs for keys that have not been granted are retained in memory but
    /// their MQTT clients are not started.
    async fn apply_granted_configs(&mut self) {
        let granted: Vec<ParsedMqttClientConfig> = self
            .configs
            .iter()
            .filter(|c| {
                let key = format!("{CONFIG_KEY_PREFIX}{}", c.mqtt_client_id);
                self.granted_keys.contains(&key)
            })
            .cloned()
            .collect();
        self.tenant_mgr.apply_configs(granted).await;
    }

    fn parse_request_config(
        &self,
        request: &uptrakit_internal_wire::extension::ExtensionRequestPayload,
        existing: Option<&ParsedMqttClientConfig>,
    ) -> Result<ParsedMqttClientConfig, String> {
        #[derive(serde::Deserialize)]
        struct SensitiveConfigParams {
            #[serde(default)]
            password: Option<uptrakit_internal_wire::SecretString>,
            #[serde(default)]
            ca_pem: Option<uptrakit_internal_wire::SecretString>,
        }

        let mut value = request.params.clone();
        let obj = value
            .as_object_mut()
            .ok_or_else(|| "extension params must be a JSON object".to_string())?;

        let sensitive = uptrakit_service_sdk::decrypt_sensitive_params::<SensitiveConfigParams>(
            request.sensitive_params.as_ref().map(|s| s.expose_secret()),
            self.private_key_der.as_deref(),
        )?;
        if let Some(sensitive) = sensitive {
            if let Some(password) = sensitive.password {
                obj.insert("password".to_string(), serde_json::json!(password));
            }
            if let Some(ca_pem) = sensitive.ca_pem {
                obj.insert("ca_pem".to_string(), serde_json::json!(ca_pem));
            }
        }

        if let Some(existing) = existing {
            if !obj.contains_key("password") && existing.password.is_some() {
                obj.insert("password".to_string(), serde_json::json!(existing.password));
            }
            if !obj.contains_key("ca_pem") && existing.ca_pem.is_some() {
                obj.insert("ca_pem".to_string(), serde_json::json!(existing.ca_pem));
            }
        }

        obj.remove("id");

        serde_json::from_value(value).map_err(|e| format!("invalid MQTT client configuration: {e}"))
    }

    /// Handle `ACTION_CREATE`: store a new MQTT client config.
    async fn handle_create_client(
        &mut self,
        request: uptrakit_internal_wire::extension::ExtensionRequestPayload,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        let request_id = request.request_id.clone();
        let Some(tenant_id) = request.tenant_id else {
            return extension::send_error_response(
                conn,
                request_id,
                "missing tenant scope for MQTT client config",
            )
            .await;
        };
        let new_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{new_id}");
        let config = match self.parse_request_config(&request, None) {
            Ok(config) => config,
            Err(message) => return extension::send_error_response(conn, request_id, message).await,
        };
        let config_value = match serde_json::to_value(&config) {
            Ok(value) => value,
            Err(e) => {
                return extension::send_error_response(
                    conn,
                    request_id,
                    format!("failed to serialize MQTT client config: {e}"),
                )
                .await;
            }
        };

        let pending = self
            .config_proxy
            .store(Some(tenant_id), key, config_value, true);
        let msg = pending.message.clone();
        if let Err(e) = conn.send(msg).await {
            return extension::send_error_response(
                conn,
                request_id,
                format!("failed to send store request: {e}"),
            )
            .await;
        }

        match pending
            .wait(&self.config_proxy, std::time::Duration::from_secs(10))
            .await
        {
            Ok(()) => {
                let response = uptrakit_internal_wire::extension::ExtensionResponsePayload {
                    request_id,
                    success: true,
                    data: serde_json::json!({ "id": new_id.to_string() }),
                    error: None,
                };
                conn.send(ServiceMessage::ExtensionResponse(response))
                    .await
                    .map_err(|e| {
                        report!(LoopError::Other(format!(
                            "failed to send extension response: {e}"
                        )))
                    })
            }
            Err(e) => extension::send_error_response(conn, request_id, e.to_string()).await,
        }
    }

    /// Handle `ACTION_EDIT`: update an existing MQTT client config.
    async fn handle_edit_client(
        &mut self,
        request: uptrakit_internal_wire::extension::ExtensionRequestPayload,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        let request_id = request.request_id.clone();
        let Some(tenant_id) = request.tenant_id else {
            return extension::send_error_response(
                conn,
                request_id,
                "missing tenant scope for MQTT client config",
            )
            .await;
        };

        let Some(id_str) = request.params.get("id").and_then(|v| v.as_str()) else {
            return extension::send_error_response(conn, request_id, "missing 'id' parameter")
                .await;
        };
        let Ok(mqtt_client_id) = Uuid::parse_str(id_str) else {
            return extension::send_error_response(conn, request_id, "invalid 'id' parameter")
                .await;
        };
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");
        let Some(existing) = self
            .configs
            .iter()
            .find(|config| config.mqtt_client_id == mqtt_client_id)
        else {
            return extension::send_error_response(conn, request_id, "MQTT client not found").await;
        };
        let config = match self.parse_request_config(&request, Some(existing)) {
            Ok(config) => config,
            Err(message) => return extension::send_error_response(conn, request_id, message).await,
        };
        let config_value = match serde_json::to_value(&config) {
            Ok(value) => value,
            Err(e) => {
                return extension::send_error_response(
                    conn,
                    request_id,
                    format!("failed to serialize MQTT client config: {e}"),
                )
                .await;
            }
        };

        let pending = self
            .config_proxy
            .store(Some(tenant_id), key, config_value, true);
        let msg = pending.message.clone();
        if let Err(e) = conn.send(msg).await {
            return extension::send_error_response(
                conn,
                request_id,
                format!("failed to send store request: {e}"),
            )
            .await;
        }

        match pending
            .wait(&self.config_proxy, std::time::Duration::from_secs(10))
            .await
        {
            Ok(()) => {
                let response = uptrakit_internal_wire::extension::ExtensionResponsePayload {
                    request_id,
                    success: true,
                    data: serde_json::Value::Null,
                    error: None,
                };
                conn.send(ServiceMessage::ExtensionResponse(response))
                    .await
                    .map_err(|e| {
                        report!(LoopError::Other(format!(
                            "failed to send extension response: {e}"
                        )))
                    })
            }
            Err(e) => extension::send_error_response(conn, request_id, e.to_string()).await,
        }
    }

    /// Handle `ACTION_DELETE`: delete an MQTT client config.
    async fn handle_delete_client(
        &mut self,
        request: uptrakit_internal_wire::extension::ExtensionRequestPayload,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        let request_id = request.request_id.clone();
        let Some(tenant_id) = request.tenant_id else {
            return extension::send_error_response(
                conn,
                request_id,
                "missing tenant scope for MQTT client config",
            )
            .await;
        };

        let Some(id_str) = request.params.get("id").and_then(|v| v.as_str()) else {
            return extension::send_error_response(conn, request_id, "missing 'id' parameter")
                .await;
        };
        let Ok(mqtt_client_id) = Uuid::parse_str(id_str) else {
            return extension::send_error_response(conn, request_id, "invalid 'id' parameter")
                .await;
        };
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");

        let pending = self.config_proxy.delete(Some(tenant_id), key);
        let msg = pending.message.clone();
        if let Err(e) = conn.send(msg).await {
            return extension::send_error_response(
                conn,
                request_id,
                format!("failed to send delete request: {e}"),
            )
            .await;
        }

        match pending
            .wait(&self.config_proxy, std::time::Duration::from_secs(10))
            .await
        {
            Ok(()) => {
                let response = uptrakit_internal_wire::extension::ExtensionResponsePayload {
                    request_id,
                    success: true,
                    data: serde_json::Value::Null,
                    error: None,
                };
                conn.send(ServiceMessage::ExtensionResponse(response))
                    .await
                    .map_err(|e| {
                        report!(LoopError::Other(format!(
                            "failed to send extension response: {e}"
                        )))
                    })
            }
            Err(e) => extension::send_error_response(conn, request_id, e.to_string()).await,
        }
    }
}

/// Parse a slice of `ServiceConfigEntry` values into `ParsedMqttClientConfig`.
///
/// Entries whose key does not start with `"clients."` or whose value cannot be
/// deserialized are logged and skipped.
fn parse_client_configs(entries: Vec<ServiceConfigEntry>) -> Vec<ParsedMqttClientConfig> {
    entries
        .into_iter()
        .filter_map(|entry| {
            let Some(mqtt_client_id) = parse_client_key(&entry.key) else {
                tracing::debug!(key = %entry.key, "skipping non-client config entry");
                return None;
            };
            match serde_json::from_value::<ParsedMqttClientConfig>(entry.value) {
                Ok(mut config) => {
                    config.mqtt_client_id = mqtt_client_id;
                    config.tenant_id = entry.tenant_id.unwrap_or_default();
                    Some(config)
                }
                Err(e) => {
                    tracing::warn!(
                        key = %entry.key,
                        error = %e,
                        "failed to deserialize MQTT client config, skipping"
                    );
                    None
                }
            }
        })
        .collect()
}

/// Parse the UUID suffix from a `"clients.{uuid}"` config key.
///
/// Returns `None` if the key does not start with `CONFIG_KEY_PREFIX` or if
/// the UUID suffix is malformed.
fn parse_client_key(key: &str) -> Option<Uuid> {
    let suffix = key.strip_prefix(CONFIG_KEY_PREFIX)?;
    Uuid::parse_str(suffix).ok()
}

/// Capabilities advertised by the MQTT service.
///
/// `SystemService` marks this service as global infrastructure (routed to the
/// `system_services` table instead of the per-tenant `services` table).
/// `UiExtensions` enables the MQTT clients settings page.
fn mqtt_capabilities() -> BTreeSet<Capability> {
    [
        Capability::SystemService,
        Capability::UpdateTracking,
        Capability::GracefulShutdown,
        Capability::UiExtensions,
        Capability::WorkloadClaims,
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

    tracing::info!("starting uptrakit-mqtt service");

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(MQTT_EVENT_CHANNEL_CAPACITY);
    let tenant_mgr = TenantManager::new(Some(event_tx));

    let mut handler = MqttHandler {
        tenant_mgr,
        event_rx,
        configs: Vec::new(),
        granted_keys: BTreeSet::new(),
        config_proxy: ServiceConfigProxy::new(),
        private_key_der: None,
        encryption_public_key: None,
    };

    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-mqtt",
        &args.common,
        &mut handler,
    )
    .await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_internal_wire::SecretString;

    fn test_handler() -> MqttHandler {
        let (_event_tx, event_rx) = tokio::sync::mpsc::channel(1);
        MqttHandler {
            tenant_mgr: TenantManager::new(None),
            event_rx,
            configs: Vec::new(),
            granted_keys: BTreeSet::new(),
            config_proxy: ServiceConfigProxy::new(),
            private_key_der: None,
            encryption_public_key: None,
        }
    }

    fn base_request(
        params: serde_json::Value,
    ) -> uptrakit_internal_wire::extension::ExtensionRequestPayload {
        uptrakit_internal_wire::extension::ExtensionRequestPayload {
            request_id: "req-1".to_string(),
            extension_id: extension::EXT_ID.to_string(),
            action_id: extension::ACTION_EDIT.to_string(),
            params,
            sensitive_params: None,
            tenant_id: Some(Uuid::now_v7()),
        }
    }

    fn existing_config() -> ParsedMqttClientConfig {
        ParsedMqttClientConfig {
            mqtt_client_id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            enabled: true,
            transport: crate::types::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "existing-client".to_string(),
            username: Some(SecretString::new("user")),
            password: Some(SecretString::new("existing-password")),
            ca_pem: Some(SecretString::new("existing-ca")),
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: true,
            ha_discovery_prefix: "homeassistant".to_string(),
        }
    }

    #[test]
    fn parse_request_config_preserves_existing_secret_fields() {
        let handler = test_handler();
        let existing = existing_config();
        let request = base_request(serde_json::json!({
            "id": existing.mqtt_client_id.to_string(),
            "enabled": true,
            "transport": "tls",
            "host": "new-broker.example.com",
            "port": 8883,
            "client_id": "updated-client",
            "username": "user",
            "topic_prefix": "uptrakit",
            "ha_discovery": true,
            "ha_discovery_prefix": "homeassistant"
        }));

        let parsed = handler
            .parse_request_config(&request, Some(&existing))
            .expect("parsed config");

        assert_eq!(parsed.host, "new-broker.example.com");
        assert_eq!(parsed.client_id, "updated-client");
        assert_eq!(
            parsed
                .password
                .as_ref()
                .map(|secret| secret.expose_secret()),
            Some("existing-password")
        );
        assert_eq!(
            parsed.ca_pem.as_ref().map(|secret| secret.expose_secret()),
            Some("existing-ca")
        );
    }

    #[test]
    fn parse_request_config_decrypts_sensitive_params() {
        let mut handler = test_handler();
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let public_key_b64 =
            base64::engine::general_purpose::STANDARD.encode(key_pair.public_key_raw());
        let sealed = uptrakit_crypto::ecies::sealed_box_encrypt_base64(
            r#"{"password":"new-password","ca_pem":"new-ca"}"#,
            &public_key_b64,
        )
        .expect("encrypt");
        handler.private_key_der = Some(key_pair.serialize_der());

        let request = uptrakit_internal_wire::extension::ExtensionRequestPayload {
            sensitive_params: Some(SecretString::new(sealed)),
            ..base_request(serde_json::json!({
                "enabled": true,
                "transport": "tcp",
                "host": "broker.example.com",
                "port": 1883,
                "client_id": "new-client",
                "topic_prefix": "uptrakit",
                "ha_discovery": false,
                "ha_discovery_prefix": "homeassistant"
            }))
        };

        let parsed = handler
            .parse_request_config(&request, None)
            .expect("parsed config");

        assert_eq!(
            parsed
                .password
                .as_ref()
                .map(|secret| secret.expose_secret()),
            Some("new-password")
        );
        assert_eq!(
            parsed.ca_pem.as_ref().map(|secret| secret.expose_secret()),
            Some("new-ca")
        );
    }
}
