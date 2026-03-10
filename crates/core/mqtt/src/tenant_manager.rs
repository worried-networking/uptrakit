use std::collections::HashMap;

use futures_util::stream::{FuturesUnordered, StreamExt};
use uptrakit_internal_wire::HostConnectivityUpdate;
use uptrakit_internal_wire::MqttTenantConfig;
use uuid::Uuid;

use crate::mqtt_client::{MqttConfig, MqttHandle, MqttServiceEvent};
use tokio::sync::mpsc;
use uptrakit_internal_wire::MqttClientConnectionStatus;

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

/// Tracks the cached state for an MQTT client.
struct ClientState {
    handle: MqttHandle,
    config_hash: u64,
    tenant_id: uuid::Uuid,
    topic_prefix: String,
    ha_discovery: bool,
    ha_discovery_prefix: String,
}

/// Cached connectivity state for a single host within a tenant.
#[derive(Debug, Clone)]
struct ConnectivityState {
    online: bool,
    last_seen_at: Option<String>,
    agent_version: Option<String>,
}

/// Manages per-MQTT-client lifecycles with push-based config updates.
///
/// Unlike the database-polling version, this manager receives configuration
/// updates from the controller via WebSocket messages.
pub struct TenantManager {
    clients: HashMap<Uuid, ClientState>,
    event_tx: Option<mpsc::Sender<MqttServiceEvent>>,
    software_states: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttSoftwareStateItem>>,
    /// Cached per-host summary states, keyed by tenant_id.
    host_summary_states: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttHostSummary>>,
    /// Cached per-host metadata (OS info, tags, agent last_seen), keyed by tenant_id.
    host_metadata: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttHostMetadata>>,
    /// Cached per-host connectivity state, keyed by `(tenant_id, host_id)`.
    ///
    /// Updated by `HostConnectivityUpdated` events from the controller. Not
    /// sourced from `SoftwareStates` so that multi-controller deployments
    /// always receive the authoritative online/offline state from whichever
    /// controller holds the agent WebSocket connection.
    connectivity_cache: HashMap<(Uuid, Uuid), ConnectivityState>,
}

impl TenantManager {
    pub fn new(event_tx: Option<mpsc::Sender<MqttServiceEvent>>) -> Self {
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
    pub async fn apply_assignments(&mut self, configs: Vec<MqttTenantConfig>) {
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
    pub async fn reload_client(&mut self, config: MqttTenantConfig) {
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
    pub async fn stop_client(&mut self, mqtt_client_id: &Uuid) {
        if let Some(state) = self.clients.remove(mqtt_client_id) {
            tracing::info!(%mqtt_client_id, "shutting down MQTT client");
            self.report_status(*mqtt_client_id, MqttClientConnectionStatus::Offline);
            state.handle.shutdown().await;
        }
    }

    /// Return list of active MQTT client IDs (used in `Disconnecting` payload).
    pub fn active_mqtt_client_ids(&self) -> Vec<Uuid> {
        self.clients.keys().copied().collect()
    }

    /// Graceful shutdown: stop all MQTT clients.
    #[tracing::instrument(skip_all)]
    pub async fn shutdown_all(&mut self) {
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
    #[tracing::instrument(skip_all, fields(tenant_id = %payload.tenant_id))]
    pub async fn update_software_states(
        &mut self,
        payload: uptrakit_internal_wire::MqttSoftwareStatesPayload,
    ) {
        let tenant_id = payload.tenant_id;
        self.software_states
            .insert(tenant_id, payload.items.clone());
        self.host_summary_states
            .insert(tenant_id, payload.host_summaries.clone());
        self.host_metadata.insert(tenant_id, payload.hosts.clone());

        // Collect client IDs for this tenant (all of them, not just HA-enabled).
        let client_ids: Vec<uuid::Uuid> = self
            .clients
            .iter()
            .filter(|(_, s)| s.tenant_id == tenant_id)
            .map(|(id, _)| *id)
            .collect();

        for client_id in client_ids {
            self.publish_software_states(client_id, &payload.items)
                .await;
            self.publish_host_summary_states(client_id, &payload.host_summaries)
                .await;
            self.publish_host_metadata(client_id, &payload.hosts).await;
        }
    }

    /// Handle a `HostConnectivityUpdated` message from the controller.
    ///
    /// Updates the in-memory connectivity cache and publishes the connectivity
    /// state and attributes topics for affected hosts across all clients of
    /// that tenant. Connectivity discovery configs are published for
    /// HA-enabled clients on first sight of a host.
    #[tracing::instrument(skip_all, fields(tenant_id = %tenant_id))]
    pub async fn handle_host_connectivity_updated(
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
    pub async fn handle_reconnected(&mut self, mqtt_client_id: &uuid::Uuid) {
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
    pub async fn handle_ha_online(&mut self, mqtt_client_id: &uuid::Uuid) {
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
    pub fn resolve_update_trigger(
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
    pub fn resolve_host_batch_update_trigger(
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
    pub fn resolve_host_security_batch_update_trigger(
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

    /// Publish software state topics and subscribe to command topics for all
    /// `(item, host)` pairs, then publish HA discovery config topics for clients
    /// that have `ha_discovery` enabled.
    ///
    /// Also publishes per-host `hostname` and `friendly_name` topics (retained)
    /// for MQTT explorer visibility.
    ///
    /// Called on every `SoftwareStates` push and on broker reconnect.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    async fn publish_software_states(
        &self,
        mqtt_client_id: uuid::Uuid,
        items: &[uptrakit_internal_wire::MqttSoftwareStateItem],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let tenant_id = state.tenant_id;
        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        // Build a host_id → metadata lookup for enriching HA discovery configs.
        let meta_map: std::collections::HashMap<
            uuid::Uuid,
            &uptrakit_internal_wire::MqttHostMetadata,
        > = self
            .host_metadata
            .get(&tenant_id)
            .map(|v| v.iter().map(|m| (m.host_id, m)).collect())
            .unwrap_or_default();

        // Track which hosts we've already published hostname/friendly_name for.
        let mut published_hosts = std::collections::HashSet::new();

        for item in items {
            for host in &item.hosts {
                // Per-host topics (deduplicated across items).
                if published_hosts.insert(host.host_id) {
                    self.publish_host_identity(
                        state,
                        host.host_id,
                        &host.hostname,
                        &host.friendly_name,
                    )
                    .await;
                }

                // Always: publish installed version (empty string if unknown).
                let st = crate::ha_discovery::state_topic(
                    topic_prefix,
                    item.software_item_id,
                    host.host_id,
                );
                let installed = host
                    .installed_version
                    .as_deref()
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec();
                publish_or_abort!(
                    state.handle.publish_retained(&st, installed).await,
                    mqtt_client_id,
                    "publish state topic"
                );

                // Always: publish latest version.
                let lt = crate::ha_discovery::latest_version_topic(
                    topic_prefix,
                    item.software_item_id,
                    host.host_id,
                );
                let latest = host
                    .latest_version
                    .as_deref()
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec();
                publish_or_abort!(
                    state.handle.publish_retained(&lt, latest).await,
                    mqtt_client_id,
                    "publish latest version topic"
                );

                // Always: subscribe to command topic.
                let ct = crate::ha_discovery::command_topic(
                    topic_prefix,
                    item.software_item_id,
                    host.host_id,
                );
                publish_or_abort!(
                    state.handle.subscribe_topic(&ct).await,
                    mqtt_client_id,
                    "subscribe to command topic"
                );

                // Always: publish JSON attributes.
                let at = crate::ha_discovery::json_attributes_topic(
                    topic_prefix,
                    item.software_item_id,
                    host.host_id,
                );
                let attributes_bytes = crate::ha_discovery::build_attributes_payload(
                    host.update_in_progress,
                    host.update_category.as_deref(),
                    host.release_date.as_deref(),
                    host.last_checked_at.as_deref(),
                )
                .to_string()
                .into_bytes();
                publish_or_abort!(
                    state.handle.publish_retained(&at, attributes_bytes).await,
                    mqtt_client_id,
                    "publish JSON attributes topic"
                );

                // HA-only: publish HA discovery config so HA creates an update entity.
                if state.ha_discovery {
                    let uid = crate::ha_discovery::unique_id(
                        tenant_id,
                        item.software_item_id,
                        host.host_id,
                    );
                    let config_topic = crate::ha_discovery::discovery_config_topic(ha_prefix, &uid);
                    let meta = meta_map.get(&host.host_id);
                    let os_info = crate::ha_discovery::HostOsInfo {
                        os_type: meta.and_then(|m| m.os_type.as_deref()),
                        os_version: meta.and_then(|m| m.os_version.as_deref()),
                        architecture: meta.and_then(|m| m.architecture.as_deref()),
                    };
                    let config_json = crate::ha_discovery::build_discovery_config(
                        topic_prefix,
                        tenant_id,
                        item.software_item_id,
                        host.host_id,
                        &item.name,
                        display_name(&host.friendly_name, &host.hostname),
                        crate::ha_discovery::ReleaseInfo {
                            url: host.release_url.as_deref(),
                            notes: host.release_notes.as_deref(),
                        },
                        os_info,
                    );
                    let config_bytes = config_json.to_string().into_bytes();
                    publish_or_abort!(
                        state
                            .handle
                            .publish_retained(&config_topic, config_bytes)
                            .await,
                        mqtt_client_id,
                        "publish HA discovery config"
                    );
                }
            }
        }
    }

    /// Publish retained `hostname` and `friendly_name` topics for a host.
    ///
    /// These topics are for MQTT explorer visibility and are published under
    /// `{prefix}/hosts/{host_id}/hostname` and `{prefix}/hosts/{host_id}/friendly_name`.
    async fn publish_host_identity(
        &self,
        client_state: &ClientState,
        host_id: uuid::Uuid,
        hostname: &str,
        friendly_name: &str,
    ) {
        let topic_prefix = &client_state.topic_prefix;
        let mqtt_client_id = "host_identity";

        let hn_topic = crate::ha_discovery::hostname_topic(topic_prefix, host_id);
        publish_or_abort!(
            client_state
                .handle
                .publish_retained(&hn_topic, hostname.as_bytes().to_vec())
                .await,
            mqtt_client_id,
            "publish hostname topic"
        );

        let fn_topic = crate::ha_discovery::friendly_name_topic(topic_prefix, host_id);
        publish_or_abort!(
            client_state
                .handle
                .publish_retained(&fn_topic, friendly_name.as_bytes().to_vec())
                .await,
            mqtt_client_id,
            "publish friendly_name topic"
        );
    }

    /// Republish only the Home Assistant discovery config topics for an
    /// HA-enabled client.
    ///
    /// Used exclusively by [`handle_ha_online`](Self::handle_ha_online): when HA
    /// restarts, state and version topics are already retained on the broker and
    /// do not need re-sending. Only the `{ha_prefix}/update/.../config` messages
    /// need to be republished so that HA re-registers its `update` entities.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    async fn publish_ha_configs_only(
        &self,
        mqtt_client_id: uuid::Uuid,
        items: &[uptrakit_internal_wire::MqttSoftwareStateItem],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };
        if !state.ha_discovery {
            return;
        }

        let tenant_id = state.tenant_id;
        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        // Build a host_id → metadata lookup for enriching HA discovery configs.
        let meta_map: std::collections::HashMap<
            uuid::Uuid,
            &uptrakit_internal_wire::MqttHostMetadata,
        > = self
            .host_metadata
            .get(&tenant_id)
            .map(|v| v.iter().map(|m| (m.host_id, m)).collect())
            .unwrap_or_default();

        for item in items {
            for host in &item.hosts {
                let uid =
                    crate::ha_discovery::unique_id(tenant_id, item.software_item_id, host.host_id);
                let config_topic = crate::ha_discovery::discovery_config_topic(ha_prefix, &uid);
                let meta = meta_map.get(&host.host_id);
                let os_info = crate::ha_discovery::HostOsInfo {
                    os_type: meta.and_then(|m| m.os_type.as_deref()),
                    os_version: meta.and_then(|m| m.os_version.as_deref()),
                    architecture: meta.and_then(|m| m.architecture.as_deref()),
                };
                let config_json = crate::ha_discovery::build_discovery_config(
                    topic_prefix,
                    tenant_id,
                    item.software_item_id,
                    host.host_id,
                    &item.name,
                    display_name(&host.friendly_name, &host.hostname),
                    crate::ha_discovery::ReleaseInfo {
                        url: host.release_url.as_deref(),
                        notes: host.release_notes.as_deref(),
                    },
                    os_info,
                );
                let config_bytes = config_json.to_string().into_bytes();
                publish_or_abort!(
                    state
                        .handle
                        .publish_retained(&config_topic, config_bytes)
                        .await,
                    mqtt_client_id,
                    "publish HA discovery config"
                );
            }
        }
    }

    /// Publish per-host package state topics for all hosts in `host_states`.
    ///
    /// For each host:
    /// - Publishes `{prefix}/hosts/{host_id}/state` (retained) — `"unknown"` or `"up-to-date"`
    /// - Publishes `{prefix}/hosts/{host_id}/latest_version` (retained) — `"{N} available"` or `"up-to-date"`
    /// - Publishes `{prefix}/hosts/{host_id}/attributes` (retained)
    /// - Subscribes to `{prefix}/hosts/{host_id}/set`
    /// - If `ha_discovery`: publishes HA discovery config (retained)
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    async fn publish_host_summary_states(
        &self,
        mqtt_client_id: uuid::Uuid,
        host_states: &[uptrakit_internal_wire::MqttHostSummary],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let tenant_id = state.tenant_id;
        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        // Build a host_id → metadata lookup for enriching HA discovery configs.
        let meta_map: std::collections::HashMap<
            uuid::Uuid,
            &uptrakit_internal_wire::MqttHostMetadata,
        > = self
            .host_metadata
            .get(&tenant_id)
            .map(|v| v.iter().map(|m| (m.host_id, m)).collect())
            .unwrap_or_default();

        for hs in host_states {
            // Publish per-host identity topics (hostname, friendly_name).
            self.publish_host_identity(state, hs.host_id, &hs.hostname, &hs.friendly_name)
                .await;

            // Compute state string: "unknown" or "up-to-date".
            let installed_str = crate::ha_discovery::host_packages_state_string(hs.pending_count);

            // Publish state topic.
            let st = crate::ha_discovery::host_packages_state_topic(topic_prefix, hs.host_id);
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&st, installed_str.into_bytes())
                    .await,
                mqtt_client_id,
                "publish host package state topic"
            );

            // Publish latest_version topic.
            let lt =
                crate::ha_discovery::host_packages_latest_version_topic(topic_prefix, hs.host_id);
            let latest_str =
                crate::ha_discovery::host_packages_latest_version_string(hs.pending_count);
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&lt, latest_str.into_bytes())
                    .await,
                mqtt_client_id,
                "publish host package latest_version topic"
            );

            // Publish JSON attributes.
            let at =
                crate::ha_discovery::host_packages_json_attributes_topic(topic_prefix, hs.host_id);
            let attributes_bytes = crate::ha_discovery::build_host_packages_attributes_payload(
                hs.update_in_progress,
                hs.pending_count,
                hs.total_count,
                hs.bugfix_count,
                hs.feature_count,
            )
            .to_string()
            .into_bytes();
            publish_or_abort!(
                state.handle.publish_retained(&at, attributes_bytes).await,
                mqtt_client_id,
                "publish host package attributes topic"
            );

            // Subscribe to command topic.
            let ct = crate::ha_discovery::host_packages_command_topic(topic_prefix, hs.host_id);
            publish_or_abort!(
                state.handle.subscribe_topic(&ct).await,
                mqtt_client_id,
                "subscribe to host package command topic"
            );

            // HA-only: publish HA discovery config for packages entity.
            if state.ha_discovery {
                let config_topic = crate::ha_discovery::host_packages_discovery_config_topic(
                    ha_prefix, tenant_id, hs.host_id,
                );
                let meta = meta_map.get(&hs.host_id);
                let os_info = crate::ha_discovery::HostOsInfo {
                    os_type: meta.and_then(|m| m.os_type.as_deref()),
                    os_version: meta.and_then(|m| m.os_version.as_deref()),
                    architecture: meta.and_then(|m| m.architecture.as_deref()),
                };
                let config_json = crate::ha_discovery::build_host_packages_discovery_config(
                    topic_prefix,
                    tenant_id,
                    hs.host_id,
                    display_name(&hs.friendly_name, &hs.hostname),
                    os_info,
                );
                let config_bytes = config_json.to_string().into_bytes();
                publish_or_abort!(
                    state
                        .handle
                        .publish_retained(&config_topic, config_bytes)
                        .await,
                    mqtt_client_id,
                    "publish host package HA discovery config"
                );
            }

            // Always: publish security entity state topic.
            let sec_state_str =
                crate::ha_discovery::host_security_state_string(hs.security_pending_count);
            let sec_st = crate::ha_discovery::host_security_state_topic(topic_prefix, hs.host_id);
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&sec_st, sec_state_str.into_bytes())
                    .await,
                mqtt_client_id,
                "publish host security state topic"
            );

            // Always: publish security entity latest_version topic.
            let sec_lt =
                crate::ha_discovery::host_security_latest_version_topic(topic_prefix, hs.host_id);
            let sec_latest_str =
                crate::ha_discovery::host_security_latest_version_string(hs.security_pending_count);
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&sec_lt, sec_latest_str.into_bytes())
                    .await,
                mqtt_client_id,
                "publish host security latest_version topic"
            );

            // Always: publish security entity JSON attributes.
            let sec_at =
                crate::ha_discovery::host_security_json_attributes_topic(topic_prefix, hs.host_id);
            let sec_attributes_bytes = crate::ha_discovery::build_host_security_attributes_payload(
                hs.update_in_progress,
                hs.security_pending_count,
            )
            .to_string()
            .into_bytes();
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&sec_at, sec_attributes_bytes)
                    .await,
                mqtt_client_id,
                "publish host security attributes topic"
            );

            // Always: subscribe to security command topic.
            let sec_ct = crate::ha_discovery::host_security_command_topic(topic_prefix, hs.host_id);
            publish_or_abort!(
                state.handle.subscribe_topic(&sec_ct).await,
                mqtt_client_id,
                "subscribe to host security command topic"
            );

            // HA-only: publish HA discovery config for security entity.
            if state.ha_discovery {
                let sec_config_topic = crate::ha_discovery::host_security_discovery_config_topic(
                    ha_prefix, tenant_id, hs.host_id,
                );
                let meta = meta_map.get(&hs.host_id);
                let os_info = crate::ha_discovery::HostOsInfo {
                    os_type: meta.and_then(|m| m.os_type.as_deref()),
                    os_version: meta.and_then(|m| m.os_version.as_deref()),
                    architecture: meta.and_then(|m| m.architecture.as_deref()),
                };
                let sec_config_json = crate::ha_discovery::build_host_security_discovery_config(
                    topic_prefix,
                    tenant_id,
                    hs.host_id,
                    display_name(&hs.friendly_name, &hs.hostname),
                    os_info,
                );
                let sec_config_bytes = sec_config_json.to_string().into_bytes();
                publish_or_abort!(
                    state
                        .handle
                        .publish_retained(&sec_config_topic, sec_config_bytes)
                        .await,
                    mqtt_client_id,
                    "publish host security HA discovery config"
                );
            }
        }
    }

    /// Republish only the HA discovery config topics for host package and
    /// security entities to an HA-enabled client.
    ///
    /// Used exclusively by [`handle_ha_online`](Self::handle_ha_online): when
    /// HA restarts, retained state/attributes topics are already on the broker
    /// and do not need re-sending. Only the discovery config topics need to be
    /// republished so that HA re-registers the `update` entities.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    async fn publish_host_summary_ha_configs_only(
        &self,
        mqtt_client_id: uuid::Uuid,
        host_states: &[uptrakit_internal_wire::MqttHostSummary],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };
        if !state.ha_discovery {
            return;
        }

        let tenant_id = state.tenant_id;
        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        // Build a host_id → metadata lookup for enriching HA discovery configs.
        let meta_map: std::collections::HashMap<
            uuid::Uuid,
            &uptrakit_internal_wire::MqttHostMetadata,
        > = self
            .host_metadata
            .get(&tenant_id)
            .map(|v| v.iter().map(|m| (m.host_id, m)).collect())
            .unwrap_or_default();

        for hs in host_states {
            // Packages entity config.
            let config_topic = crate::ha_discovery::host_packages_discovery_config_topic(
                ha_prefix, tenant_id, hs.host_id,
            );
            let meta = meta_map.get(&hs.host_id);
            let os_info = crate::ha_discovery::HostOsInfo {
                os_type: meta.and_then(|m| m.os_type.as_deref()),
                os_version: meta.and_then(|m| m.os_version.as_deref()),
                architecture: meta.and_then(|m| m.architecture.as_deref()),
            };
            let config_json = crate::ha_discovery::build_host_packages_discovery_config(
                topic_prefix,
                tenant_id,
                hs.host_id,
                display_name(&hs.friendly_name, &hs.hostname),
                os_info,
            );
            let config_bytes = config_json.to_string().into_bytes();
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&config_topic, config_bytes)
                    .await,
                mqtt_client_id,
                "publish host package HA discovery config"
            );

            // Security entity config.
            let sec_config_topic = crate::ha_discovery::host_security_discovery_config_topic(
                ha_prefix, tenant_id, hs.host_id,
            );
            let meta = meta_map.get(&hs.host_id);
            let os_info = crate::ha_discovery::HostOsInfo {
                os_type: meta.and_then(|m| m.os_type.as_deref()),
                os_version: meta.and_then(|m| m.os_version.as_deref()),
                architecture: meta.and_then(|m| m.architecture.as_deref()),
            };
            let sec_config_json = crate::ha_discovery::build_host_security_discovery_config(
                topic_prefix,
                tenant_id,
                hs.host_id,
                display_name(&hs.friendly_name, &hs.hostname),
                os_info,
            );
            let sec_config_bytes = sec_config_json.to_string().into_bytes();
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&sec_config_topic, sec_config_bytes)
                    .await,
                mqtt_client_id,
                "publish host security HA discovery config"
            );
        }
    }

    /// Publish per-host metadata topics (info, tags, agent) for all hosts in
    /// `metadata`.
    ///
    /// For each host:
    /// - Publishes `{prefix}/hosts/{host_id}/info` (retained) — OS info JSON
    /// - Publishes `{prefix}/hosts/{host_id}/tags` (retained) — JSON array of tags
    /// - Publishes `{prefix}/hosts/{host_id}/agent` (retained) — last_seen + version JSON
    ///
    /// Also publishes the HA connectivity `binary_sensor` discovery config for
    /// HA-enabled clients on first sight of a host (discovery config is idempotent
    /// to re-publish so we do it every time).
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    async fn publish_host_metadata(
        &self,
        mqtt_client_id: uuid::Uuid,
        metadata: &[uptrakit_internal_wire::MqttHostMetadata],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let topic_prefix = &state.topic_prefix;
        let tenant_id = state.tenant_id;
        let ha_prefix = &state.ha_discovery_prefix;

        for host in metadata {
            // Info topic.
            let info_topic = crate::ha_discovery::host_info_topic(topic_prefix, host.host_id);
            let info_bytes = crate::ha_discovery::build_host_info_payload(
                host.os_type.as_deref(),
                host.os_version.as_deref(),
                host.architecture.as_deref(),
            )
            .to_string()
            .into_bytes();
            publish_or_abort!(
                state.handle.publish_retained(&info_topic, info_bytes).await,
                mqtt_client_id,
                "publish host info topic"
            );

            // Tags topic.
            let tags_topic = crate::ha_discovery::host_tags_topic(topic_prefix, host.host_id);
            let tags_bytes = serde_json::to_string(&host.tags)
                .unwrap_or_else(|_| "[]".to_string())
                .into_bytes();
            publish_or_abort!(
                state.handle.publish_retained(&tags_topic, tags_bytes).await,
                mqtt_client_id,
                "publish host tags topic"
            );

            // Agent topic.
            let agent_topic = crate::ha_discovery::host_agent_topic(topic_prefix, host.host_id);
            let agent_bytes = crate::ha_discovery::build_host_agent_payload(
                host.agent_last_seen_at.as_deref(),
                host.agent_version.as_deref(),
            )
            .to_string()
            .into_bytes();
            publish_or_abort!(
                state
                    .handle
                    .publish_retained(&agent_topic, agent_bytes)
                    .await,
                mqtt_client_id,
                "publish host agent topic"
            );

            // HA-only: publish connectivity binary_sensor discovery config.
            if state.ha_discovery {
                let config_topic = crate::ha_discovery::host_connectivity_discovery_config_topic(
                    ha_prefix,
                    tenant_id,
                    host.host_id,
                );
                let config_json = crate::ha_discovery::build_host_connectivity_discovery_config(
                    topic_prefix,
                    tenant_id,
                    host.host_id,
                    display_name(&host.friendly_name, &host.hostname),
                    crate::ha_discovery::HostOsInfo {
                        os_type: host.os_type.as_deref(),
                        os_version: host.os_version.as_deref(),
                        architecture: host.architecture.as_deref(),
                    },
                );
                let config_bytes = config_json.to_string().into_bytes();
                publish_or_abort!(
                    state
                        .handle
                        .publish_retained(&config_topic, config_bytes)
                        .await,
                    mqtt_client_id,
                    "publish host connectivity HA discovery config"
                );
            }
        }
    }

    /// Publish connectivity state and attributes topics for a single host.
    ///
    /// If no entry exists in `connectivity_cache` for `(tenant_id, host_id)`,
    /// this method is a no-op (connectivity is unknown until the first
    /// `HostConnectivityUpdated` event arrives).
    #[tracing::instrument(skip_all, fields(%mqtt_client_id, %host_id))]
    async fn publish_connectivity_for_host(
        &self,
        mqtt_client_id: uuid::Uuid,
        tenant_id: Uuid,
        host_id: Uuid,
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };
        let Some(conn) = self.connectivity_cache.get(&(tenant_id, host_id)) else {
            return;
        };

        let topic_prefix = &state.topic_prefix;

        // State topic: "online" or "offline".
        let state_payload = if conn.online { "online" } else { "offline" };
        let state_topic = crate::ha_discovery::host_connectivity_state_topic(topic_prefix, host_id);
        publish_or_abort!(
            state
                .handle
                .publish_retained(&state_topic, state_payload.as_bytes().to_vec())
                .await,
            mqtt_client_id,
            "publish connectivity state topic"
        );

        // Attributes topic.
        let attr_topic =
            crate::ha_discovery::host_connectivity_attributes_topic(topic_prefix, host_id);
        let attr_bytes = crate::ha_discovery::build_host_connectivity_attributes_payload(
            conn.last_seen_at.as_deref(),
            conn.agent_version.as_deref(),
        )
        .to_string()
        .into_bytes();
        publish_or_abort!(
            state.handle.publish_retained(&attr_topic, attr_bytes).await,
            mqtt_client_id,
            "publish connectivity attributes topic"
        );
    }

    /// Publish only the HA connectivity `binary_sensor` discovery config for a
    /// single host.
    ///
    /// Looks up the friendly name from the host metadata cache; if absent,
    /// falls back to the host_id string so the topic is still published.
    /// Called from `handle_ha_online` to re-register entities after HA restarts.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id, %host_id))]
    async fn publish_connectivity_discovery_config(
        &self,
        mqtt_client_id: uuid::Uuid,
        tenant_id: Uuid,
        host_id: Uuid,
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };
        if !state.ha_discovery {
            return;
        }

        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        // Resolve friendly name and OS info from host metadata cache.
        let host_id_str = host_id.to_string();
        let metadata_entry = self
            .host_metadata
            .get(&tenant_id)
            .and_then(|hosts| hosts.iter().find(|h| h.host_id == host_id));
        let friendly_name: &str = metadata_entry
            .map(|h| display_name(h.friendly_name.as_str(), h.hostname.as_str()))
            .unwrap_or(host_id_str.as_str());
        let os_info = crate::ha_discovery::HostOsInfo {
            os_type: metadata_entry.and_then(|h| h.os_type.as_deref()),
            os_version: metadata_entry.and_then(|h| h.os_version.as_deref()),
            architecture: metadata_entry.and_then(|h| h.architecture.as_deref()),
        };

        let config_topic = crate::ha_discovery::host_connectivity_discovery_config_topic(
            ha_prefix, tenant_id, host_id,
        );
        let config_json = crate::ha_discovery::build_host_connectivity_discovery_config(
            topic_prefix,
            tenant_id,
            host_id,
            friendly_name,
            os_info,
        );
        let config_bytes = config_json.to_string().into_bytes();
        publish_or_abort!(
            state
                .handle
                .publish_retained(&config_topic, config_bytes)
                .await,
            mqtt_client_id,
            "publish connectivity HA discovery config"
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

/// Build MqttConfig from wire protocol config.
fn build_config_from_wire(config: &MqttTenantConfig) -> MqttConfig {
    // Wire and MqttConfig now use the same canonical MqttTransport type.
    let transport = config.transport;
    let port = if config.port == 0 {
        transport.default_port()
    } else {
        config.port
    };

    MqttConfig {
        transport,
        host: config.host.clone(),
        port,
        client_id: config.client_id.clone(),
        username: config.username.clone(),
        password: config.password.clone(),
        ca_pem: config.ca_pem.clone(),
        topic_prefix: config.topic_prefix.clone(),
    }
}

/// Return the display name for a host, falling back to hostname when friendly_name is empty.
///
/// Home Assistant shows the `name` field of the device block. When `friendly_name` has
/// not been set by the user it is an empty string (the `serde` default). In that case
/// we fall back to `hostname` so HA shows something meaningful instead of a blank name.
fn display_name<'a>(friendly_name: &'a str, hostname: &'a str) -> &'a str {
    if friendly_name.is_empty() {
        hostname
    } else {
        friendly_name
    }
}

/// Compute a hash of the config for change detection.
///
/// Uses `DefaultHasher` (SipHash with a per-process random seed), so
/// hashes are only valid within the same process lifetime. This is
/// correct for the intended use: detecting config changes between
/// consecutive `TenantAssignments` messages during a single service
/// run. Hashes are not persisted or compared across process restarts.
fn compute_config_hash(config: &MqttTenantConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.transport.hash(&mut hasher);
    config.host.hash(&mut hasher);
    config.port.hash(&mut hasher);
    config.client_id.hash(&mut hasher);
    config.username.hash(&mut hasher);
    config.password.hash(&mut hasher);
    config.ca_pem.hash(&mut hasher);
    config.topic_prefix.hash(&mut hasher);
    config.ha_discovery.hash(&mut hasher);
    config.ha_discovery_prefix.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
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
            username: Some(SecretString::new("user".into())),
            password: Some(SecretString::new("pass".into())),
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
                    hosts: vec![],
                },
                uptrakit_internal_wire::MqttSoftwareStateItem {
                    software_item_id: Uuid::from_u128(1),
                    name: "redis".to_string(),
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
}
