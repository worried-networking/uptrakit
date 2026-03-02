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
mod m20260302_000004_service_cert_lifetime;
mod m20260307_000002_manage_commands_permission;

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
            Box::new(m20260302_000004_service_cert_lifetime::Migration),
            Box::new(m20260307_000002_manage_commands_permission::Migration),
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
    use sea_orm::{ConnectOptions, Database, EntityTrait as _, PaginatorTrait as _};

    use crate::entity::{
        crl_cache, global_setting, host_discovery_allowlist, host_package, host_package_ignore,
        host_package_update_history, host_software_item, notification_channel, notification_log,
        notification_rule, plugin_config, revoked_token_jti, revoked_token_user, service,
        software_item, tenant_discovery_allowlist, update_batch, update_history,
    };

    /// Verify that the `has_update` generated column exists in `host_packages`.
    ///
    /// `has_update` is a SQLite generated column and is not part of the
    /// `host_package::Model` entity. It must be checked via a sea_query SELECT
    /// rather than an entity query.
    async fn assert_has_update_column_exists(db: &DatabaseConnection) {
        let stmt = Query::select()
            .from(Alias::new("host_packages"))
            .column(Alias::new("has_update"))
            .limit(0)
            .to_owned();
        db.query_all(&stmt)
            .await
            .expect("has_update generated column must exist in host_packages");
    }

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
        assert_has_update_column_exists(&db).await;
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
        //
        // `CREATE TABLE … AS SELECT *` is a SQLite-specific shorthand that
        // snapshots the live schema at runtime. sea_query has no equivalent
        // builder for this construct, so execute_unprepared is the only option
        // here. This is an approved exception: the sole purpose of this
        // statement is to replicate the exact mid-migration crash state that
        // m20260302_000003 is designed to recover from.
        db.execute_unprepared(
            "CREATE TABLE host_packages_new AS SELECT * FROM host_packages",
        )
        .await
        .unwrap();

        // The next Migrator::up call must not crash.
        Migrator::up(&db, None).await.expect(
            "migration must succeed even when host_packages_new already exists alongside original",
        );
        assert_has_update_column_exists(&db).await;
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
        // See the State B comment above for why execute_unprepared is used here.
        db.execute_unprepared(
            "CREATE TABLE host_packages_new AS SELECT * FROM host_packages",
        )
        .await
        .unwrap();

        // Drop the original table using the sea_query builder.
        let drop_stmt = Table::drop()
            .table(Alias::new("host_packages"))
            .to_owned();
        db.execute(&drop_stmt).await.unwrap();

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

        // Verify tables added by various migrations exist and are queryable.
        software_item::Entity::find().count(&db).await.unwrap();
        plugin_config::Entity::find().count(&db).await.unwrap();
        tenant_discovery_allowlist::Entity::find().count(&db).await.unwrap();
        host_discovery_allowlist::Entity::find().count(&db).await.unwrap();
        notification_channel::Entity::find().count(&db).await.unwrap();
        notification_rule::Entity::find().count(&db).await.unwrap();
        notification_log::Entity::find().count(&db).await.unwrap();
        global_setting::Entity::find().count(&db).await.unwrap();
        revoked_token_jti::Entity::find().count(&db).await.unwrap();
        revoked_token_user::Entity::find().count(&db).await.unwrap();
        crl_cache::Entity::find().count(&db).await.unwrap();

        // Entity count queries verify that each table exists and is queryable.
        host_software_item::Entity::find().count(&db).await.unwrap();
        update_history::Entity::find().count(&db).await.unwrap();
        update_batch::Entity::find().count(&db).await.unwrap();
        host_package::Entity::find().count(&db).await.unwrap();
        host_package_ignore::Entity::find().count(&db).await.unwrap();
        host_package_update_history::Entity::find().count(&db).await.unwrap();

        // `has_update` is a SQLite generated column not part of the entity
        // model; verify it exists via a targeted sea_query SELECT.
        assert_has_update_column_exists(&db).await;

        service::Entity::find().count(&db).await.unwrap();

        // Verify split_version_check migration: detect_version task row exists.
        let count_stmt = Query::select()
            .expr(Func::count(Expr::col(Alias::new("id"))))
            .from(Alias::new("scheduled_tasks"))
            .and_where(Expr::col(Alias::new("task_type")).eq("detect_version"))
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

        // Verify manage_commands permission was created and assigned.
        let perm_count_stmt = sea_orm_migration::prelude::Query::select()
            .expr(sea_orm_migration::prelude::Func::count(
                sea_orm_migration::prelude::Expr::col(Alias::new("id")),
            ))
            .from(Alias::new("permissions"))
            .and_where(
                sea_orm_migration::prelude::Expr::col(Alias::new("name"))
                    .eq("manage_commands"),
            )
            .to_owned();
        let perm_rows = db.query_all(&perm_count_stmt).await.unwrap();
        let perm_count: i64 = {
            use sea_orm::TryGetable;
            perm_rows
                .first()
                .map(|r| i64::try_get_by_index(r, 0).unwrap_or(0))
                .unwrap_or(0)
        };
        assert_eq!(
            perm_count, 1,
            "manage_commands permission must exist after all migrations"
        );
    }

    /// After all migrations, owner and admin roles must have manage_commands.
    #[tokio::test]
    async fn manage_commands_assigned_to_owner_and_admin() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        for role_name in ["owner", "admin"] {
            let count_stmt = sea_orm_migration::prelude::Query::select()
                .expr(sea_orm_migration::prelude::Func::count(
                    sea_orm_migration::prelude::Expr::col(Alias::new("rp.role_id")),
                ))
                .from_as(Alias::new("role_permissions"), Alias::new("rp"))
                .join_as(
                    sea_orm_migration::prelude::JoinType::InnerJoin,
                    Alias::new("roles"),
                    Alias::new("r"),
                    sea_orm_migration::prelude::Expr::col(
                        (Alias::new("r"), Alias::new("id")),
                    )
                    .equals((Alias::new("rp"), Alias::new("role_id"))),
                )
                .join_as(
                    sea_orm_migration::prelude::JoinType::InnerJoin,
                    Alias::new("permissions"),
                    Alias::new("p"),
                    sea_orm_migration::prelude::Expr::col(
                        (Alias::new("p"), Alias::new("id")),
                    )
                    .equals((Alias::new("rp"), Alias::new("permission_id"))),
                )
                .and_where(
                    sea_orm_migration::prelude::Expr::col((Alias::new("r"), Alias::new("name")))
                        .eq(role_name),
                )
                .and_where(
                    sea_orm_migration::prelude::Expr::col((Alias::new("p"), Alias::new("name")))
                        .eq("manage_commands"),
                )
                .to_owned();

            let rows = db.query_all(&count_stmt).await.unwrap();
            let count: i64 = {
                use sea_orm::TryGetable;
                rows.first()
                    .map(|r| i64::try_get_by_index(r, 0).unwrap_or(0))
                    .unwrap_or(0)
            };
            assert_eq!(
                count, 1,
                "{role_name} role must have manage_commands permission after all migrations"
            );
        }
    }

    /// The user role must NOT have manage_commands.
    #[tokio::test]
    async fn manage_commands_not_assigned_to_user_role() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        let count_stmt = sea_orm_migration::prelude::Query::select()
            .expr(sea_orm_migration::prelude::Func::count(
                sea_orm_migration::prelude::Expr::col(Alias::new("rp.role_id")),
            ))
            .from_as(Alias::new("role_permissions"), Alias::new("rp"))
            .join_as(
                sea_orm_migration::prelude::JoinType::InnerJoin,
                Alias::new("roles"),
                Alias::new("r"),
                sea_orm_migration::prelude::Expr::col(
                    (Alias::new("r"), Alias::new("id")),
                )
                .equals((Alias::new("rp"), Alias::new("role_id"))),
            )
            .join_as(
                sea_orm_migration::prelude::JoinType::InnerJoin,
                Alias::new("permissions"),
                Alias::new("p"),
                sea_orm_migration::prelude::Expr::col(
                    (Alias::new("p"), Alias::new("id")),
                )
                .equals((Alias::new("rp"), Alias::new("permission_id"))),
            )
            .and_where(
                sea_orm_migration::prelude::Expr::col((Alias::new("r"), Alias::new("name")))
                    .eq("user"),
            )
            .and_where(
                sea_orm_migration::prelude::Expr::col((Alias::new("p"), Alias::new("name")))
                    .eq("manage_commands"),
            )
            .to_owned();

        let rows = db.query_all(&count_stmt).await.unwrap();
        let count: i64 = {
            use sea_orm::TryGetable;
            rows.first()
                .map(|r| i64::try_get_by_index(r, 0).unwrap_or(0))
                .unwrap_or(0)
        };
        assert_eq!(
            count, 0,
            "user role must NOT have manage_commands permission"
        );
    }
}
