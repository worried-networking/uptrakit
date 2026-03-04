use std::collections::HashMap;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use tokio::task::JoinSet;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, JoinType, QueryFilter,
    QuerySelect, RelationTrait, Set, prelude::Expr,
};
use time::OffsetDateTime;
use uptrakit_command::{CommandExecutor, NoopCommandExecutor};
use uptrakit_internal_wire::{
    CheckVersionsPayload, ControllerMessage, PluginAssignment, VersionCheckAssignment,
};
use uptrakit_plugin_infrastructure_core::{BatchFetchItem, PluginCapability};
use uptrakit_plugin_infrastructure_registry::PluginRegistry;
use uptrakit_shared_db::entity::{
    host_software_item, host_software_item_plugin, plugin_config, scheduled_task, software_item,
};
use uptrakit_shared_types::PluginType;
use uuid::Uuid;

use super::queries::{
    merge_config, query_agent_assignment_rows, query_host_package_assignment_rows,
};
use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::notifier::SchedulerNotifier;

/// Fetches latest available release information for all tracked software items.
///
/// This executor handles the **fetch_releases** half of what was previously the
/// single `version_check` task. It runs in two phases:
///
/// **Phase A — Controller-side fetch_releases:**
/// Queries `host_software_item_plugins` rows with `role = 'fetch_releases'` that
/// should run on the controller (either `execution_site = 'controller'`, or
/// `execution_site = 'auto'` with a plugin that has `ControllerSideFetchReleases`
/// capability). Groups by `plugin_config_id` and calls `batch_fetch_releases` once
/// per config, reducing API or subprocess overhead for multi-package configs.
/// Stores the latest version in `host_software_items.latest_version`.
///
/// **Phase B — Agent-side fetch_releases assignments:**
/// Builds `VersionCheckAssignment` per `(service_id, host_machine_id)` group using
/// two sources:
///
/// 1. `host_software_item_plugins` rows with `role = 'fetch_releases'` that should
///    run on the agent (APT, Homebrew, npm — i.e. without `ControllerSideFetchReleases`).
///    Results update `host_software_items.latest_version`.
/// 2. `host_packages` rows (auto-discovered packages). Each carries `host_package_id`
///    so the controller handler routes results to `host_packages.latest_version`.
///
/// Both sources send `CheckVersions` messages with only `fetch_releases` set
/// (no `detect_version`). Installed-version detection is handled by the separate
/// [`DetectVersionExecutor`](super::detect_version::DetectVersionExecutor).
pub struct FetchReleasesExecutor {
    db: DatabaseConnection,
    notifier: Arc<dyn SchedulerNotifier>,
}

impl FetchReleasesExecutor {
    pub fn new(db: DatabaseConnection, notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { db, notifier }
    }
}

// ── Phase A: controller-side fetch_releases ──────────────────────────────────

/// Maximum number of concurrent controller-side `fetch_releases` calls.
///
/// Bounds parallelism to avoid hammering rate-limited external APIs.
/// The HTTP client pool further limits actual open connections.
const MAX_CONCURRENT_CONTROLLER_FETCHES: usize = 10;

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

/// Data for a single controller-side Phase A fetch group.
///
/// All packages sharing the same `plugin_config_id` are batched into one
/// `batch_fetch_releases` call, reducing API or subprocess overhead.
struct PhaseAGroup {
    plugin_type: String,
    /// Merged plugin config (base + representative override from the first row).
    merged_config: serde_json::Value,
    execution_site: String,
    /// package_identifier → `(host_id, software_item_id)` targets to update.
    packages: HashMap<String, Vec<(Uuid, Uuid)>>,
}

/// One independent controller-side fetch job, ready to be spawned into a `JoinSet`.
struct FetchJob {
    packages: HashMap<String, Vec<(Uuid, Uuid)>>,
    plugin: Box<dyn uptrakit_plugin_infrastructure_core::Plugin>,
}

#[async_trait::async_trait]
impl TaskExecutor for FetchReleasesExecutor {
    async fn execute(&self, task: &scheduled_task::Model) -> crate::error::Result<()> {
        let tenant_id = task.tenant_id;

        // Phase A (controller-side API calls) and Phase B (agent dispatch) are
        // independent — run them concurrently to reduce overall wall-clock time.
        let (a, b) = tokio::join!(
            self.run_controller_side_fetch_releases(tenant_id),
            self.send_agent_fetch_release_assignments(tenant_id),
        );
        a?;
        b?;
        Ok(())
    }
}

impl FetchReleasesExecutor {
    // ── Phase A ──────────────────────────────────────────────────────────

    /// Execute controller-side fetch_releases for eligible plugins and store
    /// the latest version on all matching `host_software_items` rows.
    ///
    /// Rows are grouped by `plugin_config_id`; all packages within a group are
    /// passed to `batch_fetch_releases` in a single call. Groups run concurrently
    /// (up to [`MAX_CONCURRENT_CONTROLLER_FETCHES`] at a time) via a `JoinSet`
    /// and a `Semaphore`. After all fetches complete, the DB update loop and MQTT
    /// push run sequentially.
    ///
    /// After updating `host_software_items`, batch-updates `software_item.last_checked_at`
    /// and pushes MQTT software states so that controller-only items receive the
    /// same post-check notifications as agent-backed items.
    async fn run_controller_side_fetch_releases(
        &self,
        tenant_id: Uuid,
    ) -> crate::error::Result<()> {
        let rows = self.query_controller_fetch_releases_rows(tenant_id).await?;
        if rows.is_empty() {
            return Ok(());
        }

        let noop_executor: Arc<dyn CommandExecutor> = Arc::new(NoopCommandExecutor);

        // ── 1. Build groups map ───────────────────────────────────────────
        // Group rows by plugin_config_id only. All packages sharing the same plugin
        // config are batched into a single batch_fetch_releases call instead of N
        // individual fetch_releases calls. The first row's config_override is used
        // as a representative merge for plugin instantiation.
        let mut groups: HashMap<Uuid, PhaseAGroup> = HashMap::new();

        for row in &rows {
            let entry = groups.entry(row.plugin_config_id).or_insert_with(|| {
                let merged_config = match row.config_override.as_ref() {
                    Some(ovr) => merge_config(&row.config, ovr),
                    None => row.config.clone(),
                };
                PhaseAGroup {
                    plugin_type: row.plugin_type.clone(),
                    merged_config,
                    execution_site: row.execution_site.clone(),
                    packages: HashMap::new(),
                }
            });
            entry
                .packages
                .entry(row.package_identifier.clone())
                .or_default()
                .push((row.host_id, row.software_item_id));
        }

        // ── 2. Build FetchJobs ────────────────────────────────────────────
        // Instantiate plugins synchronously here (cheap config construction),
        // skipping non-controller-side groups. Each job carries all packages
        // for its plugin config and is spawned into the JoinSet.
        let mut jobs: Vec<FetchJob> = Vec::new();

        for (_plugin_config_id, group) in groups {
            let plugin_type = PluginType::from_str(&group.plugin_type).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown plugin type: {}",
                    group.plugin_type
                )))
            })?;

            let should_run_controller_side = match group.execution_site.as_str() {
                "controller" => true,
                "agent" => false,
                _ => PluginRegistry::capabilities_for(plugin_type.clone())
                    .contains(&PluginCapability::ControllerSideFetchReleases),
            };

            if !should_run_controller_side {
                continue;
            }

            let plugin = PluginRegistry::create_plugin(
                plugin_type.clone(),
                &group.merged_config,
                noop_executor.clone(),
            )
            .await
            .map_err(|e| {
                report!(SchedulerError::Execution(format!(
                    "failed to create plugin {plugin_type}: {e}"
                )))
            })?;

            jobs.push(FetchJob {
                packages: group.packages,
                plugin,
            });
        }

        if jobs.is_empty() {
            return Ok(());
        }

        // ── 3. Spawn all jobs into a JoinSet with semaphore ───────────────
        // The semaphore bounds peak concurrency to avoid hammering rate-limited APIs.
        // AcquireError only fires if the semaphore is closed, which cannot happen
        // here — map it to a fatal SchedulerError rather than unwrapping.
        /// `(packages, batch_results)` tuples accumulated after all spawned fetches complete.
        type FetchRecord = (
            HashMap<String, Vec<(Uuid, Uuid)>>,
            Vec<uptrakit_plugin_infrastructure_core::BatchFetchResult>,
        );
        type FetchResult = crate::error::Result<(
            HashMap<String, Vec<(Uuid, Uuid)>>,
            uptrakit_plugin_infrastructure_core::Result<
                Vec<uptrakit_plugin_infrastructure_core::BatchFetchResult>,
            >,
        )>;
        let sem = Arc::new(tokio::sync::Semaphore::new(
            MAX_CONCURRENT_CONTROLLER_FETCHES,
        ));
        let mut join_set: JoinSet<FetchResult> = JoinSet::new();

        for job in jobs {
            let sem = Arc::clone(&sem);
            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.map_err(|e| {
                    report!(SchedulerError::Execution(format!(
                        "semaphore acquire failed (this is a bug): {e}"
                    )))
                })?;
                let fetch_items: Vec<BatchFetchItem> = job
                    .packages
                    .keys()
                    .map(|pkg| BatchFetchItem {
                        package_identifier: pkg.clone(),
                    })
                    .collect();
                let results = job.plugin.batch_fetch_releases(&fetch_items).await;
                Ok((job.packages, results))
            });
        }

        // ── 4. Collect results ────────────────────────────────────────────
        // Results arrive as tasks complete (not necessarily in spawn order).
        let mut fetch_results: Vec<FetchRecord> = Vec::new();

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok(Ok((packages, Ok(results)))) => {
                    fetch_results.push((packages, results));
                }
                Ok(Ok((_, Err(e)))) => {
                    tracing::warn!(
                        error = %e,
                        "controller-side batch_fetch_releases failed; skipping group"
                    );
                }
                Ok(Err(e)) => {
                    // Propagate semaphore/plugin creation errors (should not happen).
                    return Err(e);
                }
                Err(join_err) => {
                    tracing::warn!(error = %join_err, "fetch task panicked; skipping");
                }
            }
        }

        // ── 5. Sequential DB update loop ──────────────────────────────────
        let now = OffsetDateTime::now_utc();
        let mut updated_item_ids: HashSet<Uuid> = HashSet::new();

        for (packages, batch_results) in fetch_results {
            for result in batch_results {
                if let Some(ref err) = result.error {
                    tracing::warn!(
                        package = %result.package_identifier,
                        error = %err,
                        "controller-side fetch_releases failed for package; skipping"
                    );
                    continue;
                }

                // Determine the latest stable version (first non-prerelease, or first overall).
                let latest = result
                    .releases
                    .iter()
                    .find(|r| !r.is_prerelease)
                    .or(result.releases.first());

                let Some(latest) = latest else {
                    tracing::debug!(
                        package = %result.package_identifier,
                        "fetch_releases returned no releases"
                    );
                    continue;
                };

                let latest_version_str = latest.version.to_string();
                let release_metadata =
                    serde_json::to_value(latest).unwrap_or(serde_json::Value::Null);
                let category_str = latest.category.clone().unwrap_or_default().to_string();

                let Some(targets) = packages.get(&result.package_identifier) else {
                    continue;
                };

                tracing::debug!(
                    package = %result.package_identifier,
                    latest_version = %latest_version_str,
                    host_count = targets.len(),
                    "controller-side fetch_releases succeeded"
                );

                for (host_id, software_item_id) in targets {
                    let active = host_software_item::ActiveModel {
                        host_id: Set(*host_id),
                        software_item_id: Set(*software_item_id),
                        latest_version: Set(Some(latest_version_str.clone())),
                        latest_version_fetched_at: Set(Some(now)),
                        latest_release_metadata: Set(Some(release_metadata.clone())),
                        update_category: Set(category_str.clone()),
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

            // Load software states then push to MQTT services.
            match crate::software_states::load_software_states_for_tenant(&self.db, tenant_id).await
            {
                Ok(payload) => {
                    self.notifier.push_software_states_for_tenant(payload).await;
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        %tenant_id,
                        "failed to load software states for MQTT push after controller-side fetch"
                    );
                }
            }
        }

        Ok(())
    }

    /// Query `host_software_item_plugins` rows with `role = 'fetch_releases'`
    /// and `execution_site != 'agent'`, scoped to the tenant's active software items.
    async fn query_controller_fetch_releases_rows(
        &self,
        tenant_id: Uuid,
    ) -> crate::error::Result<Vec<ControllerFetchRow>> {
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

    /// Build and send `CheckVersions` messages to agents containing only
    /// `fetch_releases` assignments. Covers two sources:
    ///
    /// - Targeted software items with a `role = 'fetch_releases'` plugin
    ///   assignment (APT, Homebrew, npm, and other agent-side release-index
    ///   plugins that are not [`PluginCapability::ControllerSideFetchReleases`]).
    /// - Auto-discovered host packages (`host_packages` table) whose plugin
    ///   config supports agent-side release fetching. These assignments carry
    ///   `host_package_id` so the controller handler routes results to the
    ///   `host_packages.latest_version` column.
    async fn send_agent_fetch_release_assignments(
        &self,
        tenant_id: Uuid,
    ) -> crate::error::Result<()> {
        let rows = query_agent_assignment_rows(&self.db, tenant_id, &["fetch_releases"]).await?;
        let hp_rows = query_host_package_assignment_rows(&self.db, tenant_id).await?;

        if rows.is_empty() && hp_rows.is_empty() {
            tracing::debug!("no agent-side fetch_releases items");
            return Ok(());
        }

        // Build VersionCheckAssignment per (service_id, host_machine_id).
        let mut by_agent_host: HashMap<(Uuid, String), HashMap<Uuid, VersionCheckAssignment>> =
            HashMap::new();

        // Targeted software items (fetch_releases role).
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
                plugin_type: plugin_type.clone(),
                package_identifier: row.package_identifier,
                config,
            };

            // Only include fetch_releases for agent-side execution.
            let should_agent_handle = match row.execution_site.as_str() {
                "agent" => true,
                "controller" => false,
                _ => {
                    // "auto" — check static capability (no instantiation)
                    !PluginRegistry::capabilities_for(plugin_type)
                        .contains(&PluginCapability::ControllerSideFetchReleases)
                }
            };
            if !should_agent_handle {
                continue;
            }

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
                        host_package_id: None,
                    });

            item.fetch_releases = Some(assignment);
        }

        // Host packages — use the same plugin config for fetch_releases so that
        // `host_packages.latest_version` is populated alongside `installed_version`.
        for row in hp_rows {
            let plugin_type = PluginType::from_str(&row.plugin_type).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown plugin type: {}",
                    row.plugin_type
                )))
            })?;

            // Skip any plugin that declares controller-side capability; host
            // package plugins (APT, Homebrew, npm) are always agent-side.
            if PluginRegistry::capabilities_for(plugin_type.clone())
                .contains(&PluginCapability::ControllerSideFetchReleases)
            {
                continue;
            }

            let assignment = PluginAssignment {
                plugin_type,
                package_identifier: row.package_identifier,
                config: row.config,
            };

            let agent_key = (row.service_id, row.host_machine_id.clone());
            let items = by_agent_host.entry(agent_key).or_default();
            // Use host_package_id as the map key (guaranteed unique per host package).
            let item = items
                .entry(row.host_package_id)
                .or_insert_with(|| VersionCheckAssignment {
                    // Wire-compat: software_item_id reused; handler routes via host_package_id.
                    software_item_id: row.host_package_id,
                    name: row.host_package_name.clone(),
                    detect_version: None,
                    fetch_releases: None,
                    host_package_id: Some(row.host_package_id),
                });

            item.fetch_releases = Some(assignment);
        }

        // Flatten and send messages.
        let mut msg_count = 0;
        let mut item_count = 0;

        for ((service_id, host_machine_id), items) in by_agent_host {
            let assignments: Vec<VersionCheckAssignment> = items
                .into_values()
                .filter(|a| a.fetch_releases.is_some())
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
            self.notifier.send_to_service(&service_id, msg).await;
        }

        tracing::debug!(
            messages = msg_count,
            items = item_count,
            "sent fetch_releases requests to agents"
        );
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifier::NoopSchedulerNotifier;
    use sea_orm::{ConnectOptions, Database};
    use uptrakit_shared_db::migration::run_migrations;

    #[tokio::test]
    async fn fetch_releases_executor_empty_db_returns_ok() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        let notifier = Arc::new(NoopSchedulerNotifier);
        let executor = FetchReleasesExecutor::new(db.clone(), notifier);

        // Build a minimal scheduled_task model for the call.
        let tenant_id = uuid::Uuid::now_v7();
        let task = scheduled_task::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id,
            task_type: uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType::FetchReleases,
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
        };

        // With no software items in the DB, execute should return Ok(()).
        executor.execute(&task).await.unwrap();
    }
}
