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
pub mod ha_discovery;
mod mqtt_client;
mod state_publisher;
mod surface_runtime;
mod tenant_manager;
mod types;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use uptrakit_internal_wire::{
    Capability, ControllerMessage, DisconnectReason, DisconnectingPayload, RegisterPayload,
    ServiceMessage, TransportError,
    payloads::ServiceConfigEntry,
    payloads::ServiceConfigUpdatedPayload,
    surfaces::{SurfaceActionErrorCode, SurfaceActionRequest, SurfaceActionResponse},
};
use uptrakit_service_sdk::{PendingServiceConfigRequest, ServiceConfigProxy};

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
    pub service_id: Option<Uuid>,
    pub private_key_der: Option<Vec<u8>>,
    pub encryption_public_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MqttRuntimeSettings {
    pub ui_surfaces_enabled: bool,
    pub tenant_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttRuntimeLoopOutcome {
    Disconnected,
}

/// Shared MQTT service runtime used by the standalone and embedded adapters.
pub struct MqttRuntime {
    event_tx: tokio::sync::mpsc::Sender<MqttServiceEvent>,
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
    config_proxy: Arc<ServiceConfigProxy>,
    /// Private key used to decrypt ECIES-encrypted sensitive extension params.
    private_key_der: Option<Vec<u8>>,
    /// Base64-encoded uncompressed P-256 public key for extension param encryption.
    encryption_public_key: Option<String>,
    /// Stable controller-assigned service id for provider identity composition.
    service_id: Option<Uuid>,
    /// Effective service tenant scope from settings negotiation.
    service_tenant_id: Option<Uuid>,
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
        let tenant_mgr = TenantManager::new(Some(event_tx.clone()));

        Self {
            event_tx,
            tenant_mgr,
            event_rx,
            configs: Vec::new(),
            granted_keys: BTreeSet::new(),
            config_proxy: Arc::new(ServiceConfigProxy::new()),
            private_key_der: None,
            encryption_public_key: None,
            service_id: None,
            service_tenant_id: None,
            yielded: false,
        }
    }

    pub async fn on_connected(
        &mut self,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
        identity: MqttRuntimeIdentity,
    ) -> Result<(), TransportError> {
        self.service_id = identity.service_id;
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
        self.service_tenant_id = settings.tenant_id;
        if !settings.ui_surfaces_enabled {
            return;
        }

        let Some(register_payload) = surface_runtime::build_surface_registration_with_ids(
            self.encryption_public_key.clone(),
            self.service_id,
            self.service_tenant_id,
        ) else {
            tracing::info!(
                "skipping MQTT settings surface registration: tenant binding unavailable"
            );
            return;
        };
        transport
            .transport_send_best_effort(ServiceMessage::SurfaceRegistration(register_payload))
            .await;
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
            ControllerMessage::SurfaceActionRequest(request) => {
                self.handle_surface_action_request(request, transport)
                    .await?;
                Ok(None)
            }
            ControllerMessage::SurfaceActionResponse(response) => {
                self.on_surface_action_response(response);
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
            Some(MqttServiceEvent::SurfaceConfigRequestCompleted {
                request_id,
                local_update,
                result,
                error,
            }) => {
                if let Some(local_update) = local_update {
                    self.reconcile_after_local_config_update(local_update, transport)
                        .await;
                }

                let send_result = if let Some(error) = error {
                    surface_runtime::send_error_response(
                        transport,
                        request_id,
                        SurfaceActionErrorCode::InternalError,
                        error,
                    )
                    .await
                } else {
                    transport
                        .transport_send(ServiceMessage::SurfaceActionResponse(
                            SurfaceActionResponse {
                                request_id,
                                success: true,
                                result,
                                error: None,
                            },
                        ))
                        .await
                };

                if let Err(error) = send_result {
                    tracing::warn!(
                        error = %error,
                        request_id = %request_id,
                        "failed to send MQTT surface action response"
                    );
                    return Some(MqttRuntimeLoopOutcome::Disconnected);
                }
                None
            }
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

    pub fn on_surface_action_response(&mut self, response: SurfaceActionResponse) {
        let _ = response;
    }

    pub fn on_service_config_ack(
        &self,
        ack: uptrakit_internal_wire::payloads::ServiceConfigAckPayload,
    ) {
        self.config_proxy.complete(&ack.request_id.clone(), ack);
    }

    fn spawn_surface_config_completion(
        &self,
        pending: PendingServiceConfigRequest,
        request_id: Uuid,
        local_update: Option<ServiceConfigUpdatedPayload>,
        result: Option<serde_json::Value>,
    ) {
        let proxy = Arc::clone(&self.config_proxy);
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let event = match pending.wait(proxy.as_ref(), Duration::from_secs(10)).await {
                Ok(()) => MqttServiceEvent::SurfaceConfigRequestCompleted {
                    request_id,
                    local_update,
                    result,
                    error: None,
                },
                Err(error) => MqttServiceEvent::SurfaceConfigRequestCompleted {
                    request_id,
                    local_update: None,
                    result: None,
                    error: Some(error.to_string()),
                },
            };

            if let Err(error) = event_tx.send(event).await {
                tracing::warn!(
                    error = %error,
                    request_id = %request_id,
                    "failed to queue MQTT surface config completion event"
                );
            }
        });
    }

    fn expected_provider_id(&self) -> String {
        self.service_id
            .map(|id| format!("service.uptrakit-mqtt.{id}"))
            .unwrap_or_else(|| "service.uptrakit-mqtt".to_string())
    }

    fn validate_surface_request_context(
        &self,
        request: &SurfaceActionRequest,
    ) -> Result<Uuid, (SurfaceActionErrorCode, String)> {
        let Some(bound_tenant_id) = self.service_tenant_id else {
            return Err((
                SurfaceActionErrorCode::PermissionDenied,
                "MQTT surface is unavailable without tenant binding".to_string(),
            ));
        };

        let expected_provider_id = self.expected_provider_id();
        let Some(target_provider_id) = request.target_provider_id.as_deref() else {
            return Err((
                SurfaceActionErrorCode::PermissionDenied,
                "missing target provider id".to_string(),
            ));
        };
        if target_provider_id != expected_provider_id {
            return Err((
                SurfaceActionErrorCode::PermissionDenied,
                format!(
                    "surface request target provider mismatch (expected {expected_provider_id})"
                ),
            ));
        }

        let Some(request_tenant_id) = parse_request_tenant(request) else {
            return Err((
                SurfaceActionErrorCode::InvalidRequest,
                "missing tenant scope for MQTT client config".to_string(),
            ));
        };
        if request_tenant_id != bound_tenant_id {
            return Err((
                SurfaceActionErrorCode::PermissionDenied,
                "tenant scope does not match MQTT runtime binding".to_string(),
            ));
        }

        Ok(request_tenant_id)
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

    async fn handle_surface_action_request(
        &mut self,
        request: SurfaceActionRequest,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id;
        let request_tenant_id = match self.validate_surface_request_context(&request) {
            Ok(tenant_id) => tenant_id,
            Err((code, message)) => {
                return surface_runtime::send_error_response(transport, request_id, code, message)
                    .await;
            }
        };

        if let Some(response) =
            surface_runtime::handle_list_action(&request, request_tenant_id, &self.configs)
        {
            return transport
                .transport_send(ServiceMessage::SurfaceActionResponse(response))
                .await;
        }
        if request.interaction_id.as_str() == surface_runtime::ACTION_GET {
            if let Some(response) =
                surface_runtime::handle_get_action(&request, request_tenant_id, &self.configs)
            {
                return transport
                    .transport_send(ServiceMessage::SurfaceActionResponse(response))
                    .await;
            }
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InvalidRequest,
                "missing or invalid MQTT client id",
            )
            .await;
        }

        match request.interaction_id.as_str() {
            surface_runtime::ACTION_CREATE => {
                self.handle_create_client(request, request_tenant_id, transport)
                    .await?;
            }
            surface_runtime::ACTION_EDIT => {
                self.handle_edit_client(request, request_tenant_id, transport)
                    .await?;
            }
            surface_runtime::ACTION_DELETE => {
                self.handle_delete_client(request, request_tenant_id, transport)
                    .await?;
            }
            _ => {
                surface_runtime::send_error_response(
                    transport,
                    request_id,
                    SurfaceActionErrorCode::UnsupportedCapability,
                    format!("unknown action: {}", request.interaction_id),
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

    async fn reconcile_after_local_config_update(
        &mut self,
        payload: ServiceConfigUpdatedPayload,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) {
        self.apply_config_update(payload).await;
        self.send_workload_claim(transport).await;
        self.apply_granted_configs().await;
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
        request: &SurfaceActionRequest,
        existing: Option<&ParsedMqttClientConfig>,
    ) -> Result<ParsedMqttClientConfig, String> {
        #[derive(serde::Deserialize)]
        struct SensitiveConfigParams {
            #[serde(default)]
            password: Option<uptrakit_internal_wire::SecretString>,
            #[serde(default)]
            ca_pem: Option<uptrakit_internal_wire::SecretString>,
        }

        let mut value = serde_json::Value::Object(request.params.clone());
        let obj = value
            .as_object_mut()
            .ok_or_else(|| "surface params must be a JSON object".to_string())?;

        let sensitive = uptrakit_service_sdk::decrypt_sensitive_params::<SensitiveConfigParams>(
            request
                .encrypted_sensitive_params
                .as_ref()
                .map(|payload| payload.ciphertext_b64.as_str()),
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
        request: SurfaceActionRequest,
        tenant_id: Uuid,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id;

        let new_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{new_id}");
        let config = match self.parse_request_config(&request, None) {
            Ok(config) => config,
            Err(message) => {
                return surface_runtime::send_error_response(
                    transport,
                    request_id,
                    SurfaceActionErrorCode::InvalidRequest,
                    message,
                )
                .await;
            }
        };
        let config_value = match serde_json::to_value(&config) {
            Ok(value) => value,
            Err(error) => {
                return surface_runtime::send_error_response(
                    transport,
                    request_id,
                    SurfaceActionErrorCode::InternalError,
                    format!("failed to serialize MQTT client config: {error}"),
                )
                .await;
            }
        };
        let key_for_local = key.clone();
        let config_value_for_local = config_value.clone();

        let pending = self
            .config_proxy
            .store(Some(tenant_id), key, config_value, true);
        let msg = pending.message.clone();
        if let Err(error) = transport.transport_send(msg).await {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InternalError,
                format!("failed to send store request: {error}"),
            )
            .await;
        }

        self.spawn_surface_config_completion(
            pending,
            request_id,
            Some(ServiceConfigUpdatedPayload::new(
                vec![ServiceConfigEntry::new(
                    Some(tenant_id),
                    key_for_local,
                    config_value_for_local,
                )],
                Vec::new(),
            )),
            Some(serde_json::json!({ "id": new_id.to_string() })),
        );
        Ok(())
    }

    async fn handle_edit_client(
        &mut self,
        request: SurfaceActionRequest,
        tenant_id: Uuid,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id;

        let Some(id_str) = request.params.get("id").and_then(|v| v.as_str()) else {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InvalidRequest,
                "missing 'id' parameter",
            )
            .await;
        };
        let Ok(mqtt_client_id) = Uuid::parse_str(id_str) else {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InvalidRequest,
                "invalid 'id' parameter",
            )
            .await;
        };
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");
        let Some(existing) = self.configs.iter().find(|config| {
            config.mqtt_client_id == mqtt_client_id && config.tenant_id == tenant_id
        }) else {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InvalidRequest,
                "MQTT client not found",
            )
            .await;
        };
        let config = match self.parse_request_config(&request, Some(existing)) {
            Ok(config) => config,
            Err(message) => {
                return surface_runtime::send_error_response(
                    transport,
                    request_id,
                    SurfaceActionErrorCode::InvalidRequest,
                    message,
                )
                .await;
            }
        };
        let config_value = match serde_json::to_value(&config) {
            Ok(value) => value,
            Err(error) => {
                return surface_runtime::send_error_response(
                    transport,
                    request_id,
                    SurfaceActionErrorCode::InternalError,
                    format!("failed to serialize MQTT client config: {error}"),
                )
                .await;
            }
        };
        let key_for_local = key.clone();
        let config_value_for_local = config_value.clone();

        let pending = self
            .config_proxy
            .store(Some(tenant_id), key, config_value, true);
        let msg = pending.message.clone();
        if let Err(error) = transport.transport_send(msg).await {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InternalError,
                format!("failed to send store request: {error}"),
            )
            .await;
        }

        self.spawn_surface_config_completion(
            pending,
            request_id,
            Some(ServiceConfigUpdatedPayload::new(
                vec![ServiceConfigEntry::new(
                    Some(tenant_id),
                    key_for_local,
                    config_value_for_local,
                )],
                Vec::new(),
            )),
            None,
        );
        Ok(())
    }

    async fn handle_delete_client(
        &mut self,
        request: SurfaceActionRequest,
        tenant_id: Uuid,
        transport: &mut dyn uptrakit_internal_wire::ServiceTransport,
    ) -> Result<(), TransportError> {
        let request_id = request.request_id;

        let Some(id_str) = request.params.get("id").and_then(|v| v.as_str()) else {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InvalidRequest,
                "missing 'id' parameter",
            )
            .await;
        };
        let Ok(mqtt_client_id) = Uuid::parse_str(id_str) else {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InvalidRequest,
                "invalid 'id' parameter",
            )
            .await;
        };
        if !self
            .configs
            .iter()
            .any(|config| config.mqtt_client_id == mqtt_client_id && config.tenant_id == tenant_id)
        {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InvalidRequest,
                "MQTT client not found",
            )
            .await;
        }
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");
        let key_for_local = key.clone();

        let pending = self.config_proxy.delete(Some(tenant_id), key);
        let msg = pending.message.clone();
        if let Err(error) = transport.transport_send(msg).await {
            return surface_runtime::send_error_response(
                transport,
                request_id,
                SurfaceActionErrorCode::InternalError,
                format!("failed to send delete request: {error}"),
            )
            .await;
        }

        self.spawn_surface_config_completion(
            pending,
            request_id,
            Some(ServiceConfigUpdatedPayload::new(
                Vec::new(),
                vec![uptrakit_internal_wire::payloads::ServiceConfigKey::new(
                    Some(tenant_id),
                    key_for_local,
                )],
            )),
            None,
        );
        Ok(())
    }
}

fn parse_request_tenant(request: &SurfaceActionRequest) -> Option<Uuid> {
    Uuid::parse_str(&request.tenant_id).ok()
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
/// `UiSurfaces` enables the MQTT clients settings page.
pub fn mqtt_capabilities() -> BTreeSet<Capability> {
    [
        Capability::SystemService,
        Capability::UpdateTracking,
        Capability::GracefulShutdown,
        Capability::UiSurfaces,
        Capability::WorkloadClaims,
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use uptrakit_internal_wire::{SecretString, surfaces::CallerOrigin};
    use uptrakit_service_sdk::test_support::MockTransport;

    fn base_request(params: serde_json::Value) -> SurfaceActionRequest {
        let params_obj = params
            .as_object()
            .cloned()
            .expect("params must be object in tests");
        SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7().to_string(),
            surface_id: uptrakit_internal_wire::surfaces::SurfaceId::new(surface_runtime::EXT_ID)
                .expect("surface id"),
            interaction_id: uptrakit_internal_wire::surfaces::InteractionId::new(
                surface_runtime::ACTION_EDIT,
            )
            .expect("interaction id"),
            idempotency_key: "req-1".to_string(),
            target_provider_id: Some("service.uptrakit-mqtt".to_string()),
            caller_origin: CallerOrigin::BuiltInSystem {
                principal: "tests".to_string(),
            },
            params: params_obj,
            encrypted_sensitive_params: None,
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

    fn config_entry_with_host(
        tenant_id: Uuid,
        mqtt_client_id: Uuid,
        host: &str,
    ) -> ServiceConfigEntry {
        ServiceConfigEntry::new(
            Some(tenant_id),
            format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}"),
            serde_json::json!({
                "enabled": true,
                "transport": "tcp",
                "host": host,
                "port": 1883,
                "client_id": "client-1",
                "topic_prefix": "uptrakit",
                "ha_discovery": false,
                "ha_discovery_prefix": "homeassistant"
            }),
        )
    }

    fn action_request(
        interaction_id: &str,
        tenant_id: Uuid,
        target_provider_id: Option<&str>,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> SurfaceActionRequest {
        SurfaceActionRequest {
            request_id: Uuid::now_v7(),
            tenant_id: tenant_id.to_string(),
            surface_id: uptrakit_internal_wire::surfaces::SurfaceId::new(surface_runtime::EXT_ID)
                .expect("surface id"),
            interaction_id: uptrakit_internal_wire::surfaces::InteractionId::new(interaction_id)
                .expect("interaction id"),
            idempotency_key: "req-1".to_string(),
            target_provider_id: target_provider_id.map(ToString::to_string),
            caller_origin: CallerOrigin::BuiltInSystem {
                principal: "tests".to_string(),
            },
            params,
            encrypted_sensitive_params: None,
        }
    }

    fn expect_last_surface_error(
        transport: &MockTransport,
    ) -> uptrakit_internal_wire::surfaces::SurfaceActionError {
        let Some(ServiceMessage::SurfaceActionResponse(response)) = transport.send_log().last()
        else {
            panic!("expected SurfaceActionResponse");
        };
        assert!(!response.success, "expected error response");
        response.error.clone().expect("surface error")
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

        let request = SurfaceActionRequest {
            encrypted_sensitive_params: Some(
                uptrakit_internal_wire::surfaces::EncryptedSensitiveParams {
                    key_id: "mqtt".to_string(),
                    algorithm:
                        uptrakit_internal_wire::surfaces::ProviderEncryptionAlgorithm::EciesP256,
                    ciphertext_b64: sealed,
                },
            ),
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
    async fn surface_action_rejects_requests_without_runtime_tenant_binding() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let request = action_request(
            surface_runtime::ACTION_LIST,
            Uuid::now_v7(),
            Some("service.uptrakit-mqtt"),
            serde_json::Map::new(),
        );

        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(request),
                &mut transport,
            )
            .await
            .expect("request handling");

        let error = expect_last_surface_error(&transport);
        assert_eq!(error.code, SurfaceActionErrorCode::PermissionDenied);
        assert!(
            error.message.contains("tenant binding"),
            "unexpected message: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn surface_action_enforces_provider_target_and_tenant_match() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let bound_tenant = Uuid::now_v7();
        runtime.service_tenant_id = Some(bound_tenant);

        let wrong_target = action_request(
            surface_runtime::ACTION_LIST,
            bound_tenant,
            Some("service.other"),
            serde_json::Map::new(),
        );
        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(wrong_target),
                &mut transport,
            )
            .await
            .expect("wrong target request handling");

        let provider_error = expect_last_surface_error(&transport);
        assert_eq!(
            provider_error.code,
            SurfaceActionErrorCode::PermissionDenied
        );
        assert!(
            provider_error.message.contains("target provider"),
            "unexpected message: {}",
            provider_error.message
        );

        let mismatched_tenant = action_request(
            surface_runtime::ACTION_LIST,
            Uuid::now_v7(),
            Some("service.uptrakit-mqtt"),
            serde_json::Map::new(),
        );
        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(mismatched_tenant),
                &mut transport,
            )
            .await
            .expect("mismatched tenant request handling");

        let tenant_error = expect_last_surface_error(&transport);
        assert_eq!(tenant_error.code, SurfaceActionErrorCode::PermissionDenied);
        assert!(
            tenant_error.message.contains("tenant scope"),
            "unexpected message: {}",
            tenant_error.message
        );
    }

    #[tokio::test]
    async fn edit_action_rejects_cross_tenant_lookup_by_id() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let request_tenant_id = Uuid::now_v7();
        let config_tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();
        runtime.service_tenant_id = Some(request_tenant_id);
        runtime.configs =
            parse_client_configs(vec![config_entry(config_tenant_id, mqtt_client_id)]);

        let request = action_request(
            surface_runtime::ACTION_EDIT,
            request_tenant_id,
            Some("service.uptrakit-mqtt"),
            serde_json::Map::from_iter([(
                "id".to_string(),
                serde_json::json!(mqtt_client_id.to_string()),
            )]),
        );

        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(request),
                &mut transport,
            )
            .await
            .expect("request handling");

        assert!(
            !transport
                .send_log()
                .iter()
                .any(|message| matches!(message, ServiceMessage::StoreServiceConfig(_))),
            "edit should not issue StoreServiceConfig for cross-tenant id"
        );
        let error = expect_last_surface_error(&transport);
        assert_eq!(error.code, SurfaceActionErrorCode::InvalidRequest);
        assert!(
            error.message.contains("not found"),
            "unexpected message: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn delete_action_rejects_cross_tenant_lookup_by_id() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let request_tenant_id = Uuid::now_v7();
        let config_tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();
        runtime.service_tenant_id = Some(request_tenant_id);
        runtime.configs =
            parse_client_configs(vec![config_entry(config_tenant_id, mqtt_client_id)]);

        let request = action_request(
            surface_runtime::ACTION_DELETE,
            request_tenant_id,
            Some("service.uptrakit-mqtt"),
            serde_json::Map::from_iter([(
                "id".to_string(),
                serde_json::json!(mqtt_client_id.to_string()),
            )]),
        );

        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(request),
                &mut transport,
            )
            .await
            .expect("request handling");

        assert!(
            !transport
                .send_log()
                .iter()
                .any(|message| matches!(message, ServiceMessage::DeleteServiceConfig(_))),
            "delete should not issue DeleteServiceConfig for cross-tenant id"
        );
        let error = expect_last_surface_error(&transport);
        assert_eq!(error.code, SurfaceActionErrorCode::InvalidRequest);
        assert!(
            error.message.contains("not found"),
            "unexpected message: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn apply_settings_registers_extensions_when_enabled() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();

        runtime
            .on_connected(
                &mut transport,
                MqttRuntimeIdentity {
                    service_id: Some(Uuid::now_v7()),
                    private_key_der: None,
                    encryption_public_key: Some("public-key".to_string()),
                },
            )
            .await
            .expect("connect should succeed");
        runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: true,
                    tenant_id: Some(Uuid::now_v7()),
                },
                &mut transport,
            )
            .await;

        assert!(matches!(
            transport.send_log().first(),
            Some(ServiceMessage::Register(_))
        ));
        assert!(matches!(
            transport.send_log().get(1),
            Some(ServiceMessage::SurfaceRegistration(_))
        ));
    }

    #[tokio::test]
    async fn apply_settings_registers_surfaces_best_effort_when_reliable_send_is_failing() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();

        runtime
            .on_connected(
                &mut transport,
                MqttRuntimeIdentity {
                    service_id: Some(Uuid::now_v7()),
                    private_key_der: None,
                    encryption_public_key: Some("public-key".to_string()),
                },
            )
            .await
            .expect("connect should succeed");
        transport.set_fail_send(true);

        runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: true,
                    tenant_id: Some(Uuid::now_v7()),
                },
                &mut transport,
            )
            .await;

        assert!(matches!(
            transport.send_log().first(),
            Some(ServiceMessage::Register(_))
        ));
        assert!(matches!(
            transport.send_log().get(1),
            Some(ServiceMessage::SurfaceRegistration(_))
        ));
    }

    #[tokio::test]
    async fn edit_surface_action_completes_after_service_config_ack() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let existing = existing_config();
        let tenant_id = existing.tenant_id;
        runtime.service_tenant_id = Some(tenant_id);
        runtime.service_id = Some(Uuid::now_v7());
        runtime.configs = vec![existing.clone()];

        let request = action_request(
            surface_runtime::ACTION_EDIT,
            tenant_id,
            Some(&runtime.expected_provider_id()),
            serde_json::Map::from_iter([
                (
                    "id".to_string(),
                    serde_json::json!(existing.mqtt_client_id.to_string()),
                ),
                ("enabled".to_string(), serde_json::json!(true)),
                ("transport".to_string(), serde_json::json!("tls")),
                (
                    "host".to_string(),
                    serde_json::json!("updated-broker.example.com"),
                ),
                ("port".to_string(), serde_json::json!(8883)),
                ("client_id".to_string(), serde_json::json!("updated-client")),
                ("username".to_string(), serde_json::json!("user")),
                ("topic_prefix".to_string(), serde_json::json!("uptrakit")),
                ("ha_discovery".to_string(), serde_json::json!(true)),
                (
                    "ha_discovery_prefix".to_string(),
                    serde_json::json!("homeassistant"),
                ),
            ]),
        );

        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(request),
                &mut transport,
            )
            .await
            .expect("surface action request should succeed");

        let Some(ServiceMessage::StoreServiceConfig(store)) = transport.send_log().last() else {
            panic!("expected StoreServiceConfig to be sent");
        };
        assert!(
            !transport
                .send_log()
                .iter()
                .any(|message| matches!(message, ServiceMessage::SurfaceActionResponse(_))),
            "surface response must wait for ServiceConfigAck"
        );

        runtime.on_service_config_ack(
            uptrakit_internal_wire::payloads::ServiceConfigAckPayload::success(
                store.request_id.clone(),
            ),
        );

        let event = runtime.poll_event().await;
        assert!(matches!(
            event,
            Some(MqttServiceEvent::SurfaceConfigRequestCompleted { .. })
        ));
        runtime.handle_event(event, &mut transport).await;

        assert!(
            runtime.configs.iter().any(|config| {
                config.mqtt_client_id == existing.mqtt_client_id
                    && config.host == "updated-broker.example.com"
                    && config.client_id == "updated-client"
            }),
            "local config cache should be reconciled after ack"
        );
        assert!(
            transport.send_log().iter().any(|message| matches!(
                message,
                ServiceMessage::SurfaceActionResponse(response) if response.success
            )),
            "surface success response should be emitted after ack processing"
        );
    }

    #[tokio::test]
    async fn apply_settings_skips_surface_registration_without_tenant_binding() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();

        runtime
            .on_connected(
                &mut transport,
                MqttRuntimeIdentity {
                    service_id: Some(Uuid::now_v7()),
                    private_key_der: None,
                    encryption_public_key: Some("public-key".to_string()),
                },
            )
            .await
            .expect("connect should succeed");
        runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: true,
                    tenant_id: None,
                },
                &mut transport,
            )
            .await;

        assert!(matches!(
            transport.send_log().first(),
            Some(ServiceMessage::Register(_))
        ));
        assert!(
            !transport
                .send_log()
                .iter()
                .any(|message| matches!(message, ServiceMessage::SurfaceRegistration(_)))
        );
    }

    #[tokio::test]
    async fn reconcile_after_local_create_updates_local_configs_and_claims() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");

        runtime
            .reconcile_after_local_config_update(
                ServiceConfigUpdatedPayload::new(
                    vec![config_entry(tenant_id, mqtt_client_id)],
                    Vec::new(),
                ),
                &mut transport,
            )
            .await;

        assert_eq!(runtime.configs.len(), 1);
        assert_eq!(runtime.configs[0].mqtt_client_id, mqtt_client_id);
        let Some(ServiceMessage::WorkloadClaim(payload)) = transport.send_log().last() else {
            panic!("expected WorkloadClaim after local create reconcile");
        };
        assert_eq!(payload.claims.get(&key), Some(&tenant_id));
    }

    #[tokio::test]
    async fn reconcile_after_local_edit_updates_existing_config() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();

        runtime.configs = parse_client_configs(vec![config_entry_with_host(
            tenant_id,
            mqtt_client_id,
            "old-broker.example.com",
        )]);

        runtime
            .reconcile_after_local_config_update(
                ServiceConfigUpdatedPayload::new(
                    vec![config_entry_with_host(
                        tenant_id,
                        mqtt_client_id,
                        "new-broker.example.com",
                    )],
                    Vec::new(),
                ),
                &mut transport,
            )
            .await;

        assert_eq!(runtime.configs.len(), 1);
        assert_eq!(runtime.configs[0].mqtt_client_id, mqtt_client_id);
        assert_eq!(runtime.configs[0].host, "new-broker.example.com");
        assert!(matches!(
            transport.send_log().last(),
            Some(ServiceMessage::WorkloadClaim(_))
        ));
    }

    #[tokio::test]
    async fn reconcile_after_local_delete_removes_config_and_claims() {
        let mut runtime = MqttRuntime::new();
        let mut transport = MockTransport::new();
        let tenant_id = Uuid::now_v7();
        let mqtt_client_id = Uuid::now_v7();
        let key = format!("{CONFIG_KEY_PREFIX}{mqtt_client_id}");

        runtime.configs = parse_client_configs(vec![config_entry(tenant_id, mqtt_client_id)]);
        runtime.granted_keys.insert(key.clone());

        runtime
            .reconcile_after_local_config_update(
                ServiceConfigUpdatedPayload::new(
                    Vec::new(),
                    vec![uptrakit_internal_wire::payloads::ServiceConfigKey::new(
                        Some(tenant_id),
                        key.clone(),
                    )],
                ),
                &mut transport,
            )
            .await;

        assert!(runtime.configs.is_empty());
        assert!(!runtime.granted_keys.contains(&key));
        let Some(ServiceMessage::WorkloadClaim(payload)) = transport.send_log().last() else {
            panic!("expected WorkloadClaim after local delete reconcile");
        };
        assert!(payload.claims.is_empty());
    }
}
