use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{controller_event, scheduled_task};
use uptrakit_web_api::event_poller::EVENT_CLEANUP_TTL_HOURS;

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
    async fn execute(&self, _task: &scheduled_task::Model) -> Result<(), String> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::hours(EVENT_CLEANUP_TTL_HOURS);
        let result = controller_event::Entity::delete_many()
            .filter(controller_event::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await
            .map_err(|e| format!("event cleanup failed: {e}"))?;
        if result.rows_affected > 0 {
            tracing::debug!(
                deleted = result.rows_affected,
                "cleaned up old controller events"
            );
        }
        Ok(())
    }
}
