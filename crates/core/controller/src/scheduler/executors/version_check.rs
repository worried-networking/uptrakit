use std::collections::HashMap;
use std::str::FromStr;

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait,
};
use uptrakit_internal_wire::{CheckVersionsPayload, ControllerMessage, VersionCheckAssignment};
use uptrakit_shared_db::entity::{
    host, host_software_item, plugin_config, scheduled_task, service, service_host, software_item,
};
use uptrakit_shared_types::PluginType;
use uptrakit_web_api::notification_service::NotificationService;
use uuid::Uuid;

use crate::scheduler::error::SchedulerError;
use crate::scheduler::executor::TaskExecutor;

/// Sends `CheckVersions` messages to connected agents for installed-version detection.
///
/// Groups software items by `(service_id, host_machine_id)` so that each
/// message targets exactly one host. The SSH agent uses `host_machine_id` to
/// route the request to the correct remote host; the regular agent uses it for
/// a defensive sanity check.
pub struct VersionCheckExecutor {
    db: DatabaseConnection,
    notification_service: NotificationService,
}

impl VersionCheckExecutor {
    pub fn new(db: DatabaseConnection, notification_service: NotificationService) -> Self {
        Self {
            db,
            notification_service,
        }
    }
}

/// Intermediate row produced by the joined query.
#[derive(Debug)]
struct AssignmentRow {
    service_id: Uuid,
    host_machine_id: String,
    software_item_id: Uuid,
    name: String,
    plugin_type: String,
    package_identifier: String,
    config: serde_json::Value,
    config_override: Option<serde_json::Value>,
}

#[async_trait::async_trait]
impl TaskExecutor for VersionCheckExecutor {
    async fn execute(&self, task: &scheduled_task::Model) -> crate::scheduler::error::Result<()> {
        let tenant_id = task.tenant_id;

        // Fetch all enabled software items joined through hosts -> agents for this tenant.
        let rows = self.fetch_assignments(tenant_id).await?;
        if rows.is_empty() {
            tracing::debug!("no software items assigned to agents for version check");
            return Ok(());
        }

        // Group by (service_id, host_machine_id) — each message targets one host.
        let mut by_agent_host: HashMap<(Uuid, String), Vec<VersionCheckAssignment>> =
            HashMap::new();
        for row in rows {
            let provider_type = PluginType::from_str(&row.plugin_type).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown provider type: {}",
                    row.plugin_type
                )))
            })?;
            let config = match row.config_override {
                Some(ovr) => merge_config(&row.config, &ovr),
                None => row.config,
            };
            by_agent_host
                .entry((row.service_id, row.host_machine_id))
                .or_default()
                .push(VersionCheckAssignment {
                    software_item_id: row.software_item_id,
                    name: row.name,
                    plugin_type: provider_type,
                    package_identifier: row.package_identifier,
                    config,
                });
        }

        let msg_count = by_agent_host.len();
        let item_count: usize = by_agent_host.values().map(|v| v.len()).sum();

        for ((service_id, host_machine_id), assignments) in by_agent_host {
            let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
                host_machine_id,
                assignments,
            });
            self.notification_service.send(&service_id, msg).await;
        }

        tracing::debug!(
            messages = msg_count,
            items = item_count,
            "sent version check requests"
        );
        Ok(())
    }
}

impl VersionCheckExecutor {
    /// Query enabled software items with their host-to-agent mapping.
    ///
    /// Returns one row per (agent_service_id, host, software_item) tuple.
    async fn fetch_assignments(
        &self,
        tenant_id: Uuid,
    ) -> crate::scheduler::error::Result<Vec<AssignmentRow>> {
        // software_item -> plugin_config (for plugin_type + config)
        // software_item -> host_software_item -> host -> service_host -> service (agent)
        //
        // We also select host.machine_id so we can set host_machine_id on
        // CheckVersionsPayload for correct routing at the agent.
        #[derive(Debug, sea_orm::FromQueryResult)]
        struct Row {
            service_id: Uuid,
            host_machine_id: String,
            software_item_id: Uuid,
            name: String,
            plugin_type: String,
            package_identifier: String,
            config: serde_json::Value,
            config_override: Option<serde_json::Value>,
        }

        // software_item
        //   <- host_software_item  (carries plugin_config_id, package_identifier, config_override)
        //   -> plugin_config       (via host_software_item::Relation::PluginConfig)
        //   -> host                (via host_software_item::Relation::Host)
        //   <- service_host
        //   -> service (agent)
        let rows: Vec<Row> = software_item::Entity::find()
            .select_only()
            .column_as(service::Column::Id, "service_id")
            .column_as(host::Column::MachineId, "host_machine_id")
            .column_as(software_item::Column::Id, "software_item_id")
            .column_as(software_item::Column::Name, "name")
            .column_as(plugin_config::Column::PluginType, "plugin_type")
            .column_as(
                host_software_item::Column::PackageIdentifier,
                "package_identifier",
            )
            .column_as(plugin_config::Column::Config, "config")
            .column_as(
                host_software_item::Column::ConfigOverride,
                "config_override",
            )
            .join(
                JoinType::InnerJoin,
                host_software_item::Relation::SoftwareItem.def().rev(),
            )
            .join(
                JoinType::InnerJoin,
                host_software_item::Relation::PluginConfig.def(),
            )
            .join(
                JoinType::InnerJoin,
                host_software_item::Relation::Host.def(),
            )
            .join(
                JoinType::InnerJoin,
                service_host::Relation::Host.def().rev(),
            )
            .join(JoinType::InnerJoin, service_host::Relation::Service.def())
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::Enabled.eq(true))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(plugin_config::Column::Enabled.eq(true))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .filter(service::Column::DeactivatedAt.is_null())
            // Exclude pending-discovery items; approved items (enabled=true) are included.
            .filter(
                sea_orm::Condition::any()
                    .add(software_item::Column::DiscoveryState.is_null())
                    .add(software_item::Column::DiscoveryState.ne("pending")),
            )
            .into_model::<Row>()
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        Ok(rows
            .into_iter()
            .map(|r| AssignmentRow {
                service_id: r.service_id,
                host_machine_id: r.host_machine_id,
                software_item_id: r.software_item_id,
                name: r.name,
                plugin_type: r.plugin_type,
                package_identifier: r.package_identifier,
                config: r.config,
                config_override: r.config_override,
            })
            .collect())
    }
}

/// Merge a base provider config with per-item overrides.
fn merge_config(base: &serde_json::Value, overrides: &serde_json::Value) -> serde_json::Value {
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
