use std::collections::HashMap;

use futures_util::stream::{FuturesUnordered, StreamExt};
use uptrakit_internal_wire::HostConnectivityUpdate;
use uptrakit_internal_wire::MqttTenantConfig;
use uuid::Uuid;

use crate::client_manager::{ClientState, build_config_from_wire, compute_config_hash};
use crate::mqtt_client::MqttServiceEvent;
use crate::state_publisher::{compute_removed_host_ids, compute_removed_items};
use tokio::sync::mpsc;
use uptrakit_internal_wire::MqttClientConnectionStatus;

/// Cached connectivity state for a single host within a tenant.
#[derive(Debug, Clone)]
pub(crate) struct ConnectivityState {
    pub(crate) online: bool,
    pub(crate) last_seen_at: Option<String>,
    pub(crate) agent_version: Option<String>,
}

/// Manages per-MQTT-client lifecycles with push-based config updates.
///
/// Unlike the database-polling version, this manager receives configuration
/// updates from the controller via WebSocket messages.
pub(crate) struct TenantManager {
    pub(crate) clients: HashMap<Uuid, ClientState>,
    pub(crate) event_tx: Option<mpsc::Sender<MqttServiceEvent>>,
    pub(crate) software_states: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttSoftwareStateItem>>,
    /// Cached per-host summary states, keyed by tenant_id.
    pub(crate) host_summary_states: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttHostSummary>>,
    /// Cached per-host metadata (OS info, tags, agent last_seen), keyed by tenant_id.
    pub(crate) host_metadata: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttHostMetadata>>,
    /// Cached per-host connectivity state, keyed by `(tenant_id, host_id)`.
    ///
    /// Updated by `HostConnectivityUpdated` events from the controller. Not
    /// sourced from `SoftwareStates` so that multi-controller deployments
    /// always receive the authoritative online/offline state from whichever
    /// controller holds the agent WebSocket connection.
    pub(crate) connectivity_cache: HashMap<(Uuid, Uuid), ConnectivityState>,
}

impl TenantManager {
    pub(crate) fn new(event_tx: Option<mpsc::Sender<MqttServiceEvent>>) -> Self {
        Self {
            clients: HashMap::new(),
            event_tx,
            software_states: HashMap::new(),
            host_summary_states: HashMap::new(),
            host_metadata: HashMap::new(),
            connectivity_cache: HashMap::new(),
        }
    }

    /// Apply MQTT client assignments from the controller.
    ///
    /// This is called when receiving `TenantAssignments` message.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn apply_assignments(&mut self, configs: Vec<MqttTenantConfig>) {
        for config in configs {
            if config.enabled {
                self.start_or_update_client(config).await;
            } else {
                self.stop_client(&config.mqtt_client_id).await;
            }
        }
    }

    /// Reload a single MQTT client's configuration.
    ///
    /// This is called when receiving `TenantConfigUpdated` message.
    #[tracing::instrument(skip_all, fields(mqtt_client_id = %config.mqtt_client_id))]
    pub(crate) async fn reload_client(&mut self, config: MqttTenantConfig) {
        if config.enabled {
            self.start_or_update_client(config).await;
        } else {
            self.stop_client(&config.mqtt_client_id).await;
        }
    }

    /// Stop an MQTT client.
    ///
    /// This is called when receiving `TenantRevoked` message or when config is disabled.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    pub(crate) async fn stop_client(&mut self, mqtt_client_id: &Uuid) {
        if let Some(state) = self.clients.remove(mqtt_client_id) {
            tracing::info!(%mqtt_client_id, "shutting down MQTT client");
            self.report_status(*mqtt_client_id, MqttClientConnectionStatus::Offline);
            state.handle.shutdown().await;
        }
    }

    /// Return list of active MQTT client IDs (used in `Disconnecting` payload).
    pub(crate) fn active_mqtt_client_ids(&self) -> Vec<Uuid> {
        self.clients.keys().copied().collect()
    }

    /// Graceful shutdown: stop all MQTT clients.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn shutdown_all(&mut self) {
        let clients = std::mem::take(&mut self.clients);
        let mut tasks = FuturesUnordered::new();

        for (mqtt_client_id, state) in clients {
            tasks.push(async move {
                tracing::info!(%mqtt_client_id, "shutting down MQTT client");
                state.handle.shutdown().await;
            });
            self.report_status(mqtt_client_id, MqttClientConnectionStatus::Offline);
        }

        while tasks.next().await.is_some() {}
    }

    /// Store new software state data for a tenant and push to all connected
    /// clients for that tenant.
    ///
    /// State and version topics are published for every connected client.
    /// Home Assistant discovery config topics are published only for clients
    /// that have `ha_discovery` enabled.
    ///
    /// Before replacing the caches, computes diff sets of removed items/hosts
    /// and publishes empty retained payloads to clean up stale MQTT topics.
    #[tracing::instrument(skip_all, fields(tenant_id = %payload.tenant_id))]
    pub(crate) async fn update_software_states(
        &mut self,
        payload: uptrakit_internal_wire::MqttSoftwareStatesPayload,
    ) {
        let tenant_id = payload.tenant_id;

        // Compute removed sets before replacing the caches.
        let removed_items =
            compute_removed_items(self.software_states.get(&tenant_id), &payload.items);
        let removed_summary_hosts = compute_removed_host_ids(
            self.host_summary_states
                .get(&tenant_id)
                .map(|v| v.iter().map(|h| h.host_id)),
            payload.host_summaries.iter().map(|h| h.host_id),
        );
        let removed_metadata_hosts = compute_removed_host_ids(
            self.host_metadata
                .get(&tenant_id)
                .map(|v| v.iter().map(|h| h.host_id)),
            payload.hosts.iter().map(|h| h.host_id),
        );

        // Replace caches with new data.
        self.software_states
            .insert(tenant_id, payload.items.clone());
        self.host_summary_states
            .insert(tenant_id, payload.host_summaries.clone());
        self.host_metadata.insert(tenant_id, payload.hosts.clone());

        // Clean up connectivity cache entries for removed metadata hosts.
        for host_id in &removed_metadata_hosts {
            self.connectivity_cache.remove(&(tenant_id, *host_id));
        }

        // Collect client IDs for this tenant (all of them, not just HA-enabled).
        let client_ids: Vec<uuid::Uuid> = self
            .clients
            .iter()
            .filter(|(_, s)| s.tenant_id == tenant_id)
            .map(|(id, _)| *id)
            .collect();

        for client_id in &client_ids {
            // Clean up stale topics for removed entities.
            if !removed_items.is_empty() {
                self.cleanup_removed_items(*client_id, tenant_id, &removed_items)
                    .await;
            }
            if !removed_summary_hosts.is_empty() {
                self.cleanup_removed_host_summaries(*client_id, tenant_id, &removed_summary_hosts)
                    .await;
            }
            if !removed_metadata_hosts.is_empty() {
                self.cleanup_removed_host_metadata(*client_id, tenant_id, &removed_metadata_hosts)
                    .await;
            }

            // Publish new state.
            self.publish_software_states(*client_id, &payload.items)
                .await;
            self.publish_host_summary_states(*client_id, &payload.host_summaries)
                .await;
            self.publish_host_metadata(*client_id, &payload.hosts).await;
        }
    }

    /// Handle a `HostConnectivityUpdated` message from the controller.
    ///
    /// Updates the in-memory connectivity cache and publishes the connectivity
    /// state and attributes topics for affected hosts across all clients of
    /// that tenant. Connectivity discovery configs are published for
    /// HA-enabled clients on first sight of a host.
    #[tracing::instrument(skip_all, fields(tenant_id = %tenant_id))]
    pub(crate) async fn handle_host_connectivity_updated(
        &mut self,
        tenant_id: Uuid,
        updates: Vec<HostConnectivityUpdate>,
    ) {
        // Update cache.
        for update in &updates {
            self.connectivity_cache.insert(
                (tenant_id, update.host_id),
                ConnectivityState {
                    online: update.online,
                    last_seen_at: update.last_seen_at.clone(),
                    agent_version: update.agent_version.clone(),
                },
            );
        }

        // Publish to all clients for this tenant.
        let client_ids: Vec<Uuid> = self
            .clients
            .iter()
            .filter(|(_, s)| s.tenant_id == tenant_id)
            .map(|(id, _)| *id)
            .collect();

        for client_id in client_ids {
            for update in &updates {
                self.publish_connectivity_for_host(client_id, tenant_id, update.host_id)
                    .await;
            }
        }
    }

    /// Called on MQTT broker reconnect: republish all state and discovery topics.
    ///
    /// Republishes both state/version topics (for all clients) and HA discovery
    /// config topics (only for HA-enabled clients) from the in-memory cache.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    pub(crate) async fn handle_reconnected(&mut self, mqtt_client_id: &uuid::Uuid) {
        let Some(state) = self.clients.get(mqtt_client_id) else {
            return;
        };
        let tenant_id = state.tenant_id;
        if let Some(items) = self.software_states.get(&tenant_id).cloned() {
            self.publish_software_states(*mqtt_client_id, &items).await;
        }
        if let Some(host_states) = self.host_summary_states.get(&tenant_id).cloned() {
            self.publish_host_summary_states(*mqtt_client_id, &host_states)
                .await;
        }
        if let Some(metadata) = self.host_metadata.get(&tenant_id).cloned() {
            self.publish_host_metadata(*mqtt_client_id, &metadata).await;
        }
        // Republish connectivity topics for all cached hosts of this tenant.
        let host_ids: Vec<Uuid> = self
            .connectivity_cache
            .keys()
            .filter(|(tid, _)| *tid == tenant_id)
            .map(|(_, hid)| *hid)
            .collect();
        for host_id in host_ids {
            self.publish_connectivity_for_host(*mqtt_client_id, tenant_id, host_id)
                .await;
        }
    }

    /// Called when HA sends its birth message (restarted): republish only HA
    /// discovery config topics.
    ///
    /// State and version topics are retained on the broker and do not need
    /// re-sending after an HA restart. Only the `{ha_prefix}/update/.../config`
    /// and `{ha_prefix}/binary_sensor/.../config` messages need to be
    /// republished so that HA re-registers its entities.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    pub(crate) async fn handle_ha_online(&mut self, mqtt_client_id: &uuid::Uuid) {
        let Some(state) = self.clients.get(mqtt_client_id) else {
            return;
        };
        if !state.ha_discovery {
            return;
        }
        let tenant_id = state.tenant_id;
        if let Some(items) = self.software_states.get(&tenant_id).cloned() {
            self.publish_ha_configs_only(*mqtt_client_id, &items).await;
        }
        if let Some(host_states) = self.host_summary_states.get(&tenant_id).cloned() {
            self.publish_host_summary_ha_configs_only(*mqtt_client_id, &host_states)
                .await;
        }
        // Republish connectivity discovery configs for all known hosts.
        let host_ids: Vec<Uuid> = self
            .connectivity_cache
            .keys()
            .filter(|(tid, _)| *tid == tenant_id)
            .map(|(_, hid)| *hid)
            .collect();
        for host_id in host_ids {
            self.publish_connectivity_discovery_config(*mqtt_client_id, tenant_id, host_id)
                .await;
        }
    }

    /// Given an inbound MQTT command topic, resolve it to an
    /// [`MqttUpdateTriggerPayload`](uptrakit_internal_wire::MqttUpdateTriggerPayload).
    ///
    /// Returns `None` if the topic doesn't match any known `(item, host)` in
    /// the stored states.
    pub(crate) fn resolve_update_trigger(
        &self,
        mqtt_client_id: uuid::Uuid,
        topic: &str,
    ) -> Option<uptrakit_internal_wire::MqttUpdateTriggerPayload> {
        let state = self.clients.get(&mqtt_client_id)?;
        let (item_id, host_id) =
            crate::ha_discovery::parse_command_topic(&state.topic_prefix, topic)?;
        let tenant_id = state.tenant_id;
        let items = self.software_states.get(&tenant_id)?;
        let item = items.iter().find(|i| i.software_item_id == item_id)?;
        let host = item.hosts.iter().find(|h| h.host_id == host_id)?;
        let to_version = host.latest_version.clone()?;
        Some(uptrakit_internal_wire::MqttUpdateTriggerPayload {
            tenant_id,
            software_item_id: item_id,
            host_id,
            to_version,
            mqtt_client_id,
        })
    }

    /// Given an inbound MQTT command topic, resolve it to an
    /// [`MqttTriggerHostBatchUpdatePayload`](uptrakit_internal_wire::MqttTriggerHostBatchUpdatePayload).
    ///
    /// Returns `None` if the topic doesn't match the host-packages command
    /// pattern `{prefix}/hosts/{host_id}/set`.
    pub(crate) fn resolve_host_batch_update_trigger(
        &self,
        mqtt_client_id: uuid::Uuid,
        topic: &str,
    ) -> Option<uptrakit_internal_wire::MqttTriggerHostBatchUpdatePayload> {
        let state = self.clients.get(&mqtt_client_id)?;
        let host_id =
            crate::ha_discovery::parse_host_packages_command_topic(&state.topic_prefix, topic)?;
        Some(uptrakit_internal_wire::MqttTriggerHostBatchUpdatePayload {
            tenant_id: state.tenant_id,
            host_id,
            mqtt_client_id,
            security_only: false,
        })
    }

    /// Given an inbound MQTT security-entity command topic, resolve it to an
    /// [`MqttTriggerHostBatchUpdatePayload`](uptrakit_internal_wire::MqttTriggerHostBatchUpdatePayload)
    /// with `security_only = true`.
    ///
    /// Returns `None` if the topic doesn't match the security command
    /// pattern `{prefix}/hosts/{host_id}/security/set`.
    pub(crate) fn resolve_host_security_batch_update_trigger(
        &self,
        mqtt_client_id: uuid::Uuid,
        topic: &str,
    ) -> Option<uptrakit_internal_wire::MqttTriggerHostBatchUpdatePayload> {
        let state = self.clients.get(&mqtt_client_id)?;
        let host_id =
            crate::ha_discovery::parse_host_security_command_topic(&state.topic_prefix, topic)?;
        Some(uptrakit_internal_wire::MqttTriggerHostBatchUpdatePayload {
            tenant_id: state.tenant_id,
            host_id,
            mqtt_client_id,
            security_only: true,
        })
    }

    /// Start or update an MQTT client.
    #[tracing::instrument(skip_all, fields(mqtt_client_id = %config.mqtt_client_id))]
    async fn start_or_update_client(&mut self, config: MqttTenantConfig) {
        let mqtt_client_id = config.mqtt_client_id;
        let new_hash = compute_config_hash(&config);

        // Check if we already have this client with same config
        if let Some(state) = self.clients.get(&mqtt_client_id) {
            if state.config_hash == new_hash {
                tracing::debug!(%mqtt_client_id, "config unchanged, skipping update");
                return;
            }
            tracing::info!(%mqtt_client_id, "config changed, reloading");
        }

        // Stop existing client if any
        if let Some(state) = self.clients.remove(&mqtt_client_id) {
            self.report_status(mqtt_client_id, MqttClientConnectionStatus::Offline);
            state.handle.shutdown().await;
        }

        // Build and start new client
        let mqtt_config = build_config_from_wire(&config);
        tracing::info!(%mqtt_client_id, config = ?mqtt_config, "starting MQTT client");

        let ha_status_topic = if config.ha_discovery {
            Some(format!("{}/status", config.ha_discovery_prefix))
        } else {
            None
        };

        let handle = crate::mqtt_client::start(
            mqtt_config,
            self.event_tx.clone(),
            mqtt_client_id,
            ha_status_topic,
        )
        .await;

        self.clients.insert(
            mqtt_client_id,
            ClientState {
                handle,
                config_hash: new_hash,
                tenant_id: config.tenant_id,
                topic_prefix: config.topic_prefix.clone(),
                ha_discovery: config.ha_discovery,
                ha_discovery_prefix: config.ha_discovery_prefix.clone(),
            },
        );
    }

    fn report_status(&self, mqtt_client_id: Uuid, status: MqttClientConnectionStatus) {
        let Some(sender) = self.event_tx.as_ref() else {
            return;
        };
        if let Err(e) = sender.try_send(MqttServiceEvent::Status(
            crate::mqtt_client::MqttClientStatusEvent {
                mqtt_client_id,
                status,
            },
        )) {
            tracing::warn!(error = %e, "MQTT event channel full, dropping status event");
        }
    }
}

impl Default for TenantManager {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_manager::build_config_from_wire;
    use crate::client_manager::compute_config_hash;
    use crate::state_publisher::{compute_removed_host_ids, compute_removed_items};
    use time::UtcDateTime;
    use uptrakit_internal_wire::MqttTransport;
    use uptrakit_internal_wire::SecretString;

    #[test]
    fn build_config_from_wire_correct() {
        let config = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000001").unwrap(),
            tenant_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "my-client".to_string(),
            username: Some(SecretString::new("user")),
            password: Some(SecretString::new("pass")),
            ca_pem: None,
            topic_prefix: "home/uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let mqtt_config = build_config_from_wire(&config);

        assert_eq!(mqtt_config.transport, MqttTransport::Tls);
        assert_eq!(mqtt_config.host, "broker.example.com");
        assert_eq!(mqtt_config.port, 8883);
        assert_eq!(mqtt_config.client_id, "my-client");
        assert_eq!(
            mqtt_config.username.as_ref().map(|s| s.expose_secret()),
            Some("user")
        );
        assert_eq!(
            mqtt_config.password.as_ref().map(|s| s.expose_secret()),
            Some("pass")
        );
        assert_eq!(mqtt_config.topic_prefix, "home/uptrakit");
    }

    #[test]
    fn build_config_uses_default_port_when_zero() {
        let config = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000002").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 0,
            client_id: "client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let mqtt_config = build_config_from_wire(&config);
        assert_eq!(mqtt_config.port, 8883); // TLS default port
    }

    #[test]
    fn config_hash_changes_on_different_values() {
        let config1 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000003").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker1.example.com".to_string(),
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let config2 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000003").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker2.example.com".to_string(), // Different host
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        assert_ne!(compute_config_hash(&config1), compute_config_hash(&config2));
    }

    #[test]
    fn build_config_uses_default_port_for_tcp_when_zero() {
        let config = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000005").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: MqttTransport::Tcp,
            host: "broker.example.com".to_string(),
            port: 0,
            client_id: "client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let mqtt_config = build_config_from_wire(&config);
        assert_eq!(mqtt_config.port, 1883); // TCP default port
    }

    #[test]
    fn build_config_no_credentials() {
        let config = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000006").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: MqttTransport::Tcp,
            host: "broker.local".to_string(),
            port: 1883,
            client_id: "anon-client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "prefix".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let mqtt_config = build_config_from_wire(&config);
        assert!(mqtt_config.username.is_none());
        assert!(mqtt_config.password.is_none());
    }

    #[test]
    fn tenant_manager_new_has_no_clients() {
        let manager = TenantManager::new(None);
        assert!(manager.active_mqtt_client_ids().is_empty());
    }

    #[test]
    fn tenant_manager_default_has_no_clients() {
        let manager = TenantManager::default();
        assert!(manager.active_mqtt_client_ids().is_empty());
    }

    #[tokio::test]
    async fn stop_client_noop_for_nonexistent() {
        let mut manager = TenantManager::new(None);
        let fake_id = Uuid::parse_str("019471a0-0000-7000-8000-000000000099").unwrap();
        // Should not panic or error.
        manager.stop_client(&fake_id).await;
        assert!(manager.active_mqtt_client_ids().is_empty());
    }

    #[tokio::test]
    async fn shutdown_all_on_empty_manager() {
        let mut manager = TenantManager::new(None);
        // Should not panic on empty manager.
        manager.shutdown_all().await;
        assert!(manager.active_mqtt_client_ids().is_empty());
    }

    #[tokio::test]
    async fn apply_assignments_disabled_configs_ignored() {
        let mut manager = TenantManager::new(None);
        let configs = vec![MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000010").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: false, // disabled
            transport: MqttTransport::Tcp,
            host: "broker.local".to_string(),
            port: 1883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        }];
        // Disabled configs should be a no-op (stop_client on non-existent is noop).
        manager.apply_assignments(configs).await;
        assert!(manager.active_mqtt_client_ids().is_empty());
    }

    #[tokio::test]
    async fn apply_assignments_empty_vec() {
        let mut manager = TenantManager::new(None);
        manager.apply_assignments(vec![]).await;
        assert!(manager.active_mqtt_client_ids().is_empty());
    }

    #[test]
    fn config_hash_same_for_same_values() {
        let config1 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000004").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let config2 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000004").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::from_unix_timestamp(12345).unwrap(), // Different updated_at doesn't matter
        };

        assert_eq!(compute_config_hash(&config1), compute_config_hash(&config2));
    }

    #[test]
    fn resolve_update_trigger_returns_none_for_unknown_client() {
        let manager = TenantManager::new(None);
        let unknown_id = Uuid::parse_str("019471a0-0000-7000-8000-000000000099").unwrap();
        let result = manager.resolve_update_trigger(unknown_id, "uptrakit/update/anything/set");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn update_software_states_stores_for_all_tenants() {
        // update_software_states must cache state regardless of ha_discovery.
        let mut manager = TenantManager::new(None);
        let tenant_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap();

        let payload = uptrakit_internal_wire::MqttSoftwareStatesPayload {
            tenant_id,
            items: vec![uptrakit_internal_wire::MqttSoftwareStateItem {
                software_item_id: Uuid::nil(),
                name: "nginx".to_string(),
                icon_url: None,
                hosts: vec![],
            }],
            host_summaries: vec![],
            hosts: vec![],
        };

        // No connected clients — but the cache must still be updated.
        manager.update_software_states(payload).await;

        // Verify the cache was populated (resolve_update_trigger won't find an
        // unknown client, but the internal cache entry is what matters here).
        assert!(manager.software_states.contains_key(&tenant_id));
        assert_eq!(manager.software_states[&tenant_id].len(), 1);
        assert_eq!(manager.software_states[&tenant_id][0].name, "nginx");
    }

    #[tokio::test]
    async fn handle_reconnected_noop_for_unknown_client() {
        // handle_reconnected must not panic for a client that isn't in the map.
        let mut manager = TenantManager::new(None);
        let unknown_id = Uuid::parse_str("019471a0-0000-7000-8000-000000000099").unwrap();
        // Should return without panicking even with no clients and no states.
        manager.handle_reconnected(&unknown_id).await;
    }

    #[tokio::test]
    async fn handle_ha_online_noop_for_unknown_client() {
        // handle_ha_online must not panic for a client that isn't in the map.
        let mut manager = TenantManager::new(None);
        let unknown_id = Uuid::parse_str("019471a0-0000-7000-8000-000000000099").unwrap();
        manager.handle_ha_online(&unknown_id).await;
    }

    #[tokio::test]
    async fn update_software_states_replaces_cached_items() {
        // A second update for the same tenant replaces the first in the cache.
        let mut manager = TenantManager::new(None);
        let tenant_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440002").unwrap();

        let first = uptrakit_internal_wire::MqttSoftwareStatesPayload {
            tenant_id,
            items: vec![uptrakit_internal_wire::MqttSoftwareStateItem {
                software_item_id: Uuid::nil(),
                name: "nginx".to_string(),
                icon_url: None,
                hosts: vec![],
            }],
            host_summaries: vec![],
            hosts: vec![],
        };
        manager.update_software_states(first).await;

        let second = uptrakit_internal_wire::MqttSoftwareStatesPayload {
            tenant_id,
            items: vec![
                uptrakit_internal_wire::MqttSoftwareStateItem {
                    software_item_id: Uuid::nil(),
                    name: "nginx".to_string(),
                    icon_url: None,
                    hosts: vec![],
                },
                uptrakit_internal_wire::MqttSoftwareStateItem {
                    software_item_id: Uuid::from_u128(1),
                    name: "redis".to_string(),
                    icon_url: None,
                    hosts: vec![],
                },
            ],
            host_summaries: vec![],
            hosts: vec![],
        };
        manager.update_software_states(second).await;

        assert_eq!(manager.software_states[&tenant_id].len(), 2);
        assert_eq!(manager.software_states[&tenant_id][1].name, "redis");
    }

    // -------------------------------------------------------------------------
    // compute_removed_items
    // -------------------------------------------------------------------------

    fn make_item(
        item_id: Uuid,
        host_ids: &[Uuid],
    ) -> uptrakit_internal_wire::MqttSoftwareStateItem {
        uptrakit_internal_wire::MqttSoftwareStateItem {
            software_item_id: item_id,
            name: "test".to_string(),
            icon_url: None,
            hosts: host_ids
                .iter()
                .map(|&hid| uptrakit_internal_wire::MqttSoftwareStateHostEntry {
                    host_id: hid,
                    hostname: "host".to_string(),
                    friendly_name: String::new(),
                    installed_version: None,
                    latest_version: None,
                    update_available: false,
                    release_url: None,
                    release_notes: None,
                    update_in_progress: false,
                    update_category: None,
                    release_date: None,
                    last_checked_at: None,
                })
                .collect(),
        }
    }

    #[test]
    fn compute_removed_items_no_old_returns_empty() {
        let new = vec![make_item(Uuid::from_u128(1), &[Uuid::from_u128(10)])];
        let removed = compute_removed_items(None, &new);
        assert!(removed.is_empty());
    }

    #[test]
    fn compute_removed_items_same_returns_empty() {
        let items = vec![make_item(Uuid::from_u128(1), &[Uuid::from_u128(10)])];
        let removed = compute_removed_items(Some(&items), &items);
        assert!(removed.is_empty());
    }

    #[test]
    fn compute_removed_items_detects_removed_pair() {
        let item_a = Uuid::from_u128(1);
        let host_a = Uuid::from_u128(10);
        let host_b = Uuid::from_u128(20);

        let old = vec![make_item(item_a, &[host_a, host_b])];
        let new = vec![make_item(item_a, &[host_a])]; // host_b removed

        let removed = compute_removed_items(Some(&old), &new);
        assert_eq!(removed.len(), 1);
        assert!(removed.contains(&(item_a, host_b)));
    }

    #[test]
    fn compute_removed_items_detects_entire_item_removed() {
        let item_a = Uuid::from_u128(1);
        let item_b = Uuid::from_u128(2);
        let host = Uuid::from_u128(10);

        let old = vec![make_item(item_a, &[host]), make_item(item_b, &[host])];
        let new = vec![make_item(item_a, &[host])]; // item_b removed entirely

        let removed = compute_removed_items(Some(&old), &new);
        assert_eq!(removed.len(), 1);
        assert!(removed.contains(&(item_b, host)));
    }

    #[test]
    fn compute_removed_items_new_items_not_in_removed() {
        let item_a = Uuid::from_u128(1);
        let item_b = Uuid::from_u128(2);
        let host = Uuid::from_u128(10);

        let old = vec![make_item(item_a, &[host])];
        let new = vec![make_item(item_a, &[host]), make_item(item_b, &[host])];

        let removed = compute_removed_items(Some(&old), &new);
        assert!(removed.is_empty());
    }

    // -------------------------------------------------------------------------
    // compute_removed_host_ids
    // -------------------------------------------------------------------------

    #[test]
    fn compute_removed_host_ids_no_old_returns_empty() {
        let new = vec![Uuid::from_u128(1)];
        let removed = compute_removed_host_ids(None::<std::vec::IntoIter<Uuid>>, new.into_iter());
        assert!(removed.is_empty());
    }

    #[test]
    fn compute_removed_host_ids_same_returns_empty() {
        let ids = vec![Uuid::from_u128(1), Uuid::from_u128(2)];
        let removed = compute_removed_host_ids(Some(ids.clone().into_iter()), ids.into_iter());
        assert!(removed.is_empty());
    }

    #[test]
    fn compute_removed_host_ids_detects_removed() {
        let host_a = Uuid::from_u128(1);
        let host_b = Uuid::from_u128(2);

        let old = vec![host_a, host_b];
        let new = vec![host_a]; // host_b removed

        let removed = compute_removed_host_ids(Some(old.into_iter()), new.into_iter());
        assert_eq!(removed.len(), 1);
        assert!(removed.contains(&host_b));
    }

    #[test]
    fn compute_removed_host_ids_new_hosts_not_in_removed() {
        let host_a = Uuid::from_u128(1);
        let host_b = Uuid::from_u128(2);

        let old = vec![host_a];
        let new = vec![host_a, host_b]; // host_b is new

        let removed = compute_removed_host_ids(Some(old.into_iter()), new.into_iter());
        assert!(removed.is_empty());
    }
}
