use std::collections::HashSet;

use uuid::Uuid;

use crate::client_manager::ClientState;
use crate::tenant_manager::TenantManager;

impl TenantManager {
    /// Build a `host_id → HostStateMetadata` lookup map for the given tenant.
    pub(crate) fn build_meta_map(
        &self,
        tenant_id: uuid::Uuid,
    ) -> std::collections::HashMap<uuid::Uuid, &uptrakit_internal_wire::HostStateMetadata> {
        self.host_metadata
            .get(&tenant_id)
            .map(|v| v.iter().map(|m| (m.host_id, m)).collect())
            .unwrap_or_default()
    }

    /// Construct a [`HostOsInfo`] from the metadata map for a given host.
    pub(crate) fn os_info_from_meta<'a>(
        meta_map: &'a std::collections::HashMap<
            uuid::Uuid,
            &'a uptrakit_internal_wire::HostStateMetadata,
        >,
        host_id: uuid::Uuid,
    ) -> crate::ha_discovery::HostOsInfo<'a> {
        let meta = meta_map.get(&host_id);
        crate::ha_discovery::HostOsInfo {
            os_type: meta.and_then(|m| m.os_type.as_deref()),
            os_version: meta.and_then(|m| m.os_version.as_deref()),
            architecture: meta.and_then(|m| m.architecture.as_deref()),
        }
    }

    /// Publish the HA discovery config topic for a single software item×host pair.
    ///
    /// Shared by [`publish_software_states`] and [`publish_ha_configs_only`].
    pub(crate) async fn publish_item_ha_config(
        state: &ClientState,
        mqtt_client_id: uuid::Uuid,
        item: &uptrakit_internal_wire::SoftwareStateItem,
        host: &uptrakit_internal_wire::SoftwareStateHostEntry,
        meta_map: &std::collections::HashMap<
            uuid::Uuid,
            &uptrakit_internal_wire::HostStateMetadata,
        >,
    ) {
        let uid =
            crate::ha_discovery::unique_id(state.tenant_id, item.software_item_id, host.host_id);
        let config_topic =
            crate::ha_discovery::discovery_config_topic(&state.ha_discovery_prefix, &uid);
        let os_info = Self::os_info_from_meta(meta_map, host.host_id);
        let config_json = crate::ha_discovery::build_discovery_config(
            &state.topic_prefix,
            state.tenant_id,
            item.software_item_id,
            host.host_id,
            &item.name,
            display_name(&host.friendly_name, &host.hostname),
            crate::ha_discovery::ReleaseInfo {
                url: host.release_url.as_deref(),
                notes: host.release_notes.as_deref(),
                icon_url: item.icon_url.as_deref(),
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

    /// Publish HA discovery config topics for a host summary (packages + security entities).
    ///
    /// Shared by [`publish_host_summary_states`] and [`publish_host_summary_ha_configs_only`].
    pub(crate) async fn publish_host_summary_ha_configs(
        state: &ClientState,
        mqtt_client_id: uuid::Uuid,
        hs: &uptrakit_internal_wire::HostPackageSummary,
        meta_map: &std::collections::HashMap<
            uuid::Uuid,
            &uptrakit_internal_wire::HostStateMetadata,
        >,
    ) {
        // Packages entity config.
        let config_topic = crate::ha_discovery::host_packages_discovery_config_topic(
            &state.ha_discovery_prefix,
            state.tenant_id,
            hs.host_id,
        );
        let os_info = Self::os_info_from_meta(meta_map, hs.host_id);
        let config_json = crate::ha_discovery::build_host_packages_discovery_config(
            &state.topic_prefix,
            state.tenant_id,
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
            &state.ha_discovery_prefix,
            state.tenant_id,
            hs.host_id,
        );
        let os_info = Self::os_info_from_meta(meta_map, hs.host_id);
        let sec_config_json = crate::ha_discovery::build_host_security_discovery_config(
            &state.topic_prefix,
            state.tenant_id,
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

    /// Publish software state topics and subscribe to command topics for all
    /// `(item, host)` pairs, then publish HA discovery config topics for clients
    /// that have `ha_discovery` enabled.
    ///
    /// Also publishes per-host `hostname` and `friendly_name` topics (retained)
    /// for MQTT explorer visibility.
    ///
    /// Called on every `SoftwareStates` push and on broker reconnect.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    pub(crate) async fn publish_software_states(
        &self,
        mqtt_client_id: uuid::Uuid,
        items: &[uptrakit_internal_wire::SoftwareStateItem],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let meta_map = self.build_meta_map(state.tenant_id);

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

                Self::publish_item_state_topics(state, mqtt_client_id, item, host).await;

                if state.ha_discovery {
                    Self::publish_item_ha_config(state, mqtt_client_id, item, host, &meta_map)
                        .await;
                }
            }
        }
    }

    /// Publish state, latest_version, command subscription, and attributes for
    /// a single software item×host pair.
    pub(crate) async fn publish_item_state_topics(
        state: &ClientState,
        mqtt_client_id: uuid::Uuid,
        item: &uptrakit_internal_wire::SoftwareStateItem,
        host: &uptrakit_internal_wire::SoftwareStateHostEntry,
    ) {
        let topic_prefix = &state.topic_prefix;

        // Publish installed version (empty string if unknown).
        let st =
            crate::ha_discovery::state_topic(topic_prefix, item.software_item_id, host.host_id);
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

        // Publish latest version.
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

        // Subscribe to command topic.
        let ct =
            crate::ha_discovery::command_topic(topic_prefix, item.software_item_id, host.host_id);
        publish_or_abort!(
            state.handle.subscribe_topic(&ct).await,
            mqtt_client_id,
            "subscribe to command topic"
        );

        // Publish JSON attributes.
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
    }

    /// Publish retained `hostname` and `friendly_name` topics for a host.
    ///
    /// These topics are for MQTT explorer visibility and are published under
    /// `{prefix}/hosts/{host_id}/hostname` and `{prefix}/hosts/{host_id}/friendly_name`.
    pub(crate) async fn publish_host_identity(
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
    pub(crate) async fn publish_ha_configs_only(
        &self,
        mqtt_client_id: uuid::Uuid,
        items: &[uptrakit_internal_wire::SoftwareStateItem],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };
        if !state.ha_discovery {
            return;
        }

        let meta_map = self.build_meta_map(state.tenant_id);

        for item in items {
            for host in &item.hosts {
                Self::publish_item_ha_config(state, mqtt_client_id, item, host, &meta_map).await;
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
    pub(crate) async fn publish_host_summary_states(
        &self,
        mqtt_client_id: uuid::Uuid,
        host_states: &[uptrakit_internal_wire::HostPackageSummary],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let meta_map = self.build_meta_map(state.tenant_id);

        for hs in host_states {
            // Publish per-host identity topics (hostname, friendly_name).
            self.publish_host_identity(state, hs.host_id, &hs.hostname, &hs.friendly_name)
                .await;

            Self::publish_host_summary_state_topics(state, mqtt_client_id, hs).await;

            if state.ha_discovery {
                Self::publish_host_summary_ha_configs(state, mqtt_client_id, hs, &meta_map).await;
            }
        }
    }

    /// Publish state, latest_version, attributes, and command subscription topics
    /// for packages and security entities of a single host summary.
    pub(crate) async fn publish_host_summary_state_topics(
        state: &ClientState,
        mqtt_client_id: uuid::Uuid,
        hs: &uptrakit_internal_wire::HostPackageSummary,
    ) {
        let topic_prefix = &state.topic_prefix;

        // Packages: state topic.
        let installed_str = crate::ha_discovery::host_packages_state_string(hs.pending_count);
        let st = crate::ha_discovery::host_packages_state_topic(topic_prefix, hs.host_id);
        publish_or_abort!(
            state
                .handle
                .publish_retained(&st, installed_str.into_bytes())
                .await,
            mqtt_client_id,
            "publish host package state topic"
        );

        // Packages: latest_version topic.
        let lt = crate::ha_discovery::host_packages_latest_version_topic(topic_prefix, hs.host_id);
        let latest_str = crate::ha_discovery::host_packages_latest_version_string(hs.pending_count);
        publish_or_abort!(
            state
                .handle
                .publish_retained(&lt, latest_str.into_bytes())
                .await,
            mqtt_client_id,
            "publish host package latest_version topic"
        );

        // Packages: JSON attributes.
        let at = crate::ha_discovery::host_packages_json_attributes_topic(topic_prefix, hs.host_id);
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

        // Packages: command subscription.
        let ct = crate::ha_discovery::host_packages_command_topic(topic_prefix, hs.host_id);
        publish_or_abort!(
            state.handle.subscribe_topic(&ct).await,
            mqtt_client_id,
            "subscribe to host package command topic"
        );

        // Security: state topic.
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

        // Security: latest_version topic.
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

        // Security: JSON attributes.
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

        // Security: command subscription.
        let sec_ct = crate::ha_discovery::host_security_command_topic(topic_prefix, hs.host_id);
        publish_or_abort!(
            state.handle.subscribe_topic(&sec_ct).await,
            mqtt_client_id,
            "subscribe to host security command topic"
        );
    }

    /// Republish only the HA discovery config topics for host package and
    /// security entities to an HA-enabled client.
    ///
    /// Used exclusively by [`handle_ha_online`](Self::handle_ha_online): when
    /// HA restarts, retained state/attributes topics are already on the broker
    /// and do not need re-sending. Only the discovery config topics need to be
    /// republished so that HA re-registers the `update` entities.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id))]
    pub(crate) async fn publish_host_summary_ha_configs_only(
        &self,
        mqtt_client_id: uuid::Uuid,
        host_states: &[uptrakit_internal_wire::HostPackageSummary],
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };
        if !state.ha_discovery {
            return;
        }

        let meta_map = self.build_meta_map(state.tenant_id);

        for hs in host_states {
            Self::publish_host_summary_ha_configs(state, mqtt_client_id, hs, &meta_map).await;
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
    pub(crate) async fn publish_host_metadata(
        &self,
        mqtt_client_id: uuid::Uuid,
        metadata: &[uptrakit_internal_wire::HostStateMetadata],
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
    pub(crate) async fn publish_connectivity_for_host(
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
    pub(crate) async fn publish_connectivity_discovery_config(
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

    /// Clean up MQTT topics for `(item_id, host_id)` pairs that are no longer
    /// present in the new software states payload.
    ///
    /// Publishes empty retained payloads (which deletes the retained message
    /// from the broker) for state, latest_version, attributes, and HA discovery
    /// config topics. Best-effort unsubscribes from command topics.
    #[tracing::instrument(skip_all, fields(%mqtt_client_id, removed_count = removed.len()))]
    pub(crate) async fn cleanup_removed_items(
        &self,
        mqtt_client_id: Uuid,
        tenant_id: Uuid,
        removed: &HashSet<(Uuid, Uuid)>,
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        for &(item_id, host_id) in removed {
            tracing::debug!(%item_id, %host_id, "cleaning up removed item topics");

            // Delete retained state/version/attributes topics.
            let st = crate::ha_discovery::state_topic(topic_prefix, item_id, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&st, Vec::new()).await,
                mqtt_client_id,
                "clear state topic"
            );

            let lt = crate::ha_discovery::latest_version_topic(topic_prefix, item_id, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&lt, Vec::new()).await,
                mqtt_client_id,
                "clear latest_version topic"
            );

            let at = crate::ha_discovery::json_attributes_topic(topic_prefix, item_id, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&at, Vec::new()).await,
                mqtt_client_id,
                "clear attributes topic"
            );

            // Unsubscribe from command topic.
            let ct = crate::ha_discovery::command_topic(topic_prefix, item_id, host_id);
            publish_best_effort!(
                state.handle.unsubscribe_topic(&ct).await,
                mqtt_client_id,
                "unsubscribe from command topic"
            );

            // HA-only: clear discovery config.
            if state.ha_discovery {
                let uid = crate::ha_discovery::unique_id(tenant_id, item_id, host_id);
                let config_topic = crate::ha_discovery::discovery_config_topic(ha_prefix, &uid);
                publish_best_effort!(
                    state
                        .handle
                        .publish_retained(&config_topic, Vec::new())
                        .await,
                    mqtt_client_id,
                    "clear HA discovery config topic"
                );
            }
        }
    }

    /// Clean up MQTT topics for hosts that are no longer present in the new
    /// host summary states payload (packages + security entities).
    #[tracing::instrument(skip_all, fields(%mqtt_client_id, removed_count = removed.len()))]
    pub(crate) async fn cleanup_removed_host_summaries(
        &self,
        mqtt_client_id: Uuid,
        tenant_id: Uuid,
        removed: &HashSet<Uuid>,
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        for &host_id in removed {
            tracing::debug!(%host_id, "cleaning up removed host summary topics");

            // Packages entity.
            let st = crate::ha_discovery::host_packages_state_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&st, Vec::new()).await,
                mqtt_client_id,
                "clear host packages state topic"
            );
            let lt = crate::ha_discovery::host_packages_latest_version_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&lt, Vec::new()).await,
                mqtt_client_id,
                "clear host packages latest_version topic"
            );
            let at =
                crate::ha_discovery::host_packages_json_attributes_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&at, Vec::new()).await,
                mqtt_client_id,
                "clear host packages attributes topic"
            );
            let ct = crate::ha_discovery::host_packages_command_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.unsubscribe_topic(&ct).await,
                mqtt_client_id,
                "unsubscribe from host packages command topic"
            );

            // Security entity.
            let sec_st = crate::ha_discovery::host_security_state_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&sec_st, Vec::new()).await,
                mqtt_client_id,
                "clear host security state topic"
            );
            let sec_lt =
                crate::ha_discovery::host_security_latest_version_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&sec_lt, Vec::new()).await,
                mqtt_client_id,
                "clear host security latest_version topic"
            );
            let sec_at =
                crate::ha_discovery::host_security_json_attributes_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&sec_at, Vec::new()).await,
                mqtt_client_id,
                "clear host security attributes topic"
            );
            let sec_ct = crate::ha_discovery::host_security_command_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.unsubscribe_topic(&sec_ct).await,
                mqtt_client_id,
                "unsubscribe from host security command topic"
            );

            // HA-only: clear discovery configs.
            if state.ha_discovery {
                let pkgs_config = crate::ha_discovery::host_packages_discovery_config_topic(
                    ha_prefix, tenant_id, host_id,
                );
                publish_best_effort!(
                    state
                        .handle
                        .publish_retained(&pkgs_config, Vec::new())
                        .await,
                    mqtt_client_id,
                    "clear host packages HA discovery config"
                );

                let sec_config = crate::ha_discovery::host_security_discovery_config_topic(
                    ha_prefix, tenant_id, host_id,
                );
                publish_best_effort!(
                    state.handle.publish_retained(&sec_config, Vec::new()).await,
                    mqtt_client_id,
                    "clear host security HA discovery config"
                );
            }
        }
    }

    /// Clean up MQTT topics for hosts that are no longer present in the new
    /// host metadata payload (hostname, friendly_name, info, tags, agent,
    /// connectivity).
    #[tracing::instrument(skip_all, fields(%mqtt_client_id, removed_count = removed.len()))]
    pub(crate) async fn cleanup_removed_host_metadata(
        &self,
        mqtt_client_id: Uuid,
        tenant_id: Uuid,
        removed: &HashSet<Uuid>,
    ) {
        let Some(state) = self.clients.get(&mqtt_client_id) else {
            return;
        };

        let topic_prefix = &state.topic_prefix;
        let ha_prefix = &state.ha_discovery_prefix;

        for &host_id in removed {
            tracing::debug!(%host_id, "cleaning up removed host metadata topics");

            // Identity topics.
            let hn = crate::ha_discovery::hostname_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&hn, Vec::new()).await,
                mqtt_client_id,
                "clear hostname topic"
            );
            let fn_topic = crate::ha_discovery::friendly_name_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&fn_topic, Vec::new()).await,
                mqtt_client_id,
                "clear friendly_name topic"
            );

            // Metadata topics.
            let info = crate::ha_discovery::host_info_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&info, Vec::new()).await,
                mqtt_client_id,
                "clear host info topic"
            );
            let tags = crate::ha_discovery::host_tags_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&tags, Vec::new()).await,
                mqtt_client_id,
                "clear host tags topic"
            );
            let agent = crate::ha_discovery::host_agent_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&agent, Vec::new()).await,
                mqtt_client_id,
                "clear host agent topic"
            );

            // Connectivity topics.
            let conn_st = crate::ha_discovery::host_connectivity_state_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&conn_st, Vec::new()).await,
                mqtt_client_id,
                "clear connectivity state topic"
            );
            let conn_at =
                crate::ha_discovery::host_connectivity_attributes_topic(topic_prefix, host_id);
            publish_best_effort!(
                state.handle.publish_retained(&conn_at, Vec::new()).await,
                mqtt_client_id,
                "clear connectivity attributes topic"
            );

            // HA-only: clear connectivity discovery config.
            if state.ha_discovery {
                let conn_config = crate::ha_discovery::host_connectivity_discovery_config_topic(
                    ha_prefix, tenant_id, host_id,
                );
                publish_best_effort!(
                    state
                        .handle
                        .publish_retained(&conn_config, Vec::new())
                        .await,
                    mqtt_client_id,
                    "clear connectivity HA discovery config"
                );
            }
        }
    }
}

/// Return the display name for a host, falling back to hostname when `friendly_name` is empty.
///
/// Home Assistant shows the `name` field of the device block. When `friendly_name` has
/// not been set by the user it is an empty string (the `serde` default). In that case
/// we fall back to `hostname` so HA shows something meaningful instead of a blank name.
pub(crate) fn display_name<'a>(friendly_name: &'a str, hostname: &'a str) -> &'a str {
    if friendly_name.is_empty() {
        hostname
    } else {
        friendly_name
    }
}

/// Compute the set of `(item_id, host_id)` pairs present in `old` but absent
/// from `new`.
///
/// Returns an empty set when `old` is `None` (first update — nothing to remove).
pub(crate) fn compute_removed_items(
    old: Option<&Vec<uptrakit_internal_wire::SoftwareStateItem>>,
    new: &[uptrakit_internal_wire::SoftwareStateItem],
) -> HashSet<(Uuid, Uuid)> {
    let Some(old_items) = old else {
        return HashSet::new();
    };
    let old_set: HashSet<(Uuid, Uuid)> = old_items
        .iter()
        .flat_map(|item| {
            item.hosts
                .iter()
                .map(move |h| (item.software_item_id, h.host_id))
        })
        .collect();
    let new_set: HashSet<(Uuid, Uuid)> = new
        .iter()
        .flat_map(|item| {
            item.hosts
                .iter()
                .map(move |h| (item.software_item_id, h.host_id))
        })
        .collect();
    old_set.difference(&new_set).copied().collect()
}

/// Compute the set of host IDs present in `old` but absent from `new`.
///
/// Returns an empty set when `old` is `None` (first update — nothing to remove).
pub(crate) fn compute_removed_host_ids(
    old: Option<impl Iterator<Item = Uuid>>,
    new: impl Iterator<Item = Uuid>,
) -> HashSet<Uuid> {
    let Some(old_iter) = old else {
        return HashSet::new();
    };
    let old_set: HashSet<Uuid> = old_iter.collect();
    let new_set: HashSet<Uuid> = new.collect();
    old_set.difference(&new_set).copied().collect()
}
