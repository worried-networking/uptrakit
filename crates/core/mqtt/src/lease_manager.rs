use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, Set,
};
use time::OffsetDateTime;
use uuid::Uuid;

use uptrakit_shared_db::entity::{mqtt_client, mqtt_lease};

use crate::db::{DbError, Result};

pub struct LeaseManager {
    db: DatabaseConnection,
    instance_id: String,
    max_tenants: u32,
    lease_timeout_secs: u64,
}

impl LeaseManager {
    pub fn new(
        db: DatabaseConnection,
        instance_id: String,
        max_tenants: u32,
        lease_timeout_secs: u64,
    ) -> Self {
        Self {
            db,
            instance_id,
            max_tenants,
            lease_timeout_secs,
        }
    }

    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Delete leases with heartbeat_at older than lease_timeout.
    /// Returns the tenant_ids that were freed.
    pub async fn cleanup_stale_leases(&self) -> Result<Vec<Uuid>> {
        let cutoff =
            OffsetDateTime::now_utc() - time::Duration::seconds(self.lease_timeout_secs as i64);

        let stale = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::HeartbeatAt.lt(cutoff))
            .all(&self.db)
            .await
            .context_to::<DbError>()?;

        let freed: Vec<Uuid> = stale.iter().map(|l| l.tenant_id).collect();

        if !freed.is_empty() {
            mqtt_lease::Entity::delete_many()
                .filter(mqtt_lease::Column::HeartbeatAt.lt(cutoff))
                .exec(&self.db)
                .await
                .context_to::<DbError>()?;

            tracing::info!(count = freed.len(), "cleaned up stale leases");
        }

        Ok(freed)
    }

    /// Claim tenants that have enabled mqtt_clients but no active lease.
    /// Respects max_tenants limit. Returns the tenant_ids that were claimed.
    pub async fn claim_tenants(&self) -> Result<Vec<Uuid>> {
        // Count how many leases we already hold
        let held_count = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::InstanceId.eq(&self.instance_id))
            .count(&self.db)
            .await
            .context_to::<DbError>()? as u32;

        let available = if self.max_tenants == 0 {
            u32::MAX
        } else {
            self.max_tenants.saturating_sub(held_count)
        };

        if available == 0 {
            return Ok(vec![]);
        }

        // Find enabled mqtt_clients without a lease.
        // First load all currently leased tenant_ids, then exclude them.
        let leased: Vec<Uuid> = mqtt_lease::Entity::find()
            .select_only()
            .column(mqtt_lease::Column::TenantId)
            .into_tuple()
            .all(&self.db)
            .await
            .context_to::<DbError>()?;

        let mut query = mqtt_client::Entity::find().filter(mqtt_client::Column::Enabled.eq(true));

        if !leased.is_empty() {
            query = query.filter(mqtt_client::Column::TenantId.is_not_in(leased));
        }

        let unclaimed = query.all(&self.db).await.context_to::<DbError>()?;

        let now = OffsetDateTime::now_utc();
        let mut claimed = Vec::new();

        for client in unclaimed {
            if claimed.len() as u32 >= available {
                break;
            }

            let lease = mqtt_lease::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(client.tenant_id),
                instance_id: Set(self.instance_id.clone()),
                heartbeat_at: Set(now),
                created_at: Set(now),
            };

            // Unique constraint on tenant_id handles races atomically —
            // if another instance claimed it first, the insert fails and we skip.
            match lease.insert(&self.db).await {
                Ok(_) => {
                    tracing::info!(tenant_id = %client.tenant_id, "claimed tenant");
                    claimed.push(client.tenant_id);
                }
                Err(e) => {
                    tracing::debug!(tenant_id = %client.tenant_id, error = %e, "failed to claim tenant (likely race)");
                }
            }
        }

        Ok(claimed)
    }

    /// Update heartbeat_at for all leases held by this instance.
    pub async fn heartbeat(&self) -> Result<()> {
        let now = OffsetDateTime::now_utc();

        mqtt_lease::Entity::update_many()
            .filter(mqtt_lease::Column::InstanceId.eq(&self.instance_id))
            .col_expr(mqtt_lease::Column::HeartbeatAt, now.into())
            .exec(&self.db)
            .await
            .context_to::<DbError>()?;

        Ok(())
    }

    /// Release all leases held by this instance (graceful shutdown).
    pub async fn release_all(&self) -> Result<()> {
        let result = mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::InstanceId.eq(&self.instance_id))
            .exec(&self.db)
            .await
            .context_to::<DbError>()?;

        tracing::info!(count = result.rows_affected, "released all leases");
        Ok(())
    }

    /// Release a specific tenant's lease.
    pub async fn release_tenant(&self, tenant_id: Uuid) -> Result<()> {
        mqtt_lease::Entity::delete_many()
            .filter(mqtt_lease::Column::InstanceId.eq(&self.instance_id))
            .filter(mqtt_lease::Column::TenantId.eq(tenant_id))
            .exec(&self.db)
            .await
            .context_to::<DbError>()?;

        tracing::info!(%tenant_id, "released tenant lease");
        Ok(())
    }

    /// Get tenant_ids currently held by this instance.
    pub async fn held_tenants(&self) -> Result<Vec<Uuid>> {
        let leases = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::InstanceId.eq(&self.instance_id))
            .all(&self.db)
            .await
            .context_to::<DbError>()?;

        Ok(leases.into_iter().map(|l| l.tenant_id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect to in-memory sqlite");

        // Manually create tables to avoid FK issues with auto-generated schemas
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS tenants (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                slug TEXT NOT NULL UNIQUE,
                is_default INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deactivated_at TEXT
            )",
        )
        .await
        .expect("create tenants table");

        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS mqtt_clients (
                id TEXT PRIMARY KEY NOT NULL,
                tenant_id TEXT NOT NULL REFERENCES tenants(id),
                enabled INTEGER NOT NULL DEFAULT 1,
                transport TEXT NOT NULL DEFAULT 'tcp',
                host TEXT NOT NULL,
                port INTEGER NOT NULL DEFAULT 1883,
                path TEXT,
                client_id TEXT NOT NULL DEFAULT 'uptrakit',
                username TEXT,
                password TEXT,
                topic_prefix TEXT NOT NULL DEFAULT 'uptrakit',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(tenant_id)
            )",
        )
        .await
        .expect("create mqtt_clients table");

        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS mqtt_leases (
                id TEXT PRIMARY KEY NOT NULL,
                tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                instance_id TEXT NOT NULL,
                heartbeat_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(tenant_id)
            )",
        )
        .await
        .expect("create mqtt_leases table");

        db
    }

    async fn seed_tenant(db: &DatabaseConnection, tenant_id: Uuid) {
        use uptrakit_shared_db::entity::tenant;

        let now = OffsetDateTime::now_utc();
        let tenant = tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("Test Tenant".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        tenant.insert(db).await.expect("seed tenant");
    }

    async fn seed_mqtt_client(db: &DatabaseConnection, tenant_id: Uuid, enabled: bool) {
        let now = OffsetDateTime::now_utc();
        let client = mqtt_client::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            enabled: Set(enabled),
            transport: Set("tcp".to_string()),
            host: Set("localhost".to_string()),
            port: Set(1883),
            path: Set(None),
            client_id: Set("uptrakit".to_string()),
            username: Set(None),
            password: Set(None),
            topic_prefix: Set("uptrakit".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        client.insert(db).await.expect("seed mqtt_client");
    }

    #[tokio::test]
    async fn claim_unclaimed_tenant() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        seed_tenant(&db, tenant_id).await;
        seed_mqtt_client(&db, tenant_id, true).await;

        let mgr = LeaseManager::new(db, "test-instance".into(), 0, 60);
        let claimed = mgr.claim_tenants().await.unwrap();

        assert_eq!(claimed, vec![tenant_id]);
    }

    #[tokio::test]
    async fn claim_respects_max_tenants() {
        let db = setup_db().await;

        let mut tenant_ids = Vec::new();
        for _ in 0..5 {
            let tid = Uuid::now_v7();
            seed_tenant(&db, tid).await;
            seed_mqtt_client(&db, tid, true).await;
            tenant_ids.push(tid);
        }

        let mgr = LeaseManager::new(db, "test-instance".into(), 2, 60);
        let claimed = mgr.claim_tenants().await.unwrap();

        assert_eq!(claimed.len(), 2);
    }

    #[tokio::test]
    async fn claim_skips_already_claimed() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        seed_tenant(&db, tenant_id).await;
        seed_mqtt_client(&db, tenant_id, true).await;

        let mgr1 = LeaseManager::new(db.clone(), "instance-1".into(), 0, 60);
        let claimed1 = mgr1.claim_tenants().await.unwrap();
        assert_eq!(claimed1.len(), 1);

        let mgr2 = LeaseManager::new(db, "instance-2".into(), 0, 60);
        let claimed2 = mgr2.claim_tenants().await.unwrap();
        assert_eq!(claimed2.len(), 0);
    }

    #[tokio::test]
    async fn stale_lease_reclaimed() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        seed_tenant(&db, tenant_id).await;
        seed_mqtt_client(&db, tenant_id, true).await;

        // Create a stale lease with old heartbeat
        let old_time = OffsetDateTime::now_utc() - time::Duration::seconds(120);
        let lease = mqtt_lease::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            instance_id: Set("old-instance".to_string()),
            heartbeat_at: Set(old_time),
            created_at: Set(old_time),
        };
        lease.insert(&db).await.expect("insert stale lease");

        let mgr = LeaseManager::new(db, "new-instance".into(), 0, 60);
        let freed = mgr.cleanup_stale_leases().await.unwrap();
        assert_eq!(freed, vec![tenant_id]);

        let claimed = mgr.claim_tenants().await.unwrap();
        assert_eq!(claimed, vec![tenant_id]);
    }

    #[tokio::test]
    async fn heartbeat_updates_timestamp() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        seed_tenant(&db, tenant_id).await;
        seed_mqtt_client(&db, tenant_id, true).await;

        let mgr = LeaseManager::new(db.clone(), "test-instance".into(), 0, 60);
        mgr.claim_tenants().await.unwrap();

        let before = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::InstanceId.eq("test-instance"))
            .one(&db)
            .await
            .unwrap()
            .unwrap();

        // Small delay so timestamps differ
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        mgr.heartbeat().await.unwrap();

        let after = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::InstanceId.eq("test-instance"))
            .one(&db)
            .await
            .unwrap()
            .unwrap();

        assert!(after.heartbeat_at >= before.heartbeat_at);
    }

    #[tokio::test]
    async fn release_all_on_shutdown() {
        let db = setup_db().await;

        for _ in 0..3 {
            let tid = Uuid::now_v7();
            seed_tenant(&db, tid).await;
            seed_mqtt_client(&db, tid, true).await;
        }

        let mgr = LeaseManager::new(db.clone(), "test-instance".into(), 0, 60);
        mgr.claim_tenants().await.unwrap();

        let count = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::InstanceId.eq("test-instance"))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(count, 3);

        mgr.release_all().await.unwrap();

        let count = mqtt_lease::Entity::find()
            .filter(mqtt_lease::Column::InstanceId.eq("test-instance"))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn release_specific_tenant() {
        let db = setup_db().await;

        let tid1 = Uuid::now_v7();
        let tid2 = Uuid::now_v7();
        seed_tenant(&db, tid1).await;
        seed_tenant(&db, tid2).await;
        seed_mqtt_client(&db, tid1, true).await;
        seed_mqtt_client(&db, tid2, true).await;

        let mgr = LeaseManager::new(db.clone(), "test-instance".into(), 0, 60);
        mgr.claim_tenants().await.unwrap();

        mgr.release_tenant(tid1).await.unwrap();

        let remaining = mgr.held_tenants().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0], tid2);
    }

    #[tokio::test]
    async fn disabled_tenant_not_claimed() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        seed_tenant(&db, tenant_id).await;
        seed_mqtt_client(&db, tenant_id, false).await;

        let mgr = LeaseManager::new(db, "test-instance".into(), 0, 60);
        let claimed = mgr.claim_tenants().await.unwrap();
        assert!(claimed.is_empty());
    }

    #[tokio::test]
    async fn claim_zero_max_means_unlimited() {
        let db = setup_db().await;

        for _ in 0..10 {
            let tid = Uuid::now_v7();
            seed_tenant(&db, tid).await;
            seed_mqtt_client(&db, tid, true).await;
        }

        let mgr = LeaseManager::new(db, "test-instance".into(), 0, 60);
        let claimed = mgr.claim_tenants().await.unwrap();
        assert_eq!(claimed.len(), 10);
    }
}
