use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_shared_db::entity::scheduled_task;
use uptrakit_web_api::mqtt_lease_coordinator::{MQTT_LEASE_STALE_AFTER, MqttLeaseCoordinator};
use uptrakit_web_api::service_connections::ServiceConnectionRegistry;

use crate::scheduler::error::SchedulerError;
use crate::scheduler::executor::TaskExecutor;

/// Cleans stale MQTT leases whose heartbeat has expired.
pub struct StaleLeaseCleanupExecutor {
    db: DatabaseConnection,
    registry: ServiceConnectionRegistry,
}

impl StaleLeaseCleanupExecutor {
    pub fn new(db: DatabaseConnection, registry: ServiceConnectionRegistry) -> Self {
        Self { db, registry }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for StaleLeaseCleanupExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::scheduler::error::Result<()> {
        let coordinator = MqttLeaseCoordinator::new(self.db.clone(), self.registry.clone());
        let deleted = coordinator
            .cleanup_stale_leases(MQTT_LEASE_STALE_AFTER)
            .await
            .context_transform(|e| SchedulerError::Execution(e.to_string()))?;
        if deleted > 0 {
            tracing::debug!(deleted, "cleaned up stale MQTT leases");
        }
        Ok(())
    }
}
