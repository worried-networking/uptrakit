//! Discovery trigger helper.
//!
//! Contains the `trigger_discovery_for_agent_host` function extracted from the
//! unified handler module. This is re-exported as `pub(crate)` by the parent
//! `handler` module for use by `hosts.rs`.

use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use uptrakit_internal_wire::{ControllerMessage, DiscoverSoftwarePayload, DiscoveryPluginAssignment};
use uptrakit_shared_db::entity::plugin_config;

use crate::AppState;

// ---------------------------------------------------------------------------
// trigger_discovery_for_agent_host
// ---------------------------------------------------------------------------

/// Send `DiscoverSoftware` to the given agent for the given host.
///
/// Queries all active plugin configs for discovery-capable plugin types.
/// If no configs exist for a type, sends a single default (empty-config)
/// assignment so the agent can still discover software.
pub(crate) async fn trigger_discovery_for_agent_host(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    host_machine_id: &str,
) {
    let discovery_types = state.plugin_ops.discovery_plugins();

    let mut plugins: Vec<DiscoveryPluginAssignment> = Vec::new();

    for plugin_type in discovery_types {
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
                plugin_type: plugin_type.clone(),
                config: serde_json::Value::Object(Default::default()),
            });
        } else {
            for cfg in configs {
                plugins.push(DiscoveryPluginAssignment {
                    plugin_config_id: Some(cfg.id),
                    plugin_type: plugin_type.clone(),
                    config: cfg.config,
                });
            }
        }
    }

    if plugins.is_empty() {
        tracing::debug!(
            %service_id,
            "no discovery-capable plugins configured; skipping discovery trigger"
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
    state.notification_service.send(&service_id, msg).await;
}
