use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{mqtt_lease, scheduled_task};

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
    #[tracing::instrument(skip_all, fields(task = "stale_lease_cleanup"))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, Set};
    use uptrakit_shared_db::entity::{mqtt_client, tenant};
    use uptrakit_shared_db::migration::run_migrations;

    async fn setup_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        run_migrations(&db).await.expect("run migrations");
        db
    }

    /// Insert a tenant + mqtt_client for FK compliance and return their IDs.
    async fn insert_mqtt_client_for_test(db: &DatabaseConnection) -> (uuid::Uuid, uuid::Uuid) {
        let now = OffsetDateTime::now_utc();
        let tenant_id = uuid::Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");

        let client_id = uuid::Uuid::now_v7();
        mqtt_client::ActiveModel {
            id: Set(client_id),
            tenant_id: Set(tenant_id),
            enabled: Set(true),
            transport: Set(uptrakit_shared_types::MqttTransport::Tcp),
            host: Set("localhost".to_string()),
            port: Set(1883),
            client_id: Set("test-client".to_string()),
            username: Set(None),
            password: Set(None),
            ca_cert_pem: Set(None),
            topic_prefix: Set("uptrakit".to_string()),
            connection_status: Set(uptrakit_shared_types::MqttClientConnectionStatus::Offline),
            status_updated_at: Set(now),
            ha_discovery: Set(false),
            ha_discovery_prefix: Set("homeassistant".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert mqtt_client");

        (tenant_id, client_id)
    }

    fn make_task() -> scheduled_task::Model {
        let now = OffsetDateTime::now_utc();
        scheduled_task::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7(),
            task_type:
                uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType::StaleLeaseCleanup,
            cron_expression: "* * * * *".to_string(),
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: now,
            locked_by: None,
            locked_at: None,
            last_error: None,
            run_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn empty_db_returns_ok() {
        let db = setup_db().await;
        let executor = StaleLeaseCleanupExecutor::new(db);
        executor
            .execute(&make_task())
            .await
            .expect("should succeed");
    }

    #[tokio::test]
    async fn deletes_stale_lease_keeps_fresh() {
        let db = setup_db().await;
        let now = OffsetDateTime::now_utc();
        // Stale: heartbeat older than STALE_AFTER_SECS (60 s).
        let stale_at = now - time::Duration::seconds(STALE_AFTER_SECS + 10);
        // Fresh: heartbeat very recent.
        let fresh_at = now - time::Duration::seconds(5);

        // Each lease must have a distinct mqtt_client_id (UNIQUE constraint).
        let (tenant_id_stale, stale_client_id) = insert_mqtt_client_for_test(&db).await;
        let (tenant_id_fresh, fresh_client_id) = insert_mqtt_client_for_test(&db).await;

        mqtt_lease::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id_stale),
            mqtt_client_id: Set(stale_client_id),
            instance_id: Set("stale-instance".to_string()),
            heartbeat_at: Set(stale_at),
            created_at: Set(stale_at),
        }
        .insert(&db)
        .await
        .expect("insert stale lease");

        mqtt_lease::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id_fresh),
            mqtt_client_id: Set(fresh_client_id),
            instance_id: Set("fresh-instance".to_string()),
            heartbeat_at: Set(fresh_at),
            created_at: Set(fresh_at),
        }
        .insert(&db)
        .await
        .expect("insert fresh lease");

        let executor = StaleLeaseCleanupExecutor::new(db.clone());
        executor
            .execute(&make_task())
            .await
            .expect("should succeed");

        let remaining = MqttLease::find().all(&db).await.expect("query leases");
        assert_eq!(remaining.len(), 1, "only the fresh lease should remain");
        assert_eq!(remaining[0].instance_id, "fresh-instance");
    }
}
