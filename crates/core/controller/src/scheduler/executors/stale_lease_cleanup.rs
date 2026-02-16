use sea_orm::DatabaseConnection;
use uptrakit_shared_db::entity::scheduled_task;
use uptrakit_web_api::mqtt_lease_coordinator::{MqttLeaseCoordinator, MQTT_LEASE_STALE_AFTER};
use uptrakit_web_api::service_connections::ServiceConnectionRegistry;

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
    async fn execute(&self, _task: &scheduled_task::Model) -> Result<(), String> {
        let coordinator = MqttLeaseCoordinator::new(self.db.clone(), self.registry.clone());
        let deleted = coordinator
            .cleanup_stale_leases(MQTT_LEASE_STALE_AFTER)
            .await
            .map_err(|e| format!("stale lease cleanup failed: {e}"))?;
        if deleted > 0 {
            tracing::debug!(deleted, "cleaned up stale MQTT leases");
        }
        Ok(())
    }
}
