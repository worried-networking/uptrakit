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
use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{
    CheckVersionsPayload, ControllerMessage, PluginAssignment, VersionCheckAssignment,
};
use uptrakit_plugin_infrastructure_core::PluginCapability;
use uptrakit_plugin_infrastructure_registry::PluginRegistry;
use uptrakit_shared_db::entity::{
    host_software_item, host_software_item_plugin, plugin_config, scheduled_task, software_item,
};
use uptrakit_shared_types::PluginType;
use uuid::Uuid;

use super::queries::{merge_config, query_agent_assignment_rows};
use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::notifier::SchedulerNotifier;

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

/// Fetches latest available release information for all tracked software items.
///
/// This executor handles the **fetch_releases** half of what was previously the
/// single `version_check` task. It runs in two phases:
///
/// **Phase A — Controller-side fetch_releases:**
/// Queries `host_software_item_plugins` rows with `role = 'fetch_releases'` that
/// should run on the controller (either `execution_site = 'controller'`, or
/// `execution_site = 'auto'` with a plugin that has `ControllerSideFetchReleases`
/// capability). Groups by `(plugin_config_id, package_identifier)` to deduplicate
/// API calls, then stores the latest version in `host_software_items.latest_version`.
///
/// **Phase B — Agent-side fetch_releases assignments:**
/// Builds `VersionCheckAssignment` per `(service_id, host_machine_id)` group using
/// `fetch_releases` role plugins that should run on the agent (APT, Homebrew, npm).
/// Sends `CheckVersions` messages with only `fetch_releases` set (no `detect_version`).
///
/// Installed-version detection is handled by the separate [`DetectVersionExecutor`](super::detect_version::DetectVersionExecutor).
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

/// Key for deduplicating controller-side fetch_releases calls.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FetchGroupKey {
    plugin_config_id: Uuid,
    package_identifier: String,
}

/// One independent controller-side fetch job, ready to be spawned.
struct FetchJob {
    key: FetchGroupKey,
    targets: Vec<(Uuid, Uuid)>,
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
    /// Phase A runs all eligible fetch calls concurrently (up to
    /// [`MAX_CONCURRENT_CONTROLLER_FETCHES`] at a time) via a `JoinSet` and a
    /// `Semaphore`. After all fetches complete, the DB update loop and MQTT
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
        // Group rows by (plugin_config_id, package_identifier). Each group shares
        // the same plugin type + config, so we only call fetch_releases once per group.
        type FetchGroupValue = (
            String,            // plugin_type
            serde_json::Value, // base config
            String,            // execution_site
            Vec<(Uuid, Uuid)>, // (host_id, software_item_id) targets
        );
        let mut groups: HashMap<FetchGroupKey, FetchGroupValue> = HashMap::new();

        for row in &rows {
            let key = FetchGroupKey {
                plugin_config_id: row.plugin_config_id,
                package_identifier: row.package_identifier.clone(),
            };
            let entry = groups.entry(key).or_insert_with(|| {
                (
                    row.plugin_type.clone(),
                    row.config.clone(),
                    row.execution_site.clone(),
                    Vec::new(),
                )
            });
            entry.3.push((row.host_id, row.software_item_id));
        }

        // ── 2. Build FetchJobs ────────────────────────────────────────────
        // Instantiate plugins synchronously here (cheap config construction),
        // skipping non-controller-side groups. Each job is then spawned into
        // the JoinSet.
        let mut jobs: Vec<FetchJob> = Vec::new();

        for (key, (plugin_type_str, base_config, execution_site, targets)) in &groups {
            let plugin_type = PluginType::from_str(plugin_type_str).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown plugin type: {plugin_type_str}"
                )))
            })?;

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

            let should_run_controller_side = match execution_site.as_str() {
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
                &merged,
                noop_executor.clone(),
            )
            .await
            .map_err(|e| {
                report!(SchedulerError::Execution(format!(
                    "failed to create plugin {plugin_type}: {e}"
                )))
            })?;

            jobs.push(FetchJob {
                key: key.clone(),
                targets: targets.clone(),
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
        /// `(group_key, targets, releases)` tuples accumulated after all spawned fetches complete.
        type FetchRecord = (
            FetchGroupKey,
            Vec<(Uuid, Uuid)>,
            Vec<uptrakit_plugin_infrastructure_core::UpstreamRelease>,
        );
        type FetchResult = crate::error::Result<(
            FetchGroupKey,
            Vec<(Uuid, Uuid)>,
            uptrakit_plugin_infrastructure_core::Result<
                Vec<uptrakit_plugin_infrastructure_core::UpstreamRelease>,
            >,
        )>;
        let sem = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONTROLLER_FETCHES));
        let mut join_set: JoinSet<FetchResult> = JoinSet::new();

        for job in jobs {
            let sem = Arc::clone(&sem);
            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.map_err(|e| {
                    report!(SchedulerError::Execution(format!(
                        "semaphore acquire failed (this is a bug): {e}"
                    )))
                })?;
                let releases = job.plugin.fetch_releases(&job.key.package_identifier).await;
                Ok((job.key, job.targets, releases))
            });
        }

        // ── 4. Collect results ────────────────────────────────────────────
        // Results arrive as tasks complete (not necessarily in spawn order).
        let mut fetch_results: Vec<FetchRecord> = Vec::new();

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok(Ok((key, targets, Ok(releases)))) => {
                    fetch_results.push((key, targets, releases));
                }
                Ok(Ok((key, _, Err(e)))) => {
                    tracing::warn!(
                        package = %key.package_identifier,
                        error = %e,
                        "controller-side fetch_releases failed; skipping"
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

        for (key, targets, releases) in fetch_results {
            // Determine the latest stable version (first non-prerelease, or first overall).
            let latest = releases
                .iter()
                .find(|r| !r.is_prerelease)
                .or(releases.first());

            let Some(latest) = latest else {
                tracing::debug!(
                    package = %key.package_identifier,
                    "fetch_releases returned no releases"
                );
                continue;
            };

            let latest_version_str = latest.version.to_string();
            let release_metadata = serde_json::to_value(latest).unwrap_or(serde_json::Value::Null);
            let category_str = latest.category.unwrap_or_default().to_string();

            tracing::debug!(
                package = %key.package_identifier,
                latest_version = %latest_version_str,
                host_count = targets.len(),
                "controller-side fetch_releases succeeded"
            );

            for (host_id, software_item_id) in &targets {
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
            match crate::software_states::load_software_states_for_tenant(&self.db, tenant_id)
                .await
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
    /// `fetch_releases` assignments (APT, Homebrew, npm and other agent-side
    /// release-index plugins). Host packages are excluded here — they only
    /// have `detect_version` assignments handled by [`DetectVersionExecutor`](super::detect_version::DetectVersionExecutor).
    async fn send_agent_fetch_release_assignments(
        &self,
        tenant_id: Uuid,
    ) -> crate::error::Result<()> {
        let rows =
            query_agent_assignment_rows(&self.db, tenant_id, &["fetch_releases"]).await?;

        if rows.is_empty() {
            tracing::debug!("no agent-side fetch_releases items");
            return Ok(());
        }

        // Build VersionCheckAssignment per (service_id, host_machine_id).
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
            let item = items
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
