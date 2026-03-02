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
mod m20260302_000003_host_packages_has_update;
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
            Box::new(m20260302_000003_host_packages_has_update::Migration),
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

    /// Simulate the "existing database" upgrade scenario:
    /// the first twelve migrations are applied in a first run, then the
    /// remaining migrations (starting with m20260302_000003_host_packages_has_update)
    /// are applied in a second run.  This catches bugs that only surface when
    /// `host_packages` already exists at the time the recreation migration runs.
    #[tokio::test]
    async fn migrations_run_incrementally_sqlite() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        // Apply the first twelve migrations (everything before
        // m20260302_000003_host_packages_has_update).
        Migrator::up(&db, Some(12))
            .await
            .expect("first 12 migrations should succeed");
        // Apply the rest (m20260302_000003 + m20260307_000001).
        Migrator::up(&db, None)
            .await
            .expect("remaining migrations should succeed on existing database");
        db.execute_unprepared("SELECT has_update FROM host_packages LIMIT 0")
            .await
            .expect("has_update column must exist after incremental migration");
    }

    /// State B recovery: a previous run of m20260302_000003 created
    /// `host_packages_new` but crashed before dropping the original.  Both
    /// tables exist.  The migration must discard the partial temp table and
    /// restart from scratch.
    #[tokio::test]
    async fn migrations_tolerate_leftover_temp_table_state_b() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        // Apply everything up to and including m20260302_000002_host_packages.
        Migrator::up(&db, Some(12)).await.unwrap();

        // Simulate: host_packages_new was created but host_packages was NOT yet
        // dropped (both tables exist).
        db.execute_unprepared(
            "CREATE TABLE host_packages_new AS SELECT * FROM host_packages",
        )
        .await
        .unwrap();

        // The next Migrator::up call must not crash.
        Migrator::up(&db, None).await.expect(
            "migration must succeed even when host_packages_new already exists alongside original",
        );
        db.execute_unprepared("SELECT has_update FROM host_packages LIMIT 0")
            .await
            .expect("has_update column must exist after State B recovery");
    }

    /// State C recovery: a previous run of m20260302_000003 created
    /// `host_packages_new`, copied all data, and dropped the original, but
    /// crashed before the rename.  Only `host_packages_new` exists.  The
    /// migration must rename it without re-creating or re-copying.
    #[tokio::test]
    async fn migrations_tolerate_leftover_temp_table_state_c() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        Migrator::up(&db, Some(12)).await.unwrap();

        // Simulate: copy done, original dropped, rename not yet done.
        // Use the real schema so the rename results in a valid host_packages.
        db.execute_unprepared(
            "CREATE TABLE host_packages_new AS SELECT * FROM host_packages",
        )
        .await
        .unwrap();
        db.execute_unprepared("DROP TABLE host_packages")
            .await
            .unwrap();

        // The next Migrator::up call must not crash.
        Migrator::up(&db, None).await.expect(
            "migration must succeed when only host_packages_new exists (State C)",
        );
    }

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
        // Verify has_update generated column exists.
        db.execute_unprepared("SELECT has_update FROM host_packages LIMIT 0")
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
