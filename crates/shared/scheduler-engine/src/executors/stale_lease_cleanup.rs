use uptrakit_shared_db::entity::scheduled_task;

use crate::executor::TaskExecutor;

/// No-op executor: stale MQTT lease cleanup has been removed.
///
/// The `mqtt_leases` table was dropped when MQTT client management was migrated
/// to the extension framework. This executor is retained as a registered task
/// type so that existing `scheduled_tasks` rows with `task_type =
/// 'stale_lease_cleanup'` do not cause scheduler errors on startup.
pub struct StaleLeaseCleanupExecutor;

impl StaleLeaseCleanupExecutor {
    pub fn new(_db: sea_orm::DatabaseConnection) -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl TaskExecutor for StaleLeaseCleanupExecutor {
    #[tracing::instrument(skip_all, fields(task = "stale_lease_cleanup"))]
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
        // No-op: mqtt_leases table has been dropped.
        Ok(())
    }
}
