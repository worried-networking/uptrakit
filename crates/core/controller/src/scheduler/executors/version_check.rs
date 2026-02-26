use std::collections::HashMap;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, JoinType, QueryFilter,
    QuerySelect, RelationTrait, Set, prelude::Expr,
};
use time::OffsetDateTime;
use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{
    CheckVersionsPayload, ControllerMessage, PluginAssignment, VersionCheckAssignment,
};
use uptrakit_plugin_infrastructure_core::PluginCapability;
use uptrakit_plugin_infrastructure_registry::PluginRegistry;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, scheduled_task, service,
    service_host, software_item,
};
use uptrakit_shared_types::PluginType;
use uptrakit_web_api::notification_service::NotificationService;
use uuid::Uuid;

use crate::scheduler::error::SchedulerError;
use crate::scheduler::executor::TaskExecutor;

/// A [`CommandExecutor`] that panics on use.
///
/// The controller process never executes local commands for plugins. API-based
/// plugins (GitHub, Docker) perform HTTP calls internally and never invoke the
/// executor. This struct satisfies the `Arc<dyn CommandExecutor>` requirement
/// of [`PluginRegistry::create_plugin`] without pulling in a real executor.
struct NoopCommandExecutor;

#[async_trait::async_trait]
impl CommandExecutor for NoopCommandExecutor {
    async fn execute(
        &self,
        _spec: &uptrakit_command::CommandSpec,
        _output_tx: &tokio::sync::mpsc::Sender<uptrakit_command::UpdateOutputLine>,
    ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
        unreachable!("NoopCommandExecutor::execute called on the controller — this is a bug")
    }

    async fn execute_quiet(
        &self,
        _spec: &uptrakit_command::CommandSpec,
    ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
        unreachable!("NoopCommandExecutor::execute_quiet called on the controller — this is a bug")
    }
}

/// Sends `CheckVersions` messages to connected agents for installed-version detection
/// and performs controller-side `fetch_releases` for API-based plugins.
///
/// The executor runs in two phases:
///
/// **Phase A — Controller-side fetch_releases:**
/// Queries `host_software_item_plugins` rows with `role = 'fetch_releases'` that
/// should run on the controller (either `execution_site = 'controller'`, or
/// `execution_site = 'auto'` with a plugin that has `ControllerSideFetchReleases`
/// capability). Groups by `(plugin_config_id, package_identifier)` to deduplicate
/// API calls, then stores the latest version in `host_software_items.latest_version`.
///
/// **Phase B — Agent-side assignments:**
/// Builds `VersionCheckAssignment` per `(service_id, host_machine_id)` group using
/// `detect_version` role plugins and `fetch_releases` role plugins that should run
/// on the agent. Sends `CheckVersions` messages as before.
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

// ── Phase A: controller-side fetch_releases ──────────────────────────────────

/// Row returned from the controller-side fetch_releases query.
#[derive(Debug, sea_orm::FromQueryResult)]
struct ControllerFetchRow {
    host_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
    package_identifier: String,
    plugin_type: String,
    config: serde_json::Value,
    config_override: Option<serde_json::Value>,
    execution_site: String,
}

/// Key for deduplicating controller-side fetch_releases calls.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FetchGroupKey {
    plugin_config_id: Uuid,
    package_identifier: String,
}

// ── Phase B: agent-side assignments ──────────────────────────────────────────

/// Row returned from the agent-side assignment query.
#[derive(Debug, sea_orm::FromQueryResult)]
struct AgentAssignmentRow {
    service_id: Uuid,
    host_machine_id: String,
    software_item_id: Uuid,
    software_item_name: String,
    role: String,
    plugin_type: String,
    package_identifier: String,
    config: serde_json::Value,
    config_override: Option<serde_json::Value>,
    execution_site: String,
}

#[async_trait::async_trait]
impl TaskExecutor for VersionCheckExecutor {
    async fn execute(&self, task: &scheduled_task::Model) -> crate::scheduler::error::Result<()> {
        let tenant_id = task.tenant_id;

        // ── Phase A: controller-side fetch_releases ──────────────────────
        self.run_controller_side_fetch_releases(tenant_id).await?;

        // ── Phase B: agent-side assignments ──────────────────────────────
        self.send_agent_assignments(tenant_id).await?;

        Ok(())
    }
}

impl VersionCheckExecutor {
    // ── Phase A ──────────────────────────────────────────────────────────

    /// Execute controller-side fetch_releases for eligible plugins and store
    /// the latest version on all matching `host_software_items` rows.
    ///
    /// After updating `host_software_items`, batch-updates `software_item.last_checked_at`
    /// and pushes MQTT software states so that controller-only items receive the
    /// same post-check notifications as agent-backed items.
    async fn run_controller_side_fetch_releases(
        &self,
        tenant_id: Uuid,
    ) -> crate::scheduler::error::Result<()> {
        let rows = self.query_controller_fetch_releases_rows(tenant_id).await?;
        if rows.is_empty() {
            return Ok(());
        }

        let noop_executor: Arc<dyn CommandExecutor> = Arc::new(NoopCommandExecutor);

        // Group rows by (plugin_config_id, package_identifier). Each group shares
        // the same plugin type + config, so we only call fetch_releases once per group.
        // We also collect the (host_id, software_item_id) pairs to update afterward.
        type FetchGroupValue = (
            String,            // plugin_type
            serde_json::Value, // merged config (base; override applied per-row below)
            String,            // execution_site (from first row; all should match)
            Vec<(Uuid, Uuid)>, // (host_id, software_item_id) targets
        );
        let mut groups: HashMap<FetchGroupKey, FetchGroupValue> = HashMap::new();

        for row in &rows {
            let key = FetchGroupKey {
                plugin_config_id: row.plugin_config_id,
                package_identifier: row.package_identifier.clone(),
            };
            let entry = groups.entry(key).or_insert_with(|| {
                // Use the base config from plugin_config; the first row's
                // config_override is applied below.
                (
                    row.plugin_type.clone(),
                    row.config.clone(),
                    row.execution_site.clone(),
                    Vec::new(),
                )
            });
            entry.3.push((row.host_id, row.software_item_id));
        }

        let now = OffsetDateTime::now_utc();
        // Collect software_item_ids that were successfully updated.
        let mut updated_item_ids: HashSet<Uuid> = HashSet::new();

        // For each group, determine if we should run controller-side, instantiate
        // the plugin, call fetch_releases, and store results.
        for (key, (plugin_type_str, base_config, execution_site, targets)) in &groups {
            let plugin_type = PluginType::from_str(plugin_type_str).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown plugin type: {plugin_type_str}"
                )))
            })?;

            // Find the first row for this group to get a representative config_override
            // for plugin instantiation. For controller-side fetch_releases the
            // config_override typically doesn't vary per host (the package_identifier
            // is the distinguishing factor), but we merge any override found.
            let representative_override = rows
                .iter()
                .find(|r| {
                    r.plugin_config_id == key.plugin_config_id
                        && r.package_identifier == key.package_identifier
                })
                .and_then(|r| r.config_override.as_ref());

            let merged = match representative_override {
                Some(ovr) => merge_config(base_config, ovr),
                None => base_config.clone(),
            };

            // Determine whether this group should run on the controller.
            // In the "auto" case, instantiate the plugin to check capability; if it
            // should run on the controller we reuse the same instance for fetch_releases.
            let plugin: Option<Box<dyn uptrakit_plugin_infrastructure_core::Plugin>> =
                match execution_site.as_str() {
                    "controller" => {
                        // Explicit controller — create the plugin now.
                        let p = PluginRegistry::create_plugin(
                            plugin_type.clone(),
                            &merged,
                            noop_executor.clone(),
                        )
                        .map_err(|e| {
                            report!(SchedulerError::Execution(format!(
                                "failed to create plugin {plugin_type}: {e}"
                            )))
                        })?;
                        Some(p)
                    }
                    "agent" => None,
                    _ => {
                        // "auto" — check capability
                        let p = PluginRegistry::create_plugin(
                            plugin_type.clone(),
                            &merged,
                            noop_executor.clone(),
                        )
                        .map_err(|e| {
                            report!(SchedulerError::Execution(format!(
                                "failed to create plugin {plugin_type} for capability check: {e}"
                            )))
                        })?;
                        if p.has_capability(PluginCapability::ControllerSideFetchReleases) {
                            Some(p)
                        } else {
                            None
                        }
                    }
                };

            let Some(plugin) = plugin else {
                continue;
            };

            let releases = match plugin.fetch_releases(&key.package_identifier).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        plugin_type = %plugin_type,
                        package = %key.package_identifier,
                        error = %e,
                        "controller-side fetch_releases failed; skipping"
                    );
                    continue;
                }
            };

            // Determine the latest stable version (first non-prerelease, or first overall).
            let latest = releases
                .iter()
                .find(|r| !r.is_prerelease)
                .or(releases.first());

            let Some(latest) = latest else {
                tracing::debug!(
                    plugin_type = %plugin_type,
                    package = %key.package_identifier,
                    "fetch_releases returned no releases"
                );
                continue;
            };

            let latest_version_str = latest.version.to_string();
            let release_metadata = serde_json::to_value(latest).unwrap_or(serde_json::Value::Null);

            tracing::debug!(
                plugin_type = %plugin_type,
                package = %key.package_identifier,
                latest_version = %latest_version_str,
                host_count = targets.len(),
                "controller-side fetch_releases succeeded"
            );

            // Update all host_software_items rows sharing this plugin_config + package_identifier.
            for (host_id, software_item_id) in targets {
                let active = host_software_item::ActiveModel {
                    host_id: Set(*host_id),
                    software_item_id: Set(*software_item_id),
                    latest_version: Set(Some(latest_version_str.clone())),
                    latest_version_fetched_at: Set(Some(now)),
                    latest_release_metadata: Set(Some(release_metadata.clone())),
                    ..Default::default()
                };
                if let Err(e) = active.update(&self.db).await {
                    tracing::warn!(
                        host_id = %host_id,
                        software_item_id = %software_item_id,
                        error = %e,
                        "failed to update host_software_item with latest version"
                    );
                } else {
                    updated_item_ids.insert(*software_item_id);
                }
            }
        }

        if !updated_item_ids.is_empty() {
            // Batch-update software_item.last_checked_at for all items with successful fetches.
            let item_ids: Vec<Uuid> = updated_item_ids.into_iter().collect();
            if let Err(e) = software_item::Entity::update_many()
                .filter(software_item::Column::Id.is_in(item_ids))
                .col_expr(software_item::Column::LastCheckedAt, Expr::value(now))
                .exec(&self.db)
                .await
            {
                tracing::warn!(
                    error = %e,
                    "controller-side fetch: failed to batch-update last_checked_at"
                );
            }

            // Push updated software states to MQTT services.
            self.notification_service
                .push_software_states_for_tenant(tenant_id)
                .await;
        }

        Ok(())
    }

    /// Query `host_software_item_plugins` rows with `role = 'fetch_releases'`
    /// and `execution_site != 'agent'`, scoped to the tenant's active software items.
    async fn query_controller_fetch_releases_rows(
        &self,
        tenant_id: Uuid,
    ) -> crate::scheduler::error::Result<Vec<ControllerFetchRow>> {
        // host_software_item_plugin
        //   -> software_item (for tenant + enabled filter)
        //   -> plugin_config (for plugin_type + config)
        let rows: Vec<ControllerFetchRow> = host_software_item_plugin::Entity::find()
            .select_only()
            .column_as(host_software_item_plugin::Column::HostId, "host_id")
            .column_as(
                host_software_item_plugin::Column::SoftwareItemId,
                "software_item_id",
            )
            .column_as(
                host_software_item_plugin::Column::PluginConfigId,
                "plugin_config_id",
            )
            .column_as(
                host_software_item_plugin::Column::PackageIdentifier,
                "package_identifier",
            )
            .column_as(plugin_config::Column::PluginType, "plugin_type")
            .column_as(plugin_config::Column::Config, "config")
            .column_as(
                host_software_item_plugin::Column::ConfigOverride,
                "config_override",
            )
            .column_as(
                host_software_item_plugin::Column::ExecutionSite,
                "execution_site",
            )
            .join(
                JoinType::InnerJoin,
                host_software_item_plugin::Relation::SoftwareItem.def(),
            )
            .join(
                JoinType::InnerJoin,
                host_software_item_plugin::Relation::PluginConfig.def(),
            )
            .filter(host_software_item_plugin::Column::Role.eq("fetch_releases"))
            .filter(host_software_item_plugin::Column::ExecutionSite.ne("agent"))
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::Enabled.eq(true))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(plugin_config::Column::Enabled.eq(true))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .filter(
                sea_orm::Condition::any()
                    .add(software_item::Column::DiscoveryState.is_null())
                    .add(software_item::Column::DiscoveryState.ne("pending")),
            )
            .into_model::<ControllerFetchRow>()
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        Ok(rows)
    }

    // ── Phase B ──────────────────────────────────────────────────────────

    /// Build and send `CheckVersions` messages to agents.
    async fn send_agent_assignments(&self, tenant_id: Uuid) -> crate::scheduler::error::Result<()> {
        let rows = self.query_agent_assignment_rows(tenant_id).await?;
        if rows.is_empty() {
            tracing::debug!("no software items assigned to agents for version check");
            return Ok(());
        }

        let noop_executor: Arc<dyn CommandExecutor> = Arc::new(NoopCommandExecutor);

        // Build VersionCheckAssignment per (service_id, host_machine_id, software_item_id).
        // Each software item may have up to two roles: detect_version and fetch_releases.
        //
        // Key: (service_id, host_machine_id)
        // Inner key: software_item_id -> partial VersionCheckAssignment
        let mut by_agent_host: HashMap<(Uuid, String), HashMap<Uuid, VersionCheckAssignment>> =
            HashMap::new();

        for row in rows {
            let plugin_type = PluginType::from_str(&row.plugin_type).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown plugin type: {}",
                    row.plugin_type
                )))
            })?;

            let config = match row.config_override {
                Some(ovr) => merge_config(&row.config, &ovr),
                None => row.config,
            };

            let assignment = PluginAssignment {
                plugin_type,
                package_identifier: row.package_identifier,
                config,
            };

            let agent_key = (row.service_id, row.host_machine_id.clone());
            let items = by_agent_host.entry(agent_key).or_default();
            let item =
                items
                    .entry(row.software_item_id)
                    .or_insert_with(|| VersionCheckAssignment {
                        software_item_id: row.software_item_id,
                        name: row.software_item_name.clone(),
                        detect_version: None,
                        fetch_releases: None,
                    });

            match row.role.as_str() {
                "detect_version" => {
                    item.detect_version = Some(assignment);
                }
                "fetch_releases" => {
                    // Only include fetch_releases for agent-side execution.
                    let should_agent_handle = match row.execution_site.as_str() {
                        "agent" => true,
                        "controller" => false,
                        _ => {
                            // "auto" — check if plugin lacks ControllerSideFetchReleases
                            let plugin = PluginRegistry::create_plugin(
                                assignment.plugin_type.clone(),
                                &assignment.config,
                                noop_executor.clone(),
                            );
                            match plugin {
                                Ok(p) => {
                                    !p.has_capability(PluginCapability::ControllerSideFetchReleases)
                                }
                                Err(_) => {
                                    // If we can't create the plugin, let the agent try
                                    true
                                }
                            }
                        }
                    };
                    if should_agent_handle {
                        item.fetch_releases = Some(assignment);
                    }
                }
                other => {
                    tracing::warn!(role = other, "unexpected role in version check query");
                }
            }
        }

        // Flatten and send messages.
        let mut msg_count = 0;
        let mut item_count = 0;

        for ((service_id, host_machine_id), items) in by_agent_host {
            let assignments: Vec<VersionCheckAssignment> = items
                .into_values()
                .filter(|a| a.detect_version.is_some() || a.fetch_releases.is_some())
                .collect();
            if assignments.is_empty() {
                continue;
            }
            item_count += assignments.len();
            msg_count += 1;
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

    /// Query agent-side plugin assignments (detect_version + fetch_releases roles)
    /// joined through host -> service_host -> service for routing.
    async fn query_agent_assignment_rows(
        &self,
        tenant_id: Uuid,
    ) -> crate::scheduler::error::Result<Vec<AgentAssignmentRow>> {
        // host_software_item_plugin
        //   -> software_item (tenant + enabled filter)
        //   -> plugin_config (plugin_type + config)
        //   -> host          (machine_id for routing)
        //   <- service_host  (agent service mapping)
        //   -> service       (agent service id)
        let rows: Vec<AgentAssignmentRow> = host_software_item_plugin::Entity::find()
            .select_only()
            .column_as(service::Column::Id, "service_id")
            .column_as(host::Column::MachineId, "host_machine_id")
            .column_as(
                host_software_item_plugin::Column::SoftwareItemId,
                "software_item_id",
            )
            .column_as(software_item::Column::Name, "software_item_name")
            .column_as(host_software_item_plugin::Column::Role, "role")
            .column_as(plugin_config::Column::PluginType, "plugin_type")
            .column_as(
                host_software_item_plugin::Column::PackageIdentifier,
                "package_identifier",
            )
            .column_as(plugin_config::Column::Config, "config")
            .column_as(
                host_software_item_plugin::Column::ConfigOverride,
                "config_override",
            )
            .column_as(
                host_software_item_plugin::Column::ExecutionSite,
                "execution_site",
            )
            .join(
                JoinType::InnerJoin,
                host_software_item_plugin::Relation::SoftwareItem.def(),
            )
            .join(
                JoinType::InnerJoin,
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
            .filter(
                host_software_item_plugin::Column::Role.is_in(["detect_version", "fetch_releases"]),
            )
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::Enabled.eq(true))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(plugin_config::Column::Enabled.eq(true))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .filter(service::Column::DeactivatedAt.is_null())
            .filter(
                sea_orm::Condition::any()
                    .add(software_item::Column::DiscoveryState.is_null())
                    .add(software_item::Column::DiscoveryState.ne("pending")),
            )
            .into_model::<AgentAssignmentRow>()
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        Ok(rows)
    }
}

/// Merge a base plugin config with per-item overrides.
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
