use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260209_000001_initial;
mod m20260227_000001_drop_controller_events;
mod m20260227_000002_remove_event_cleanup_tasks;
mod m20260227_000003_discovery_allowlist;
mod m20260301_000001_notifications;
mod m20260302_000001_add_missing_indexes;
mod m20260303_000001_global_settings;
mod m20260303_000002_revoked_tokens;
mod m20260305_000001_crl_cache;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260209_000001_initial::Migration),
            Box::new(m20260227_000001_drop_controller_events::Migration),
            Box::new(m20260227_000002_remove_event_cleanup_tasks::Migration),
            Box::new(m20260227_000003_discovery_allowlist::Migration),
            Box::new(m20260301_000001_notifications::Migration),
            Box::new(m20260302_000001_add_missing_indexes::Migration),
            Box::new(m20260303_000001_global_settings::Migration),
            Box::new(m20260303_000002_revoked_tokens::Migration),
            Box::new(m20260305_000001_crl_cache::Migration),
        ]
    }
}

/// Run all pending migrations.
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    Migrator::up(db, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};

    #[tokio::test]
    async fn migrations_run_on_empty_sqlite() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();
        db.execute_unprepared("SELECT count(*) FROM software_items")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM plugin_configs")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM tenant_discovery_allowlist")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM host_discovery_allowlist")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM notification_channels")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM notification_rules")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM notification_log")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM global_settings")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM revoked_token_jtis")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM revoked_token_users")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM crl_cache")
            .await
            .unwrap();
    }
}
