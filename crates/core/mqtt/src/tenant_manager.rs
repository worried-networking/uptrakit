use std::collections::HashMap;

use futures_util::stream::{FuturesUnordered, StreamExt};
use uptrakit_internal_wire::MqttTenantConfig;
use uuid::Uuid;

use crate::mqtt_client::{MqttConfig, MqttHandle, MqttServiceEvent};
use tokio::sync::mpsc;
use uptrakit_internal_wire::MqttClientConnectionStatus;

/// Tracks the cached state for an MQTT client.
struct ClientState {
    handle: MqttHandle,
    config_hash: u64,
    tenant_id: uuid::Uuid,
    topic_prefix: String,
    ha_discovery: bool,
    ha_discovery_prefix: String,
}

/// Manages per-MQTT-client lifecycles with push-based config updates.
///
/// Unlike the database-polling version, this manager receives configuration
/// updates from the controller via WebSocket messages.
pub struct TenantManager {
    clients: HashMap<Uuid, ClientState>,
    event_tx: Option<mpsc::Sender<MqttServiceEvent>>,
    software_states: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttSoftwareStateItem>>,
    /// Cached per-host package states, keyed by tenant_id.
    host_package_states: HashMap<Uuid, Vec<uptrakit_internal_wire::MqttHostPackageHostState>>,
}

impl TenantManager {
    pub fn new(event_tx: Option<mpsc::Sender<MqttServiceEvent>>) -> Self {
        Self {
            clients: HashMap::new(),
            event_tx,
            software_states: HashMap::new(),
            host_package_states: HashMap::new(),
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
        self.software_states
            .insert(payload.tenant_id, payload.items.clone());
        self.host_package_states
            .insert(payload.tenant_id, payload.host_package_hosts.clone());

        // Collect client IDs for this tenant (all of them, not just HA-enabled).
        let client_ids: Vec<uuid::Uuid> = self
            .clients
            .iter()
            .filter(|(_, s)| s.tenant_id == payload.tenant_id)
            .map(|(id, _)| *id)
            .collect();

        for client_id in client_ids {
            self.publish_software_states(client_id, &payload.items)
                .await;
            self.publish_host_package_states(client_id, &payload.host_package_hosts)
                .await;
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
        if let Some(host_states) = self.host_package_states.get(&tenant_id).cloned() {
            self.publish_host_package_states(*mqtt_client_id, &host_states)
                .await;
        }
    }

    /// Called when HA sends its birth message (restarted): republish only HA
    /// discovery config topics.
    ///
    /// State and version topics are retained on the broker and do not need
    /// re-sending after an HA restart. Only the `{ha_prefix}/update/.../config`
    /// messages need to be republished so that HA re-registers its `update`
    /// entities.
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
        if let Some(host_states) = self.host_package_states.get(&tenant_id).cloned() {
            self.publish_host_package_ha_configs_only(*mqtt_client_id, &host_states)
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
    /// [`MqttTriggerHostPackageUpdatePayload`](uptrakit_internal_wire::MqttTriggerHostPackageUpdatePayload).
    ///
    /// Returns `None` if the topic doesn't match the host-packages command
    /// pattern `{prefix}/hosts/{host_id}/set`.
    pub fn resolve_host_package_update_trigger(
        &self,
        mqtt_client_id: uuid::Uuid,
        topic: &str,
    ) -> Option<uptrakit_internal_wire::MqttTriggerHostPackageUpdatePayload> {
        let state = self.clients.get(&mqtt_client_id)?;
        let host_id =
            crate::ha_discovery::parse_host_packages_command_topic(&state.topic_prefix, topic)?;
        Some(
            uptrakit_internal_wire::MqttTriggerHostPackageUpdatePayload {
                tenant_id: state.tenant_id,
                host_id,
                mqtt_client_id,
                security_only: false,
            },
        )
    }

    /// Given an inbound MQTT security-entity command topic, resolve it to an
    /// [`MqttTriggerHostPackageUpdatePayload`](uptrakit_internal_wire::MqttTriggerHostPackageUpdatePayload)
    /// with `security_only = true`.
    ///
    /// Returns `None` if the topic doesn't match the security command
    /// pattern `{prefix}/hosts/{host_id}/security/set`.
    pub fn resolve_host_security_update_trigger(
        &self,
        mqtt_client_id: uuid::Uuid,
        topic: &str,
    ) -> Option<uptrakit_internal_wire::MqttTriggerHostPackageUpdatePayload> {
        let state = self.clients.get(&mqtt_client_id)?;
        let host_id =
            crate::ha_discovery::parse_host_security_command_topic(&state.topic_prefix, topic)?;
        Some(
            uptrakit_internal_wire::MqttTriggerHostPackageUpdatePayload {
                tenant_id: state.tenant_id,
                host_id,
                mqtt_client_id,
                security_only: true,
            },
        )
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

        for item in items {
            for host in &item.hosts {
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
                if let Err(e) = state.handle.publish_retained(&st, installed).await {
                    tracing::warn!(
                        error = ?e,
                        %mqtt_client_id,
                        "failed to publish state topic"
                    );
                }

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
                if let Err(e) = state.handle.publish_retained(&lt, latest).await {
                    tracing::warn!(
                        error = ?e,
                        %mqtt_client_id,
                        "failed to publish latest version topic"
                    );
                }

                // Always: subscribe to command topic.
                let ct = crate::ha_discovery::command_topic(
                    topic_prefix,
                    item.software_item_id,
                    host.host_id,
                );
                if let Err(e) = state.handle.subscribe_topic(&ct).await {
                    tracing::warn!(
                        error = ?e,
                        %mqtt_client_id,
                        "failed to subscribe to command topic"
                    );
                }

                // Always: publish JSON attributes (in_progress flag).
                let at = crate::ha_discovery::json_attributes_topic(
                    topic_prefix,
                    item.software_item_id,
                    host.host_id,
                );
                let attributes_bytes =
                    crate::ha_discovery::build_attributes_payload(host.update_in_progress)
                        .to_string()
                        .into_bytes();
                if let Err(e) = state.handle.publish_retained(&at, attributes_bytes).await {
                    tracing::warn!(
                        error = ?e,
                        %mqtt_client_id,
                        "failed to publish JSON attributes topic"
                    );
                }

                // HA-only: publish HA discovery config so HA creates an update entity.
                if state.ha_discovery {
                    let uid = crate::ha_discovery::unique_id(
                        tenant_id,
                        item.software_item_id,
                        host.host_id,
                    );
                    let config_topic = crate::ha_discovery::discovery_config_topic(ha_prefix, &uid);
                    let config_json = crate::ha_discovery::build_discovery_config(
                        topic_prefix,
                        tenant_id,
                        item.software_item_id,
                        host.host_id,
                        &item.name,
                        &host.hostname,
                        crate::ha_discovery::ReleaseInfo {
                            url: host.release_url.as_deref(),
                            notes: host.release_notes.as_deref(),
                        },
                    );
                    let config_bytes = config_json.to_string().into_bytes();
                    if let Err(e) = state
                        .handle
                        .publish_retained(&config_topic, config_bytes)
                        .await
                    {
                        tracing::warn!(
                            error = ?e,
                            %mqtt_client_id,
                            "failed to publish HA discovery config"
                        );
                    }
                }
            }
        }
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

        for item in items {
            for host in &item.hosts {
                let uid =
                    crate::ha_discovery::unique_id(tenant_id, item.software_item_id, host.host_id);
                let config_topic = crate::ha_discovery::discovery_config_topic(ha_prefix, &uid);
                let config_json = crate::ha_discovery::build_discovery_config(
                    topic_prefix,
                    tenant_id,
                    item.software_item_id,
                    host.host_id,
                    &item.name,
                    &host.hostname,
                    crate::ha_discovery::ReleaseInfo {
                        url: host.release_url.as_deref(),
                        notes: host.release_notes.as_deref(),
                    },
                );
                let config_bytes = config_json.to_string().into_bytes();
                if let Err(e) = state
                    .handle
                    .publish_retained(&config_topic, config_bytes)
                    .await
                {
                    tracing::warn!(
                        error = ?e,
                        %mqtt_client_id,
                        "failed to publish HA discovery config"
                    );
                }
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
    async fn publish_host_package_states(
        &self,
        mqtt_client_id: uuid::Uuid,
        host_states: &[uptrakit_internal_wire::MqttHostPackageHostState],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let tenant_id = state.tenant_id;
        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        for hs in host_states {
            // Compute state string: "unknown" or "up-to-date".
            let installed_str = crate::ha_discovery::host_packages_state_string(hs.pending_count);

            // Publish state topic.
            let st = crate::ha_discovery::host_packages_state_topic(topic_prefix, hs.host_id);
            if let Err(e) = state
                .handle
                .publish_retained(&st, installed_str.into_bytes())
                .await
            {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host package state topic"
                );
            }

            // Publish latest_version topic.
            let lt =
                crate::ha_discovery::host_packages_latest_version_topic(topic_prefix, hs.host_id);
            let latest_str =
                crate::ha_discovery::host_packages_latest_version_string(hs.pending_count);
            if let Err(e) = state
                .handle
                .publish_retained(&lt, latest_str.into_bytes())
                .await
            {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host package latest_version topic"
                );
            }

            // Publish JSON attributes.
            let at =
                crate::ha_discovery::host_packages_json_attributes_topic(topic_prefix, hs.host_id);
            let attributes_bytes = crate::ha_discovery::build_host_packages_attributes_payload(
                hs.update_in_progress,
                hs.pending_count,
            )
            .to_string()
            .into_bytes();
            if let Err(e) = state.handle.publish_retained(&at, attributes_bytes).await {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host package attributes topic"
                );
            }

            // Subscribe to command topic.
            let ct = crate::ha_discovery::host_packages_command_topic(topic_prefix, hs.host_id);
            if let Err(e) = state.handle.subscribe_topic(&ct).await {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to subscribe to host package command topic"
                );
            }

            // HA-only: publish HA discovery config for packages entity.
            if state.ha_discovery {
                let config_topic = crate::ha_discovery::host_packages_discovery_config_topic(
                    ha_prefix, tenant_id, hs.host_id,
                );
                let config_json = crate::ha_discovery::build_host_packages_discovery_config(
                    topic_prefix,
                    tenant_id,
                    hs.host_id,
                    &hs.hostname,
                );
                let config_bytes = config_json.to_string().into_bytes();
                if let Err(e) = state
                    .handle
                    .publish_retained(&config_topic, config_bytes)
                    .await
                {
                    tracing::warn!(
                        error = ?e,
                        %mqtt_client_id,
                        host_id = %hs.host_id,
                        "failed to publish host package HA discovery config"
                    );
                }
            }

            // Always: publish security entity state topic.
            let sec_state_str =
                crate::ha_discovery::host_security_state_string(hs.security_pending_count);
            let sec_st = crate::ha_discovery::host_security_state_topic(topic_prefix, hs.host_id);
            if let Err(e) = state
                .handle
                .publish_retained(&sec_st, sec_state_str.into_bytes())
                .await
            {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host security state topic"
                );
            }

            // Always: publish security entity latest_version topic.
            let sec_lt =
                crate::ha_discovery::host_security_latest_version_topic(topic_prefix, hs.host_id);
            let sec_latest_str =
                crate::ha_discovery::host_security_latest_version_string(hs.security_pending_count);
            if let Err(e) = state
                .handle
                .publish_retained(&sec_lt, sec_latest_str.into_bytes())
                .await
            {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host security latest_version topic"
                );
            }

            // Always: publish security entity JSON attributes.
            let sec_at =
                crate::ha_discovery::host_security_json_attributes_topic(topic_prefix, hs.host_id);
            let sec_attributes_bytes = crate::ha_discovery::build_host_packages_attributes_payload(
                hs.update_in_progress,
                hs.security_pending_count,
            )
            .to_string()
            .into_bytes();
            if let Err(e) = state
                .handle
                .publish_retained(&sec_at, sec_attributes_bytes)
                .await
            {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host security attributes topic"
                );
            }

            // Always: subscribe to security command topic.
            let sec_ct = crate::ha_discovery::host_security_command_topic(topic_prefix, hs.host_id);
            if let Err(e) = state.handle.subscribe_topic(&sec_ct).await {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to subscribe to host security command topic"
                );
            }

            // HA-only: publish HA discovery config for security entity.
            if state.ha_discovery {
                let sec_config_topic = crate::ha_discovery::host_security_discovery_config_topic(
                    ha_prefix, tenant_id, hs.host_id,
                );
                let sec_config_json = crate::ha_discovery::build_host_security_discovery_config(
                    topic_prefix,
                    tenant_id,
                    hs.host_id,
                    &hs.hostname,
                );
                let sec_config_bytes = sec_config_json.to_string().into_bytes();
                if let Err(e) = state
                    .handle
                    .publish_retained(&sec_config_topic, sec_config_bytes)
                    .await
                {
                    tracing::warn!(
                        error = ?e,
                        %mqtt_client_id,
                        host_id = %hs.host_id,
                        "failed to publish host security HA discovery config"
                    );
                }
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
    async fn publish_host_package_ha_configs_only(
        &self,
        mqtt_client_id: uuid::Uuid,
        host_states: &[uptrakit_internal_wire::MqttHostPackageHostState],
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

        for hs in host_states {
            // Packages entity config.
            let config_topic = crate::ha_discovery::host_packages_discovery_config_topic(
                ha_prefix, tenant_id, hs.host_id,
            );
            let config_json = crate::ha_discovery::build_host_packages_discovery_config(
                topic_prefix,
                tenant_id,
                hs.host_id,
                &hs.hostname,
            );
            let config_bytes = config_json.to_string().into_bytes();
            if let Err(e) = state
                .handle
                .publish_retained(&config_topic, config_bytes)
                .await
            {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host package HA discovery config"
                );
            }

            // Security entity config.
            let sec_config_topic = crate::ha_discovery::host_security_discovery_config_topic(
                ha_prefix, tenant_id, hs.host_id,
            );
            let sec_config_json = crate::ha_discovery::build_host_security_discovery_config(
                topic_prefix,
                tenant_id,
                hs.host_id,
                &hs.hostname,
            );
            let sec_config_bytes = sec_config_json.to_string().into_bytes();
            if let Err(e) = state
                .handle
                .publish_retained(&sec_config_topic, sec_config_bytes)
                .await
            {
                tracing::warn!(
                    error = ?e,
                    %mqtt_client_id,
                    host_id = %hs.host_id,
                    "failed to publish host security HA discovery config"
                );
            }
        }
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
            host_package_hosts: vec![],
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
            host_package_hosts: vec![],
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
            host_package_hosts: vec![],
        };
        manager.update_software_states(second).await;

        assert_eq!(manager.software_states[&tenant_id].len(), 2);
        assert_eq!(manager.software_states[&tenant_id][1].name, "redis");
    }
}
