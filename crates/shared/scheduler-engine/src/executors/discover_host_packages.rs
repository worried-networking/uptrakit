//! Executor for the `discover_host_packages` scheduled task.
//!
//! Sends `DiscoverSoftware` messages to every active agent-backed host for the
//! tenant, triggering a full host-package rediscovery run. Packages that
//! disappear from the discovery results are automatically soft-deleted by the
//! autodiscovery result processor in `process_plugin_result`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait,
};
use uptrakit_internal_wire::{
    ControllerMessage, DiscoverSoftwarePayload, DiscoveryPluginAssignment,
};
use uptrakit_plugin_infrastructure_registry::PluginRegistry;
use uptrakit_shared_db::entity::{
    host, host_discovery_allowlist, plugin_config, scheduled_task, service, service_host,
    tenant_discovery_allowlist,
};
use uptrakit_shared_types::PluginType;
use uuid::Uuid;

use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::notifier::SchedulerNotifier;

/// Triggers periodic host-package rediscovery for every active host in the tenant.
///
/// Runs every 6 hours (configurable via `cron_expression` on the scheduled task).
/// Sends `DiscoverSoftware` to every agent-backed host, which causes the agent to
/// run all applicable discovery plugins and report back discovered packages.
///
/// Effective plugin-type allowlists are respected per host:
/// 1. Host-specific allowlist → only those plugin types run on that host.
/// 2. Tenant-wide allowlist → fallback when the host has no specific allowlist.
/// 3. No allowlist → all discovery-capable plugin types run (default).
pub struct DiscoverHostPackagesExecutor {
    db: DatabaseConnection,
    notifier: Arc<dyn SchedulerNotifier>,
}

impl DiscoverHostPackagesExecutor {
    pub fn new(db: DatabaseConnection, notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { db, notifier }
    }
}

/// Minimal host row returned from the active-host query.
#[derive(Debug, sea_orm::FromQueryResult)]
struct HostRow {
    host_id: Uuid,
    host_machine_id: String,
    service_id: Uuid,
}

#[async_trait::async_trait]
impl TaskExecutor for DiscoverHostPackagesExecutor {
    async fn execute(&self, task: &scheduled_task::Model) -> crate::error::Result<()> {
        let tenant_id = task.tenant_id;

        // Query all non-deactivated hosts that have an active service link.
        let host_rows = self.query_host_rows(tenant_id).await?;
        if host_rows.is_empty() {
            tracing::debug!(%tenant_id, "no active hosts found for periodic rediscovery");
            return Ok(());
        }

        // Static registry call — no I/O.
        let discovery_types = PluginRegistry::discovery_plugins();

        // Load allowlists in parallel with plugin config query.
        let host_ids: Vec<Uuid> = host_rows.iter().map(|r| r.host_id).collect();
        let type_strs: Vec<String> = discovery_types.iter().map(|t| t.to_string()).collect();

        let (tenant_allowlist, host_allowlists, configs_by_type) = tokio::try_join!(
            self.load_tenant_allowlist(tenant_id),
            self.load_host_allowlists(host_ids),
            self.load_plugin_configs(tenant_id, &type_strs),
        )?;

        // Send DiscoverSoftware to each host.
        let mut sent = 0usize;
        for row in &host_rows {
            let host_specific = host_allowlists
                .get(&row.host_id)
                .cloned()
                .unwrap_or_default();

            let assignments = build_assignments(
                &discovery_types,
                &host_specific,
                &tenant_allowlist,
                &configs_by_type,
            );

            if assignments.is_empty() {
                tracing::debug!(
                    host_id = %row.host_id,
                    "no discovery assignments for host; skipping"
                );
                continue;
            }

            let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
                host_machine_id: row.host_machine_id.clone(),
                plugins: assignments,
            });

            self.notifier.send_to_service(&row.service_id, msg).await;
            sent += 1;
        }

        tracing::info!(
            %tenant_id,
            hosts_triggered = sent,
            "periodic host-package rediscovery triggered"
        );

        Ok(())
    }
}

impl DiscoverHostPackagesExecutor {
    /// Query all non-deactivated hosts for the tenant that have an active service.
    async fn query_host_rows(&self, tenant_id: Uuid) -> crate::error::Result<Vec<HostRow>> {
        let rows: Vec<HostRow> = host::Entity::find()
            .select_only()
            .column_as(host::Column::Id, "host_id")
            .column_as(host::Column::MachineId, "host_machine_id")
            .column_as(service::Column::Id, "service_id")
            .join(
                JoinType::InnerJoin,
                service_host::Relation::Host.def().rev(),
            )
            .join(JoinType::InnerJoin, service_host::Relation::Service.def())
            .filter(host::Column::TenantId.eq(tenant_id))
            .filter(host::Column::DeactivatedAt.is_null())
            .filter(service::Column::DeactivatedAt.is_null())
            .into_model::<HostRow>()
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        Ok(rows)
    }

    /// Load the tenant-wide discovery allowlist as a set of plugin type strings.
    async fn load_tenant_allowlist(
        &self,
        tenant_id: Uuid,
    ) -> crate::error::Result<HashSet<String>> {
        let rows = tenant_discovery_allowlist::Entity::find()
            .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_id))
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        Ok(rows.into_iter().map(|r| r.plugin_type).collect())
    }

    /// Load host-specific discovery allowlists for all given hosts in one query.
    ///
    /// Returns a map from `host_id` to the set of allowed plugin type strings.
    /// Hosts absent from the map have no host-specific allowlist.
    async fn load_host_allowlists(
        &self,
        host_ids: Vec<Uuid>,
    ) -> crate::error::Result<HashMap<Uuid, HashSet<String>>> {
        if host_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = host_discovery_allowlist::Entity::find()
            .filter(host_discovery_allowlist::Column::HostId.is_in(host_ids))
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        let mut map: HashMap<Uuid, HashSet<String>> = HashMap::new();
        for row in rows {
            map.entry(row.host_id).or_default().insert(row.plugin_type);
        }
        Ok(map)
    }

    /// Load all enabled, non-deactivated plugin configs for the given plugin type strings.
    ///
    /// Returns a map from plugin type string to the list of matching configs.
    async fn load_plugin_configs(
        &self,
        tenant_id: Uuid,
        type_strs: &[String],
    ) -> crate::error::Result<HashMap<String, Vec<plugin_config::Model>>> {
        if type_strs.is_empty() {
            return Ok(HashMap::new());
        }

        let configs = plugin_config::Entity::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.is_in(type_strs.to_vec()))
            .filter(plugin_config::Column::Enabled.eq(true))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        let mut by_type: HashMap<String, Vec<plugin_config::Model>> = HashMap::new();
        for cfg in configs {
            by_type
                .entry(cfg.plugin_type.clone())
                .or_default()
                .push(cfg);
        }
        Ok(by_type)
    }
}

/// Build `DiscoveryPluginAssignment` list for a single host, applying the effective
/// allowlist and mapping plugin configs to assignments.
///
/// Precedence:
/// 1. `host_allowlist` (non-empty) → only those types.
/// 2. `tenant_allowlist` (non-empty) → only those types.
/// 3. Neither set → all `discovery_types`.
///
/// For each allowed plugin type, one assignment per plugin config is created.
/// If no configs exist for a type, one empty-config default assignment is emitted
/// so the agent still attempts discovery with its built-in defaults.
fn build_assignments(
    discovery_types: &[PluginType],
    host_allowlist: &HashSet<String>,
    tenant_allowlist: &HashSet<String>,
    configs_by_type: &HashMap<String, Vec<plugin_config::Model>>,
) -> Vec<DiscoveryPluginAssignment> {
    let effective_filter: Option<&HashSet<String>> = if !host_allowlist.is_empty() {
        Some(host_allowlist)
    } else if !tenant_allowlist.is_empty() {
        Some(tenant_allowlist)
    } else {
        None
    };

    let mut assignments = Vec::new();

    for plugin_type in discovery_types {
        if let Some(filter) = effective_filter
            && !filter.contains(plugin_type.as_str())
        {
            continue;
        }

        let type_str = plugin_type.to_string();
        let configs = configs_by_type.get(&type_str);

        if let Some(configs) = configs
            && !configs.is_empty()
        {
            for cfg in configs {
                assignments.push(DiscoveryPluginAssignment {
                    plugin_config_id: Some(cfg.id),
                    plugin_type: plugin_type.clone(),
                    config: cfg.config.clone(),
                });
            }
        } else {
            // No configs for this type — send a default empty-config assignment
            // so the agent can still attempt discovery with built-in defaults.
            assignments.push(DiscoveryPluginAssignment {
                plugin_config_id: None,
                plugin_type: plugin_type.clone(),
                config: serde_json::Value::Object(Default::default()),
            });
        }
    }

    assignments
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifier::NoopSchedulerNotifier;
    use sea_orm::{ConnectOptions, Database};
    use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
    use uptrakit_shared_db::migration::run_migrations;

    fn make_task(tenant_id: Uuid) -> scheduled_task::Model {
        scheduled_task::Model {
            id: Uuid::now_v7(),
            tenant_id,
            task_type: ScheduledTaskType::DiscoverHostPackages,
            cron_expression: "0 */6 * * *".to_string(),
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: time::OffsetDateTime::now_utc(),
            locked_by: None,
            locked_at: None,
            last_error: None,
            run_count: 0,
            created_at: time::OffsetDateTime::now_utc(),
            updated_at: time::OffsetDateTime::now_utc(),
        }
    }

    #[tokio::test]
    async fn empty_db_returns_ok() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        let notifier = Arc::new(NoopSchedulerNotifier);
        let executor = DiscoverHostPackagesExecutor::new(db, notifier);

        executor.execute(&make_task(Uuid::now_v7())).await.unwrap();
    }

    #[test]
    fn build_assignments_host_allowlist_filters_types() {
        let type_homebrew = PluginType::PackageManagerHomebrew;
        let type_apt = PluginType::PackageManagerApt;

        let host_allowlist: HashSet<String> =
            [type_homebrew.as_str().to_string()].into_iter().collect();
        let tenant_allowlist: HashSet<String> = HashSet::new();
        let configs_by_type: HashMap<String, Vec<plugin_config::Model>> = HashMap::new();

        let discovery_types = vec![type_homebrew.clone(), type_apt];
        let assignments = build_assignments(
            &discovery_types,
            &host_allowlist,
            &tenant_allowlist,
            &configs_by_type,
        );

        // Only homebrew should be included (host allowlist wins).
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].plugin_type, type_homebrew);
    }

    #[test]
    fn build_assignments_no_allowlist_includes_all_types() {
        let host_allowlist: HashSet<String> = HashSet::new();
        let tenant_allowlist: HashSet<String> = HashSet::new();
        let configs_by_type: HashMap<String, Vec<plugin_config::Model>> = HashMap::new();

        let discovery_types = vec![
            PluginType::PackageManagerHomebrew,
            PluginType::PackageManagerApt,
        ];
        let assignments = build_assignments(
            &discovery_types,
            &host_allowlist,
            &tenant_allowlist,
            &configs_by_type,
        );

        // No allowlist → all types included with empty-config default assignments.
        assert_eq!(assignments.len(), 2);
        for a in &assignments {
            assert!(a.plugin_config_id.is_none());
        }
    }

    #[test]
    fn build_assignments_tenant_allowlist_used_as_fallback() {
        let type_apt = PluginType::PackageManagerApt;

        let host_allowlist: HashSet<String> = HashSet::new();
        let tenant_allowlist: HashSet<String> =
            [type_apt.as_str().to_string()].into_iter().collect();
        let configs_by_type: HashMap<String, Vec<plugin_config::Model>> = HashMap::new();

        let discovery_types = vec![PluginType::PackageManagerHomebrew, type_apt.clone()];
        let assignments = build_assignments(
            &discovery_types,
            &host_allowlist,
            &tenant_allowlist,
            &configs_by_type,
        );

        // Tenant allowlist → only apt.
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].plugin_type, type_apt);
    }
}
