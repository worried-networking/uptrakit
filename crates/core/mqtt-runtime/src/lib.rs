/// Abort the current publish batch on first error.
///
/// When a publish or subscribe operation fails (typically due to a broker
/// connection timeout), there is no point continuing with the remaining
/// operations in the batch — the data will be automatically republished
/// on the next `SoftwareStates` push or broker reconnect. Aborting early
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

mod client_manager;
mod extension;
pub mod ha_discovery;
mod mqtt_client;
mod state_publisher;
mod tenant_manager;
mod types;

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use uptrakit_internal_wire::{
    Capability, ControllerMessage, DisconnectReason, DisconnectingPayload, RegisterPayload,
    ServiceMessage, TransportError, extension::ExtensionRequestPayload,
    payloads::ServiceConfigEntry, payloads::ServiceConfigUpdatedPayload,
};
use uptrakit_service_sdk::ServiceConfigProxy;

use crate::client_manager::ParsedMqttClientConfig;
pub use crate::mqtt_client::MqttServiceEvent;
use crate::tenant_manager::TenantManager;
pub use crate::types::MqttClientConnectionStatus;

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

pub const MQTT_DIR_NAME: &str = "mqtt";
pub const MQTT_SERVICE_LABEL: &str = "uptrakit-mqtt service";
pub const MQTT_SERVICE_APP_NAME: &str = "uptrakit-mqtt";

#[derive(Debug, Clone, Default)]
pub struct MqttRuntimeIdentity {
    pub private_key_der: Option<Vec<u8>>,
    pub encryption_public_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MqttRuntimeSettings {
    pub ui_extensions_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttRuntimeLoopOutcome {
    Disconnected,
}

/// Shared MQTT service runtime used by the standalone and embedded adapters.
pub struct MqttRuntime {
    tenant_mgr: TenantManager,
    event_rx: tokio::sync::mpsc::Receiver<MqttServiceEvent>,
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
    /// Whether this runtime is currently yielded to an external MQTT service.
    yielded: bool,
}

impl Default for MqttRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttRuntime {
    pub fn new() -> Self {
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(MQTT_EVENT_CHANNEL_CAPACITY);
        let tenant_mgr = TenantManager::new(Some(event_tx));

        Self {
            tenant_mgr,
            event_rx,
            configs: Vec::new(),
            granted_keys: BTreeSet::new(),
            config_proxy: ServiceConfigProxy::new(),
            private_key_der: None,
            encryption_public_key: None,
            yielded: false,
        }
    }

    pub async fn on_connected(
        &mut self,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
        identity: MqttRuntimeIdentity,
    ) -> Result<(), TransportError> {
        self.private_key_der = identity.private_key_der;
        self.encryption_public_key = identity.encryption_public_key;

        transport
            .transport_send(ServiceMessage::Register(RegisterPayload::new(
                mqtt_capabilities(),
            )))
            .await
    }

    pub async fn apply_settings(
        &mut self,
        settings: MqttRuntimeSettings,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) {
        if !settings.ui_extensions_enabled {
            return;
        }

        let register_payload =
            extension::build_register_payload(self.encryption_public_key.clone());
        if let Err(error) = transport
            .transport_send(ServiceMessage::ExtensionRegister(register_payload))
            .await
        {
            tracing::warn!(error = %error, "failed to register UI extensions");
        }

        let actions_payload = uptrakit_internal_wire::extension::ExtensionActionsPayload::new(
            extension::build_actions(),
        );
        if let Err(error) = transport
            .transport_send(ServiceMessage::ExtensionActionsRegister(actions_payload))
            .await
        {
            tracing::warn!(error = %error, "failed to register extension actions");
        }
    }

    pub async fn handle_controller_message(
        &mut self,
        msg: ControllerMessage,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<Option<MqttRuntimeLoopOutcome>, TransportError> {
        match msg {
            ControllerMessage::ServiceConfigDelivery(payload) => {
                tracing::info!(
                    count = payload.entries.len(),
                    "received ServiceConfigDelivery"
                );
                let parsed = parse_client_configs(payload.entries);
                self.configs = parsed;
                self.send_workload_claim(transport).await;
                self.apply_granted_configs().await;
                Ok(None)
            }
            ControllerMessage::ServiceConfigUpdated(payload) => {
                tracing::debug!("received ServiceConfigUpdated");
                self.apply_config_update(payload).await;
                self.send_workload_claim(transport).await;
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
                for key in &payload.rejected {
                    if let Some(client_id) = parse_client_key(key) {
                        self.tenant_mgr.stop_client(&client_id).await;
                    }
                }
                if self.yielded {
                    if !self.granted_keys.is_empty() {
                        tracing::info!(
                            granted = self.granted_keys.len(),
                            "releasing workload claims granted while yielded"
                        );
                        self.granted_keys.clear();
                        self.send_workload_claim(transport).await;
                    }
                    self.tenant_mgr.shutdown_all().await;
                    return Ok(None);
                }
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
            ControllerMessage::ExtensionRequest(request) => {
                self.handle_extension_request(request, transport).await?;
                Ok(None)
            }
            ControllerMessage::ExtensionResponse(response) => {
                self.on_extension_response(response);
                Ok(None)
            }
            ControllerMessage::ServiceConfigAck(ack) => {
                self.on_service_config_ack(ack);
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    pub async fn poll_event(&mut self) -> Option<MqttServiceEvent> {
        self.event_rx.recv().await
    }

    pub async fn handle_event(
        &mut self,
        event: Option<MqttServiceEvent>,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Option<MqttRuntimeLoopOutcome> {
        match event {
            Some(MqttServiceEvent::Status(status)) => {
                tracing::debug!(
                    mqtt_client_id = %status.mqtt_client_id,
                    status = %status.status,
                    "MQTT client connection status changed"
                );
                None
            }
            Some(MqttServiceEvent::Reconnected(id)) => {
                self.tenant_mgr.handle_reconnected(&id).await;
                None
            }
            Some(MqttServiceEvent::HaOnline(id)) => {
                self.tenant_mgr.handle_ha_online(&id).await;
                None
            }
            Some(MqttServiceEvent::Command {
                mqtt_client_id,
                topic,
            }) => {
                if let Some(payload) = self
                    .tenant_mgr
                    .resolve_host_security_batch_update_trigger(mqtt_client_id, &topic)
                {
                    transport
                        .transport_send_best_effort(ServiceMessage::ServiceTriggerHostBatchUpdate(
                            payload,
                        ))
                        .await;
                } else if let Some(payload) = self
                    .tenant_mgr
                    .resolve_host_batch_update_trigger(mqtt_client_id, &topic)
                {
                    transport
                        .transport_send_best_effort(ServiceMessage::ServiceTriggerHostBatchUpdate(
                            payload,
                        ))
                        .await;
                } else if let Some(payload) = self
                    .tenant_mgr
                    .resolve_update_trigger(mqtt_client_id, &topic)
                {
                    transport
                        .transport_send_best_effort(ServiceMessage::ServiceTriggerUpdate(payload))
                        .await;
                } else {
                    tracing::debug!(
                        %mqtt_client_id,
                        %topic,
                        "received MQTT command on unknown topic, ignoring"
                    );
                }
                None
            }
            None => {
                tracing::warn!("event channel closed");
                Some(MqttRuntimeLoopOutcome::Disconnected)
            }
        }
    }

    pub async fn handle_yield_change(
        &mut self,
        yielded: bool,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) {
        if self.yielded == yielded {
            return;
        }

        self.yielded = yielded;
        if yielded {
            tracing::info!("MQTT runtime yielded to external service");
            // Yielding uses the workload-claim full replacement protocol to
            // release every previously granted config key.
            self.granted_keys.clear();
            self.send_workload_claim(transport).await;
            self.tenant_mgr.shutdown_all().await;
        } else {
            tracing::info!("MQTT runtime resumed after external service disconnected");
            self.send_workload_claim(transport).await;
            self.apply_granted_configs().await;
        }
    }

    pub fn on_extension_response(
        &mut self,
        response: uptrakit_internal_wire::extension::ExtensionResponsePayload,
    ) {
        let _ = response;
    }

    pub fn on_service_config_ack(
        &self,
        ack: uptrakit_internal_wire::payloads::ServiceConfigAckPayload,
    ) {
        self.config_proxy.complete(&ack.request_id.clone(), ack);
    }

    pub async fn shutdown(
        &mut self,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
        reason: DisconnectReason,
    ) {
        transport
            .transport_send_best_effort(ServiceMessage::Disconnecting(DisconnectingPayload::new(
                reason,
            )))
            .await;

        tracing::info!("shutting down MQTT clients");
        self.tenant_mgr.shutdown_all().await;
        tracing::info!("shutdown complete");
    }

    async fn handle_extension_request(
        &mut self,
        request: ExtensionRequestPayload,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id.clone();

        if let Some(response) = extension::handle_list_action(&request, &self.configs) {
            return transport
                .transport_send(ServiceMessage::ExtensionResponse(response))
                .await;
        }
        if request.action_id == extension::ACTION_GET {
            if let Some(response) = extension::handle_get_action(&request, &self.configs) {
                return transport
                    .transport_send(ServiceMessage::ExtensionResponse(response))
                    .await;
            }
            return extension::send_error_response(
                transport,
                request_id,
                "missing or invalid MQTT client id",
            )
            .await;
        }

        match request.action_id.as_str() {
            extension::ACTION_CREATE => {
                self.handle_create_client(request, transport).await?;
            }
            extension::ACTION_EDIT => {
                self.handle_edit_client(request, transport).await?;
            }
            extension::ACTION_DELETE => {
                self.handle_delete_client(request, transport).await?;
            }
            _ => {
                extension::send_error_response(
                    transport,
                    request_id,
                    format!("unknown action: {}", request.action_id),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Apply an incremental config update from the controller.
    async fn apply_config_update(&mut self, payload: ServiceConfigUpdatedPayload) {
        let changed = parse_client_configs(payload.changed);

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

        for mqtt_client_id in deleted_ids {
            self.tenant_mgr.stop_client(&mqtt_client_id).await;
            self.granted_keys
                .remove(&format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}"));
        }
    }

    /// Compute the desired config claims from the current in-memory configs.
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

    async fn send_workload_claim(
        &self,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) {
        let claims = if self.yielded {
            BTreeMap::new()
        } else {
            self.compute_desired_claims()
        };
        tracing::info!(keys = claims.len(), "sending WorkloadClaim");
        if let Err(error) = transport
            .transport_send(ServiceMessage::WorkloadClaim(
                uptrakit_internal_wire::WorkloadClaimPayload::new(claims),
            ))
            .await
        {
            tracing::warn!(error = %error, "failed to send WorkloadClaim");
        }
    }

    async fn apply_granted_configs(&mut self) {
        if self.yielded {
            self.tenant_mgr.shutdown_all().await;
            return;
        }

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
        request: &ExtensionRequestPayload,
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

    async fn handle_create_client(
        &mut self,
        request: ExtensionRequestPayload,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id.clone();
        let Some(tenant_id) = request.tenant_id else {
            return extension::send_error_response(
                transport,
                request_id,
                "missing tenant scope for MQTT client config",
            )
            .await;
        };

        let new_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{new_id}");
        let config = match self.parse_request_config(&request, None) {
            Ok(config) => config,
            Err(message) => {
                return extension::send_error_response(transport, request_id, message).await;
            }
        };
        let config_value = match serde_json::to_value(&config) {
            Ok(value) => value,
            Err(error) => {
                return extension::send_error_response(
                    transport,
                    request_id,
                    format!("failed to serialize MQTT client config: {error}"),
                )
                .await;
            }
        };

        let pending = self
            .config_proxy
            .store(Some(tenant_id), key, config_value, true);
        let msg = pending.message.clone();
        if let Err(error) = transport.transport_send(msg).await {
            return extension::send_error_response(
                transport,
                request_id,
                format!("failed to send store request: {error}"),
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
                transport
                    .transport_send(ServiceMessage::ExtensionResponse(response))
                    .await
            }
            Err(error) => {
                extension::send_error_response(transport, request_id, error.to_string()).await
            }
        }
    }

    async fn handle_edit_client(
        &mut self,
        request: ExtensionRequestPayload,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id.clone();
        let Some(tenant_id) = request.tenant_id else {
            return extension::send_error_response(
                transport,
                request_id,
                "missing tenant scope for MQTT client config",
            )
            .await;
        };

        let Some(id_str) = request.params.get("id").and_then(|v| v.as_str()) else {
            return extension::send_error_response(transport, request_id, "missing 'id' parameter")
                .await;
        };
        let Ok(mqtt_client_id) = Uuid::parse_str(id_str) else {
            return extension::send_error_response(transport, request_id, "invalid 'id' parameter")
                .await;
        };
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");
        let Some(existing) = self
            .configs
            .iter()
            .find(|config| config.mqtt_client_id == mqtt_client_id)
        else {
            return extension::send_error_response(transport, request_id, "MQTT client not found")
                .await;
        };
        let config = match self.parse_request_config(&request, Some(existing)) {
            Ok(config) => config,
            Err(message) => {
                return extension::send_error_response(transport, request_id, message).await;
            }
        };
        let config_value = match serde_json::to_value(&config) {
            Ok(value) => value,
            Err(error) => {
                return extension::send_error_response(
                    transport,
                    request_id,
                    format!("failed to serialize MQTT client config: {error}"),
                )
                .await;
            }
        };

        let pending = self
            .config_proxy
            .store(Some(tenant_id), key, config_value, true);
        let msg = pending.message.clone();
        if let Err(error) = transport.transport_send(msg).await {
            return extension::send_error_response(
                transport,
                request_id,
                format!("failed to send store request: {error}"),
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
                transport
                    .transport_send(ServiceMessage::ExtensionResponse(response))
                    .await
            }
            Err(error) => {
                extension::send_error_response(transport, request_id, error.to_string()).await
            }
        }
    }

    async fn handle_delete_client(
        &mut self,
        request: ExtensionRequestPayload,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id.clone();
        let Some(tenant_id) = request.tenant_id else {
            return extension::send_error_response(
                transport,
                request_id,
                "missing tenant scope for MQTT client config",
            )
            .await;
        };

        let Some(id_str) = request.params.get("id").and_then(|v| v.as_str()) else {
            return extension::send_error_response(transport, request_id, "missing 'id' parameter")
                .await;
        };
        let Ok(mqtt_client_id) = Uuid::parse_str(id_str) else {
            return extension::send_error_response(transport, request_id, "invalid 'id' parameter")
                .await;
        };
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");

        let pending = self.config_proxy.delete(Some(tenant_id), key);
        let msg = pending.message.clone();
        if let Err(error) = transport.transport_send(msg).await {
            return extension::send_error_response(
                transport,
                request_id,
                format!("failed to send delete request: {error}"),
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
                transport
                    .transport_send(ServiceMessage::ExtensionResponse(response))
                    .await
            }
            Err(error) => {
                extension::send_error_response(transport, request_id, error.to_string()).await
            }
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
                Err(error) => {
                    tracing::warn!(
                        key = %entry.key,
                        error = %error,
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
pub fn mqtt_capabilities() -> BTreeSet<Capability> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use uptrakit_internal_wire::SecretString;
    use uptrakit_service_sdk::test_support::MockTransport;

    fn base_request(params: serde_json::Value) -> ExtensionRequestPayload {
        ExtensionRequestPayload {
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

    fn config_entry(tenant_id: Uuid, mqtt_client_id: Uuid) -> ServiceConfigEntry {
        ServiceConfigEntry::new(
            Some(tenant_id),
            format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}"),
            serde_json::json!({
                "enabled": true,
                "transport": "tcp",
                "host": "broker.example.com",
                "port": 1883,
                "client_id": "client-1",
                "topic_prefix": "uptrakit",
                "ha_discovery": false,
                "ha_discovery_prefix": "homeassistant"
            }),
        )
    }

    #[test]
    fn parse_request_config_preserves_existing_secret_fields() {
        let runtime = MqttRuntime::new();
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

        let parsed = runtime
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
        let mut runtime = MqttRuntime::new();
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let public_key_b64 =
            base64::engine::general_purpose::STANDARD.encode(key_pair.public_key_raw());
        let sealed = uptrakit_crypto::ecies::sealed_box_encrypt_base64(
            r#"{"password":"new-password","ca_pem":"new-ca"}"#,
            &public_key_b64,
        )
        .expect("encrypt");
        runtime.private_key_der = Some(key_pair.serialize_der());

        let request = ExtensionRequestPayload {
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

        let parsed = runtime
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

    #[tokio::test]
    async fn service_config_delivery_claims_without_starting_before_grant() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();

        runtime
            .handle_controller_message(
                ControllerMessage::ServiceConfigDelivery(
                    uptrakit_internal_wire::payloads::ServiceConfigDeliveryPayload::new(vec![
                        config_entry(tenant_id, mqtt_client_id),
                    ]),
                ),
                &mut transport,
            )
            .await
            .expect("service config delivery should succeed");

        assert_eq!(runtime.configs.len(), 1);
        assert!(runtime.granted_keys.is_empty());
        assert!(runtime.tenant_mgr.clients.is_empty());

        let Some(ServiceMessage::WorkloadClaim(payload)) = transport.send_log().last() else {
            panic!("expected WorkloadClaim to be sent");
        };
        assert_eq!(
            payload
                .claims
                .get(&format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}")),
            Some(&tenant_id)
        );
    }

    #[tokio::test]
    async fn yielding_clears_grants_and_sends_empty_claims() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");

        runtime.configs = vec![
            parse_client_configs(vec![config_entry(tenant_id, mqtt_client_id)])
                .pop()
                .expect("parsed config"),
        ];
        runtime.granted_keys.insert(key.clone());

        runtime.handle_yield_change(true, &mut transport).await;

        assert!(runtime.granted_keys.is_empty());
        let Some(ServiceMessage::WorkloadClaim(payload)) = transport.send_log().last() else {
            panic!("expected WorkloadClaim to be sent");
        };
        assert!(payload.claims.is_empty());
        assert!(runtime.tenant_mgr.clients.is_empty());
    }

    #[tokio::test]
    async fn claim_results_received_while_yielded_are_immediately_released() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");

        runtime.configs = vec![
            parse_client_configs(vec![config_entry(tenant_id, mqtt_client_id)])
                .pop()
                .expect("parsed config"),
        ];
        runtime.handle_yield_change(true, &mut transport).await;

        runtime
            .handle_controller_message(
                ControllerMessage::WorkloadClaimResult(
                    uptrakit_internal_wire::WorkloadClaimResultPayload::new(
                        [key.clone()].into_iter().collect(),
                        BTreeSet::new(),
                    ),
                ),
                &mut transport,
            )
            .await
            .expect("grant handling should succeed");

        assert!(runtime.granted_keys.is_empty());
        assert!(runtime.tenant_mgr.clients.is_empty());
        let Some(ServiceMessage::WorkloadClaim(payload)) = transport.send_log().last() else {
            panic!("expected WorkloadClaim to be sent");
        };
        assert!(payload.claims.is_empty());
    }

    #[tokio::test]
    async fn resuming_reclaims_desired_configs() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");

        runtime.configs = vec![
            parse_client_configs(vec![config_entry(tenant_id, mqtt_client_id)])
                .pop()
                .expect("parsed config"),
        ];

        runtime.handle_yield_change(true, &mut transport).await;
        runtime.handle_yield_change(false, &mut transport).await;

        let Some(ServiceMessage::WorkloadClaim(payload)) = transport.send_log().last() else {
            panic!("expected WorkloadClaim to be sent");
        };
        assert_eq!(payload.claims.get(&key), Some(&tenant_id));
    }

    #[tokio::test]
    async fn apply_settings_registers_extensions_when_enabled() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();

        runtime
            .on_connected(
                &mut transport,
                MqttRuntimeIdentity {
                    private_key_der: None,
                    encryption_public_key: Some("public-key".to_string()),
                },
            )
            .await
            .expect("connect should succeed");
        runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_extensions_enabled: true,
                },
                &mut transport,
            )
            .await;

        assert!(matches!(
            transport.send_log().get(0),
            Some(ServiceMessage::Register(_))
        ));
        assert!(matches!(
            transport.send_log().get(1),
            Some(ServiceMessage::ExtensionRegister(_))
        ));
        assert!(matches!(
            transport.send_log().get(2),
            Some(ServiceMessage::ExtensionActionsRegister(_))
        ));
    }
}
