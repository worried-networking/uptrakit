use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, JoinType, QueryFilter, QuerySelect,
    RelationTrait,
};
use uptrakit_shared_db::entity::{
    host, host_software_item_plugin, plugin_config, service, service_host, software_item,
};
use uuid::Uuid;

use crate::error::SchedulerError;

// ── Shared result row types ───────────────────────────────────────────────────

/// Row returned from agent-side assignment queries.
#[derive(Debug, sea_orm::FromQueryResult)]
pub(crate) struct AgentAssignmentRow {
    pub(crate) service_id: Uuid,
    pub(crate) host_machine_id: String,
    pub(crate) software_item_id: Uuid,
    pub(crate) software_item_name: String,
    pub(crate) plugin_type: String,
    pub(crate) package_identifier: String,
    /// FK pointing to the specific `host_software_items.id` row this plugin
    /// assignment belongs to. Used to populate
    /// `VersionCheckAssignment::host_software_item_id` so that the controller
    /// can route results back to the correct row when a service manages
    /// multiple hosts (e.g. SSH agent: one service → N remote hosts).
    pub(crate) host_software_item_id: Uuid,
    /// Profile config from `plugin_configs.config`. NULL when `plugin_config_id`
    /// is NULL (package manager assignments after type settings migration).
    pub(crate) profile_config: Option<serde_json::Value>,
    pub(crate) assignment_config: Option<serde_json::Value>,
    pub(crate) execution_site: String,
}

// ── Shared query helpers ──────────────────────────────────────────────────────

/// Query agent-side plugin assignments for the given roles,
/// joined through `host → service_host → service` for routing.
///
/// `roles` is a non-empty slice of role strings (e.g. `&["detect_version"]`
/// or `&["fetch_releases"]` or `&["detect_version", "fetch_releases"]`).
///
/// Uses LEFT JOIN on `plugin_configs` to handle assignments with nullable
/// `plugin_config_id` (package manager assignments after the type settings
/// migration). The `plugin_type` is read from `host_software_item_plugins`
/// (denormalized column) rather than from `plugin_configs`.
pub(crate) async fn query_agent_assignment_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    roles: &[&str],
) -> crate::error::Result<Vec<AgentAssignmentRow>> {
    let role_strings: Vec<String> = roles.iter().map(|s| s.to_string()).collect();

    let rows: Vec<AgentAssignmentRow> = host_software_item_plugin::Entity::find()
        .select_only()
        .column_as(service::Column::Id, "service_id")
        .column_as(host::Column::MachineId, "host_machine_id")
        .column_as(
            host_software_item_plugin::Column::SoftwareItemId,
            "software_item_id",
        )
        .column_as(software_item::Column::Name, "software_item_name")
        // Read plugin_type from the denormalized HSIP column (not plugin_configs).
        .column_as(host_software_item_plugin::Column::PluginType, "plugin_type")
        .column_as(
            host_software_item_plugin::Column::PackageIdentifier,
            "package_identifier",
        )
        .column_as(plugin_config::Column::Config, "profile_config")
        .column_as(
            host_software_item_plugin::Column::Config,
            "assignment_config",
        )
        .column_as(
            host_software_item_plugin::Column::HostSoftwareItemId,
            "host_software_item_id",
        )
        .column_as(
            host_software_item_plugin::Column::ExecutionSite,
            "execution_site",
        )
        .join(
            JoinType::InnerJoin,
            host_software_item_plugin::Relation::SoftwareItem.def(),
        )
        // LEFT JOIN: plugin_config_id may be NULL for package manager assignments.
        .join(
            JoinType::LeftJoin,
            host_software_item_plugin::Relation::PluginConfig.def(),
        )
        .join(
            JoinType::InnerJoin,
            host_software_item_plugin::Relation::Host.def(),
        )
        .join(
            JoinType::InnerJoin,
            service_host::Relation::Host.def().rev(),
        )
        .join(JoinType::InnerJoin, service_host::Relation::Service.def())
        .filter(host_software_item_plugin::Column::Role.is_in(role_strings))
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        // Accept rows where plugin_config_id is NULL (package managers) OR
        // where the linked config is enabled and not deactivated.
        .filter(
            Condition::any()
                .add(host_software_item_plugin::Column::PluginConfigId.is_null())
                .add(
                    Condition::all()
                        .add(plugin_config::Column::Enabled.eq(true))
                        .add(plugin_config::Column::DeactivatedAt.is_null()),
                ),
        )
        .filter(service::Column::DeactivatedAt.is_null())
        .into_model::<AgentAssignmentRow>()
        .all(db)
        .await
        .context_to::<SchedulerError>()?;

    Ok(rows)
}

// ── Config merge ──────────────────────────────────────────────────────────────

/// Merge a base plugin config with per-item overrides.
///
/// Performs a shallow (top-level key) merge: override keys replace base keys.
/// If either value is not a JSON object the base is returned unchanged.
///
/// Superseded by `uptrakit_update_hooks::resolve_effective_config` for the
/// three-layer config merge (type_settings + profile + assignment). Retained
/// for its unit tests and potential future use.
#[cfg(test)]
pub(crate) fn merge_config(
    base: &serde_json::Value,
    overrides: &serde_json::Value,
) -> serde_json::Value {
    match (base, overrides) {
        (serde_json::Value::Object(b), serde_json::Value::Object(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                merged.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(merged)
        }
        _ => base.clone(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_config_objects() {
        let base = serde_json::json!({"key1": "val1", "key2": "val2"});
        let overrides = serde_json::json!({"key2": "override2", "key3": "val3"});
        let merged = merge_config(&base, &overrides);
        assert_eq!(
            merged,
            serde_json::json!({"key1": "val1", "key2": "override2", "key3": "val3"})
        );
    }

    #[test]
    fn merge_config_non_object_override_returns_base() {
        let base = serde_json::json!({"key": "val"});
        let overrides = serde_json::json!("just a string");
        let merged = merge_config(&base, &overrides);
        assert_eq!(merged, base);
    }

    #[test]
    fn merge_config_non_object_base_returns_base() {
        let base = serde_json::json!(42);
        let overrides = serde_json::json!({"key": "val"});
        let merged = merge_config(&base, &overrides);
        assert_eq!(merged, serde_json::json!(42));
    }

    #[test]
    fn merge_config_empty_override() {
        let base = serde_json::json!({"key": "val"});
        let overrides = serde_json::json!({});
        let merged = merge_config(&base, &overrides);
        assert_eq!(merged, base);
    }

    #[test]
    fn merge_config_nested_objects_replaced_not_deep_merged() {
        let base = serde_json::json!({"nested": {"a": 1, "b": 2}});
        let overrides = serde_json::json!({"nested": {"c": 3}});
        let merged = merge_config(&base, &overrides);
        // Shallow merge: the entire "nested" value is replaced.
        assert_eq!(merged, serde_json::json!({"nested": {"c": 3}}));
    }
}
