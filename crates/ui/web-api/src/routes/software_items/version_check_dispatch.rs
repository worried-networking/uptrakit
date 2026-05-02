//! Version check dispatch orchestration.
//!
//! Loads all data required to dispatch version checks across hosts assigned to a
//! software item, then separates controller-side fetch jobs from agent-side
//! [`CheckVersions`] messages.

#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::Response;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, RelationTrait};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, service,
    service_host,
};
use uptrakit_shared_types::PluginTypeId;
use uuid::Uuid;

use crate::AppState;
use crate::error_response::error_response;
use crate::tenant_db::TenantDb;

use super::controller_fetch::{
    ControllerFetchJob, is_controller_fetch_site, run_controller_fetch_jobs,
};

/// All pre-loaded data needed to dispatch version checks across hosts.
pub(super) struct VersionCheckContext {
    pub(super) links: Vec<host_software_item::Model>,
    pub(super) plugin_assignments: Vec<host_software_item_plugin::Model>,
    pub(super) hosts: HashMap<Uuid, host::Model>,
    pub(super) service_hosts: HashMap<Uuid, Uuid>,
    pub(super) configs: HashMap<Uuid, plugin_config::Model>,
    pub(super) plugin_by_host_role: HashMap<(Uuid, String), usize>,
}

impl VersionCheckContext {
    fn merged_plugin_config(
        &self,
        plugin_type: &PluginTypeId,
        config_model: Option<&plugin_config::Model>,
        assignment_config: Option<&serde_json::Value>,
    ) -> serde_json::Value {
        let _ = plugin_type;
        uptrakit_config_merge::resolve_effective_config(
            None,
            config_model.map(|c| &c.config),
            assignment_config,
        )
    }

    /// Build a `PluginAssignment` from a plugin row and its (optional) config.
    pub(super) fn build_assignment(
        &self,
        plugin: &host_software_item_plugin::Model,
    ) -> Option<uptrakit_wire::PluginAssignment> {
        let config_model = plugin
            .plugin_config_id
            .and_then(|pc_id| self.configs.get(&pc_id));
        let plugin_type_str = config_model
            .map(|c| c.plugin_type.clone())
            .unwrap_or_else(|| plugin.plugin_type.clone());
        let plugin_type = PluginTypeId::new(plugin_type_str);
        let merged = self.merged_plugin_config(&plugin_type, config_model, plugin.config.as_ref());
        Some(uptrakit_wire::PluginAssignment {
            plugin_type,
            package_identifier: plugin.package_identifier.clone(),
            config: merged,
        })
    }

    /// Look up a plugin assignment by (host_id, role).
    pub(super) fn get_plugin(
        &self,
        host_id: Uuid,
        role: &str,
    ) -> Option<&host_software_item_plugin::Model> {
        self.plugin_by_host_role
            .get(&(host_id, role.to_string()))
            .map(|&idx| &self.plugin_assignments[idx])
    }
}

/// Load all data needed to dispatch version checks: host links, plugin
/// assignments, hosts, service mappings, and plugin configs.
pub(super) async fn load_version_check_context(
    tenant_db: &TenantDb,
    item_id: Uuid,
) -> std::result::Result<VersionCheckContext, Response> {
    let links = HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load software item hosts: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?;

    if links.is_empty() {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "No hosts assigned to this software item",
        ));
    }

    let plugin_assignments = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.is_in(["detect_version", "fetch_releases"]))
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load plugin assignments: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?;

    let host_ids: Vec<Uuid> = links.iter().map(|l| l.host_id).collect();
    let config_ids: Vec<Uuid> = plugin_assignments
        .iter()
        .filter_map(|p| p.plugin_config_id)
        .collect();

    let hosts: HashMap<Uuid, host::Model> = tenant_db
        .find::<host::Entity>()
        .filter(host::Column::Id.is_in(host_ids.clone()))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load hosts: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?
        .into_iter()
        .map(|h| (h.id, h))
        .collect();

    let service_hosts: HashMap<Uuid, Uuid> = tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.is_in(host_ids))
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(service::Column::Status.eq(service::ServiceStatus::Approved))
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load service-host links: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?
        .into_iter()
        .map(|sh| (sh.host_id, sh.service_id))
        .collect();

    let configs: HashMap<Uuid, plugin_config::Model> = if config_ids.is_empty() {
        HashMap::new()
    } else {
        tenant_db
            .find::<plugin_config::Entity>()
            .filter(plugin_config::Column::Id.is_in(config_ids))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .all(tenant_db.db())
            .await
            .map_err(|e| {
                tracing::error!("Failed to load plugin configs: {e}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            })?
            .into_iter()
            .map(|c| (c.id, c))
            .collect()
    };

    // Index plugin assignments by (host_id, role) → index into plugin_assignments vec.
    let mut plugin_by_host_role: HashMap<(Uuid, String), usize> = HashMap::new();
    for (idx, pa) in plugin_assignments.iter().enumerate() {
        plugin_by_host_role.insert((pa.host_id, pa.role.clone()), idx);
    }

    Ok(VersionCheckContext {
        links,
        plugin_assignments,
        hosts,
        service_hosts,
        configs,
        plugin_by_host_role,
    })
}

/// Collect controller-side fetch_releases jobs (deduplicated) and execute them.
pub(super) async fn collect_and_run_controller_fetches(
    tenant_db: &TenantDb,
    state: &Arc<AppState>,
    ctx: &VersionCheckContext,
) -> u32 {
    let mut controller_job_map: HashMap<(String, String), ControllerFetchJob> = HashMap::new();
    for pa in ctx
        .plugin_assignments
        .iter()
        .filter(|pa| pa.role == "fetch_releases")
    {
        let config_model = pa
            .plugin_config_id
            .and_then(|pc_id| ctx.configs.get(&pc_id));
        let plugin_type_str = config_model
            .map(|c| c.plugin_type.clone())
            .unwrap_or_else(|| pa.plugin_type.clone());
        let plugin_type = PluginTypeId::new(plugin_type_str);
        let merged = ctx.merged_plugin_config(&plugin_type, config_model, pa.config.as_ref());
        if is_controller_fetch_site(&pa.execution_site, &plugin_type, &merged) {
            let dedup_key = pa
                .plugin_config_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| pa.plugin_type.clone());
            let key = (dedup_key, pa.package_identifier.clone());
            controller_job_map
                .entry(key)
                .or_insert_with(|| ControllerFetchJob {
                    plugin_type: plugin_type.clone(),
                    package_identifier: pa.package_identifier.clone(),
                    merged_config: merged,
                    targets: Vec::new(),
                })
                .targets
                .push((pa.host_id, pa.software_item_id));
        }
    }

    run_controller_fetch_jobs(
        tenant_db.db(),
        &state.notification.notification_service,
        &state.notification.event_broadcaster,
        tenant_db.tenant_id,
        controller_job_map.into_values().collect(),
    )
    .await
}

/// Send CheckVersions messages to agents for agent-side assignments.
/// Returns the number of agents notified.
pub(super) async fn dispatch_agent_version_checks(
    state: &Arc<AppState>,
    ctx: &VersionCheckContext,
    item_id: Uuid,
    item_name: &str,
) -> u32 {
    let mut agents_notified: u32 = 0;
    let mut seen = std::collections::HashSet::new();

    for link in &ctx.links {
        let Some(host_record) = ctx.hosts.get(&link.host_id) else {
            continue;
        };
        let Some(&service_id) = ctx.service_hosts.get(&link.host_id) else {
            continue;
        };
        if !seen.insert((service_id, link.host_id)) {
            continue;
        }

        let detect_version = ctx
            .get_plugin(link.host_id, "detect_version")
            .and_then(|p| ctx.build_assignment(p));

        let fetch_releases = ctx
            .get_plugin(link.host_id, "fetch_releases")
            .and_then(|p| {
                let config_model = p.plugin_config_id.and_then(|pc_id| ctx.configs.get(&pc_id));
                let plugin_type_str = config_model
                    .map(|c| c.plugin_type.clone())
                    .unwrap_or_else(|| p.plugin_type.clone());
                let plugin_type = PluginTypeId::new(plugin_type_str);
                let merged =
                    ctx.merged_plugin_config(&plugin_type, config_model, p.config.as_ref());
                // Skip assignments that ran (or will run) controller-side.
                if is_controller_fetch_site(&p.execution_site, &plugin_type, &merged) {
                    None
                } else {
                    ctx.build_assignment(p)
                }
            });

        // No agent-side work for this host — controller-side fetch handled it.
        if detect_version.is_none() && fetch_releases.is_none() {
            continue;
        }

        let assignment = uptrakit_wire::VersionCheckAssignment {
            software_item_id: item_id,
            name: item_name.to_string(),
            detect_version,
            fetch_releases,
            host_software_item_id: Some(link.id),
        };

        let msg =
            uptrakit_wire::ControllerMessage::CheckVersions(uptrakit_wire::CheckVersionsPayload {
                host_machine_id: host_record.machine_id.clone(),
                assignments: vec![assignment],
            });
        state
            .notification
            .notification_service
            .send(&service_id, msg)
            .await;
        agents_notified += 1;
    }

    agents_notified
}
