use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{controller_event, scheduled_task};
use uptrakit_web_api::event_poller::EVENT_CLEANUP_TTL_HOURS;

use crate::scheduler::error::SchedulerError;
use crate::scheduler::executor::TaskExecutor;

/// Deletes controller events older than the retention window.
pub struct EventCleanupExecutor {
    db: DatabaseConnection,
}

impl EventCleanupExecutor {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for EventCleanupExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::scheduler::error::Result<()> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::hours(EVENT_CLEANUP_TTL_HOURS);
        let result = controller_event::Entity::delete_many()
            .filter(controller_event::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await
            .context_to::<SchedulerError>()?;
        if result.rows_affected > 0 {
            tracing::debug!(
                deleted = result.rows_affected,
                "cleaned up old controller events"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};
    use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;

    fn dummy_task() -> scheduled_task::Model {
        scheduled_task::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7(),
            task_type: ScheduledTaskType::EventCleanup,
            cron_expression: "0 * * * *".to_string(),
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: OffsetDateTime::now_utc(),
            locked_by: None,
            locked_at: None,
            last_error: None,
            run_count: 0,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    #[tokio::test]
    async fn execute_deletes_old_events() {
        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 5,
            }])
            .into_connection();

        let executor = EventCleanupExecutor::new(db);
        let result = executor.execute(&dummy_task()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn execute_succeeds_with_no_rows_deleted() {
        let db = MockDatabase::new(DbBackend::Sqlite)
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();

        let executor = EventCleanupExecutor::new(db);
        let result = executor.execute(&dummy_task()).await;
        assert!(result.is_ok());
    }
}
