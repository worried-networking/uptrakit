//! Discovery trigger helper.
//!
//! Contains the `trigger_discovery_for_agent_host` function extracted from the
//! unified handler module. This is re-exported as `pub(crate)` by the parent
//! `handler` module for use by `hosts.rs`.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use uptrakit_shared_db::entity::{plugin_config, plugin_type_setting};
use uptrakit_wire::{ControllerMessage, DiscoverSoftwarePayload, DiscoveryPluginAssignment};

use crate::AppState;
use crate::queries::discovery_allowlist::{load_host_allowlist_set, load_tenant_allowlist_set};

// ---------------------------------------------------------------------------
// trigger_discovery_for_agent_host
// ---------------------------------------------------------------------------

/// Send `DiscoverSoftware` to the given agent for the given host.
///
/// Queries all active plugin configs for discovery-capable plugin types.
/// If no configs exist for a type, sends a single default (empty-config)
/// assignment so the agent can still discover software.
///
/// For package manager plugin types, discovery config is read from the
/// `plugin_type_settings` table instead of `plugin_configs`. These types
/// do not require per-config credential rows.
///
/// The effective allowlist is determined as follows:
/// 1. If the host has specific allowlist entries -> only those plugin types run.
/// 2. Else if the tenant has allowlist entries -> only those plugin types run.
/// 3. Else (unconfigured) -> all discovery plugin types run (backward-compatible default).
pub(crate) async fn trigger_discovery_for_agent_host(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    host_id: uuid::Uuid,
    host_machine_id: &str,
) {
    let discovery_types = state.plugin_ops.discovery_plugins();

    // Determine the effective allowlist for this host.
    let host_allowed = load_host_allowlist_set(state.db(), host_id).await;
    let effective_filter: Option<HashSet<String>> = if !host_allowed.is_empty() {
        Some(host_allowed) // host-specific allowlist takes full precedence
    } else {
        let tenant_allowed = load_tenant_allowlist_set(state.db(), tenant_id).await;
        if !tenant_allowed.is_empty() {
            Some(tenant_allowed) // fall back to tenant allowlist
        } else {
            None // unconfigured -> all allowed (legacy/default)
        }
    };

    let mut plugins: Vec<DiscoveryPluginAssignment> = Vec::new();

    for plugin_type in discovery_types {
        // Apply allowlist filter.
        if let Some(ref allowed) = effective_filter
            && !allowed.contains(plugin_type.as_str())
        {
            continue;
        }

        let wire_plugin_type = plugin_type.clone();

        if state.plugin_ops.has_type_settings(&plugin_type) {
            // Package manager types read config from plugin_type_settings.
            let type_str = plugin_type.to_string();
            let config = match plugin_type_setting::Entity::find()
                .filter(plugin_type_setting::Column::TenantId.eq(tenant_id))
                .filter(plugin_type_setting::Column::PluginType.eq(&type_str))
                .one(state.db())
                .await
            {
                Ok(Some(setting)) => setting.config,
                Ok(None) => serde_json::Value::Object(Default::default()),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        %plugin_type,
                        "failed to query plugin_type_settings for discovery trigger"
                    );
                    continue;
                }
            };

            plugins.push(DiscoveryPluginAssignment {
                plugin_config_id: None,
                plugin_type: wire_plugin_type.clone(),
                config,
            });
        } else {
            // Non-package-manager types read from plugin_configs as before.
            let type_str = plugin_type.to_string();

            let configs = match plugin_config::Entity::find()
                .filter(plugin_config::Column::TenantId.eq(tenant_id))
                .filter(plugin_config::Column::PluginType.eq(&type_str))
                .filter(plugin_config::Column::Enabled.eq(true))
                .filter(plugin_config::Column::DeactivatedAt.is_null())
                .all(state.db())
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        %plugin_type,
                        "failed to query plugin configs for discovery trigger"
                    );
                    continue;
                }
            };

            if configs.is_empty() {
                // No configs for this type -- send a default assignment.
                plugins.push(DiscoveryPluginAssignment {
                    plugin_config_id: None,
                    plugin_type: wire_plugin_type.clone(),
                    config: serde_json::Value::Object(Default::default()),
                });
            } else {
                for cfg in configs {
                    plugins.push(DiscoveryPluginAssignment {
                        plugin_config_id: Some(cfg.id),
                        plugin_type: wire_plugin_type.clone(),
                        config: cfg.config,
                    });
                }
            }
        }
    }

    if plugins.is_empty() {
        tracing::debug!(
            %service_id,
            "no discovery-capable plugins configured or allowed; skipping discovery trigger"
        );
        return;
    }

    let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
        host_machine_id: host_machine_id.to_string(),
        plugins,
    });

    tracing::info!(
        %service_id,
        %host_machine_id,
        "triggering autodiscovery for newly registered host"
    );
    state
        .notification
        .notification_service
        .send(&service_id, msg)
        .await;
}
