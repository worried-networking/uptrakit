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
mod m20260306_000001_update_category;
mod m20260306_000002_update_batches;
mod m20260302_000002_host_packages;
mod m20260307_000001_split_version_check;

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
            Box::new(m20260306_000001_update_category::Migration),
            Box::new(m20260306_000002_update_batches::Migration),
            Box::new(m20260302_000002_host_packages::Migration),
            Box::new(m20260307_000001_split_version_check::Migration),
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
        // Verify update_category columns exist.
        db.execute_unprepared("SELECT update_category FROM host_software_items LIMIT 0")
            .await
            .unwrap();
        db.execute_unprepared("SELECT update_category FROM update_history LIMIT 0")
            .await
            .unwrap();
        // Verify update_batches table and batch_id column exist.
        db.execute_unprepared("SELECT count(*) FROM update_batches")
            .await
            .unwrap();
        db.execute_unprepared("SELECT batch_id FROM update_history LIMIT 0")
            .await
            .unwrap();
        // Verify host_packages tables exist.
        db.execute_unprepared("SELECT count(*) FROM host_packages")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM host_package_ignores")
            .await
            .unwrap();
        db.execute_unprepared("SELECT count(*) FROM host_package_update_history")
            .await
            .unwrap();
        // Verify split_version_check migration: detect_version task row exists.
        let count_stmt = sea_orm_migration::prelude::Query::select()
            .expr(sea_orm_migration::prelude::Func::count(
                sea_orm_migration::prelude::Expr::col(Alias::new("id")),
            ))
            .from(Alias::new("scheduled_tasks"))
            .and_where(
                sea_orm_migration::prelude::Expr::col(Alias::new("task_type"))
                    .eq("detect_version"),
            )
            .to_owned();
        let detect_version_count_rows = db.query_all(&count_stmt).await.unwrap();
        let detect_version_count: i64 = {
            use sea_orm::TryGetable;
            detect_version_count_rows
                .first()
                .map(|r| i64::try_get_by_index(r, 0).unwrap_or(0))
                .unwrap_or(0)
        };
        assert!(
            detect_version_count >= 1,
            "expected at least one detect_version task after migration, found {detect_version_count}"
        );
    }
}
