use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{mqtt_lease, scheduled_task};
use uptrakit_shared_db::entity::prelude::*;

use crate::error::SchedulerError;
use crate::executor::TaskExecutor;

/// Maximum allowed age of an MQTT lease heartbeat before considering it stale.
const STALE_AFTER_SECS: i64 = 60;

/// Cleans stale MQTT leases whose heartbeat has expired.
///
/// Uses direct DB queries instead of `MqttLeaseCoordinator` so the scheduler
/// engine does not depend on `uptrakit-web-api`.
pub struct StaleLeaseCleanupExecutor {
    db: DatabaseConnection,
}

impl StaleLeaseCleanupExecutor {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for StaleLeaseCleanupExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(STALE_AFTER_SECS);

        let result = MqttLease::delete_many()
            .filter(mqtt_lease::Column::HeartbeatAt.lt(cutoff))
            .exec(&self.db)
            .await
            .context_transform(|e| SchedulerError::Execution(e.to_string()))?;

        let deleted = result.rows_affected;
        if deleted > 0 {
            tracing::debug!(deleted, "cleaned up stale MQTT leases");
        }
        Ok(())
    }
}
