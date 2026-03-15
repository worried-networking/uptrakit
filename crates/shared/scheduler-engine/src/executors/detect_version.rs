use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_internal_wire::{
    CheckVersionsPayload, ControllerMessage, PluginAssignment, VersionCheckAssignment,
};
use uptrakit_shared_db::entity::scheduled_task;
use uptrakit_shared_types::PluginType;
use uuid::Uuid;

use super::queries::query_agent_assignment_rows;
use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::notifier::SchedulerNotifier;

/// Sends `CheckVersions` messages to connected agents asking each to detect
/// the currently installed version of tracked software items.
///
/// This executor handles the **detect_version** half of what was previously the
/// single `version_check` task. It only queries plugins with
/// `role = 'detect_version'`; it does **not** trigger controller-side
/// `fetch_releases` calls. Those remain in
/// [`FetchReleasesExecutor`](super::fetch_releases::FetchReleasesExecutor).
pub struct DetectVersionExecutor {
    db: DatabaseConnection,
    notifier: Arc<dyn SchedulerNotifier>,
}

impl DetectVersionExecutor {
    pub fn new(db: DatabaseConnection, notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { db, notifier }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for DetectVersionExecutor {
    #[tracing::instrument(skip_all, fields(task = "detect_version"))]
    async fn execute(&self, task: &scheduled_task::Model) -> crate::error::Result<()> {
        self.send_detect_version_assignments(task.tenant_id).await
    }
}

impl DetectVersionExecutor {
    /// Build and send `CheckVersions` messages that carry only `detect_version`
    /// assignments (no `fetch_releases`).
    async fn send_detect_version_assignments(&self, tenant_id: Uuid) -> crate::error::Result<()> {
        let rows = query_agent_assignment_rows(&self.db, tenant_id, &["detect_version"]).await?;

        tracing::debug!(
            %tenant_id,
            software_item_rows = rows.len(),
            "detect_version: queried assignment rows"
        );

        if rows.is_empty() {
            tracing::debug!(%tenant_id, "no items assigned to agents for detect_version");
            return Ok(());
        }

        // Build VersionCheckAssignment per (service_id, host_machine_id).
        // Key: (service_id, host_machine_id)
        // Inner key: assignment key Uuid -> VersionCheckAssignment
        let mut by_agent_host: HashMap<(Uuid, String), HashMap<Uuid, VersionCheckAssignment>> =
            HashMap::new();

        for row in rows {
            let plugin_type = PluginType::from_str(&row.plugin_type).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown plugin type: {}",
                    row.plugin_type
                )))
            })?;

            let config = uptrakit_config_merge::resolve_effective_config(
                None, // type_settings not loaded in scheduler query yet
                row.profile_config.as_ref(),
                row.assignment_config.as_ref(),
            );

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
                        host_software_item_id: Some(row.host_software_item_id),
                    });

            item.detect_version = Some(assignment);
        }

        // Flatten and send messages.
        let mut msg_count = 0;
        let mut item_count = 0;

        for ((service_id, host_machine_id), items) in by_agent_host {
            let assignments: Vec<VersionCheckAssignment> = items
                .into_values()
                .filter(|a| a.detect_version.is_some())
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

        tracing::info!(
            %tenant_id,
            messages = msg_count,
            items = item_count,
            "sent detect_version requests to agents"
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
    async fn detect_version_executor_empty_db_returns_ok() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        let notifier = Arc::new(NoopSchedulerNotifier);
        let executor = DetectVersionExecutor::new(db.clone(), notifier);

        // Build a minimal scheduled_task model for the call.
        let tenant_id = uuid::Uuid::now_v7();
        let task = scheduled_task::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id,
            task_type: uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType::DetectVersion,
            interval_seconds: 86400,
            jitter_seconds: 300,
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
