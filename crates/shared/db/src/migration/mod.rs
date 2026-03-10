use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

pub mod helpers;

mod m20260209_000001_initial;
mod m20260227_000001_drop_controller_events;
mod m20260227_000002_remove_event_cleanup_tasks;
mod m20260227_000003_discovery_allowlist;
mod m20260301_000001_notifications;
mod m20260302_000001_add_missing_indexes;
mod m20260302_000002_host_packages;
mod m20260302_000003_host_packages_has_update;
mod m20260302_000004_service_cert_lifetime;
mod m20260302_000005_system_services;
mod m20260303_000001_global_settings;
mod m20260303_000002_revoked_tokens;
mod m20260303_000003_audit_logs;
mod m20260305_000001_crl_cache;
mod m20260305_000002_service_app_name;
mod m20260306_000001_update_category;
mod m20260306_000002_update_batches;
mod m20260307_000001_split_version_check;
mod m20260307_000002_manage_commands_permission;
mod m20260308_000001_system_services_permissions;
mod m20260308_000002_fix_permission_uuid_storage;
mod m20260308_000003_proxmox_hm_pagination_indexes;
mod m20260309_000001_fix_permission_created_at_format;
mod m20260309_000002_simplify_autodiscovery_ignores;
mod m20260309_000003_host_tags;
mod m20260309_000003_unified_software_tracking;
mod m20260310_000001_data_encryption_keys;
mod m20260311_000001_update_history_status_index;
mod m20260311_000002_audit_log_permissions;
mod m20260312_000001_system_enrollment_tokens;
mod m20260312_000002_discover_host_packages_task;
mod m20260313_000001_per_host_update_locking;
mod m20260314_000001_proxmox_host_mapping;
mod m20260315_000001_proxmox_hm_machine_id;
mod m20260316_000001_host_machine_id_partial_unique;
mod m20260317_000001_fix_hosts_count_desync;
mod m20260318_000001_host_software_item_qualifier;
mod m20260318_000002_cron_to_interval;
mod m20260319_000001_software_items_sort_index;

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
            Box::new(m20260302_000005_system_services::Migration),
            Box::new(m20260307_000002_manage_commands_permission::Migration),
            Box::new(m20260308_000001_system_services_permissions::Migration),
            Box::new(m20260308_000002_fix_permission_uuid_storage::Migration),
            Box::new(m20260309_000001_fix_permission_created_at_format::Migration),
            Box::new(m20260303_000003_audit_logs::Migration),
            Box::new(m20260310_000001_data_encryption_keys::Migration),
            Box::new(m20260311_000001_update_history_status_index::Migration),
            Box::new(m20260311_000002_audit_log_permissions::Migration),
            Box::new(m20260312_000001_system_enrollment_tokens::Migration),
            Box::new(m20260312_000002_discover_host_packages_task::Migration),
            Box::new(m20260313_000001_per_host_update_locking::Migration),
            Box::new(m20260305_000002_service_app_name::Migration),
            Box::new(m20260314_000001_proxmox_host_mapping::Migration),
            Box::new(m20260315_000001_proxmox_hm_machine_id::Migration),
            Box::new(m20260316_000001_host_machine_id_partial_unique::Migration),
            Box::new(m20260317_000001_fix_hosts_count_desync::Migration),
            Box::new(m20260308_000003_proxmox_hm_pagination_indexes::Migration),
            Box::new(m20260318_000001_host_software_item_qualifier::Migration),
            Box::new(m20260318_000002_cron_to_interval::Migration),
            Box::new(m20260309_000002_simplify_autodiscovery_ignores::Migration),
            Box::new(m20260309_000003_host_tags::Migration),
            Box::new(m20260309_000003_unified_software_tracking::Migration),
            Box::new(m20260319_000001_software_items_sort_index::Migration),
        ]
    }
}

/// Run all pending migrations.
///
/// All migrations are executed inside a single database transaction so that
/// every DDL statement runs on the same physical connection. Without this,
/// a connection pool may dispatch successive DDL calls to different SQLite
/// connections, where schema changes made on one connection (e.g. `DROP
/// TABLE`) are not yet visible to the next connection (e.g. `ALTER TABLE …
/// RENAME TO`), causing spurious *"table or index already exists"* errors on
/// startup.
///
/// For PostgreSQL, `sea-orm-migration` wraps the run in a transaction
/// internally; our outer `begin`/`commit` becomes a harmless extra savepoint.
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    use sea_orm::TransactionTrait;
    let txn = db.begin().await?;
    Migrator::up(&txn, None).await?;
    txn.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ColumnTrait as _, ConnectOptions, Database, EntityTrait as _, PaginatorTrait as _,
        QueryFilter as _,
    };

    use crate::entity::{
        audit_log, crl_cache, data_encryption_key, global_setting, host_discovery_allowlist,
        host_software_item, host_tag, host_tag_assignment, notification_channel, notification_log,
        notification_rule, plugin_config, proxmox_host_mapping, revoked_token_jti,
        revoked_token_user, role, role_permission, service, software_ignore, software_item,
        system_audit_log, system_enrollment_token, system_service, system_service_certificate,
        tenant_discovery_allowlist, update_batch, update_history,
    };

    /// Simulate the "existing database" upgrade scenario:
    /// the first twelve migrations are applied in a first run, then the
    /// remaining migrations are applied in a second run.
    #[tokio::test]
    async fn migrations_run_incrementally_sqlite() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        Migrator::up(&db, Some(12))
            .await
            .expect("first 12 migrations should succeed");
        Migrator::up(&db, None)
            .await
            .expect("remaining migrations should succeed on existing database");
    }

    #[tokio::test]
    async fn migrations_run_on_empty_sqlite() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        // Verify tables added by various migrations exist and are queryable.
        software_item::Entity::find().count(&db).await.unwrap();
        plugin_config::Entity::find().count(&db).await.unwrap();
        tenant_discovery_allowlist::Entity::find()
            .count(&db)
            .await
            .unwrap();
        host_discovery_allowlist::Entity::find()
            .count(&db)
            .await
            .unwrap();
        notification_channel::Entity::find()
            .count(&db)
            .await
            .unwrap();
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
        software_ignore::Entity::find().count(&db).await.unwrap();

        service::Entity::find().count(&db).await.unwrap();
        system_service::Entity::find().count(&db).await.unwrap();
        system_service_certificate::Entity::find()
            .count(&db)
            .await
            .unwrap();
        data_encryption_key::Entity::find()
            .count(&db)
            .await
            .unwrap();

        // Verify audit_logs and system_audit_logs tables exist.
        audit_log::Entity::find().count(&db).await.unwrap();
        system_audit_log::Entity::find().count(&db).await.unwrap();

        // Verify system_enrollment_tokens table exists.
        system_enrollment_token::Entity::find()
            .count(&db)
            .await
            .unwrap();

        // Verify proxmox_host_mappings table exists.
        proxmox_host_mapping::Entity::find()
            .count(&db)
            .await
            .unwrap();

        // Verify host_tags and host_tag_assignments tables exist.
        host_tag::Entity::find().count(&db).await.unwrap();
        host_tag_assignment::Entity::find()
            .count(&db)
            .await
            .unwrap();

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

        // Verify discover_software task row exists (renamed from discover_host_packages).
        let dhp_stmt = Query::select()
            .expr(Func::count(Expr::col(Alias::new("id"))))
            .from(Alias::new("scheduled_tasks"))
            .and_where(Expr::col(Alias::new("task_type")).eq("discover_software"))
            .to_owned();
        let dhp_rows = db.query_all(&dhp_stmt).await.unwrap();
        let dhp_count: i64 = {
            use sea_orm::TryGetable;
            dhp_rows
                .first()
                .map(|r| i64::try_get_by_index(r, 0).unwrap_or(0))
                .unwrap_or(0)
        };
        assert!(
            dhp_count >= 1,
            "expected at least one discover_software task after migration, found {dhp_count}"
        );

        // Verify manage_commands permission was created and assigned.
        let perm_count_stmt = sea_orm_migration::prelude::Query::select()
            .expr(sea_orm_migration::prelude::Func::count(
                sea_orm_migration::prelude::Expr::col(Alias::new("id")),
            ))
            .from(Alias::new("permissions"))
            .and_where(
                sea_orm_migration::prelude::Expr::col(Alias::new("name")).eq("manage_commands"),
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

        // Verify view_system_services and manage_system_services permissions exist.
        for perm_name in ["view_system_services", "manage_system_services"] {
            let ss_perm_stmt = sea_orm_migration::prelude::Query::select()
                .expr(sea_orm_migration::prelude::Func::count(
                    sea_orm_migration::prelude::Expr::col(Alias::new("id")),
                ))
                .from(Alias::new("permissions"))
                .and_where(sea_orm_migration::prelude::Expr::col(Alias::new("name")).eq(perm_name))
                .to_owned();
            let ss_rows = db.query_all(&ss_perm_stmt).await.unwrap();
            let ss_count: i64 = {
                use sea_orm::TryGetable;
                ss_rows
                    .first()
                    .map(|r| i64::try_get_by_index(r, 0).unwrap_or(0))
                    .unwrap_or(0)
            };
            assert_eq!(
                ss_count, 1,
                "{perm_name} permission must exist after all migrations"
            );
        }
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
                    sea_orm_migration::prelude::Expr::col((Alias::new("r"), Alias::new("id")))
                        .equals((Alias::new("rp"), Alias::new("role_id"))),
                )
                .join_as(
                    sea_orm_migration::prelude::JoinType::InnerJoin,
                    Alias::new("permissions"),
                    Alias::new("p"),
                    sea_orm_migration::prelude::Expr::col((Alias::new("p"), Alias::new("id")))
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

    /// After all migrations, the `role_permissions` entity query must succeed.
    ///
    /// This catches the TEXT/BLOB UUID mismatch: if any `permission_id` in
    /// `role_permissions` is stored as a 36-char TEXT string, SeaORM fails
    /// with `ParseByteLength { len: 36 }` when loading `role_permission::Model`.
    #[tokio::test]
    async fn role_permissions_entity_query_succeeds() {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        role_permission::Entity::find()
            .all(&db)
            .await
            .expect("role_permissions entity query must succeed after all migrations");
    }

    /// Verify that the repair migration converts TEXT-stored UUIDs to BLOBs.
    ///
    /// Steps:
    /// 1. Apply migrations 1–18 (all except the repair migration).
    /// 2. Manually inject a permission row with a TEXT-stored UUID and a
    ///    matching `role_permissions` row.
    /// 3. Apply migration 19 (the repair).
    /// 4. Assert `typeof(id) = 'blob'` for the injected permission.
    /// 5. Assert the entity query still succeeds.
    #[tokio::test]
    async fn repair_migration_fixes_text_uuid_storage() {
        use sea_orm::{ConnectionTrait as _, Statement, TryGetable as _};

        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();

        // Apply migrations 1–18 (skip the repair migration at index 19).
        Migrator::up(&db, Some(18))
            .await
            .expect("first 18 migrations should succeed");

        // Resolve the owner role via the entity API — no raw SQL needed.
        let owner_role = role::Entity::find()
            .filter(role::Column::Name.eq("owner"))
            .one(&db)
            .await
            .unwrap()
            .expect("owner role must exist after migrations");

        // Inject a TEXT-stored permission — simulating the pre-fix behaviour.
        //
        // Passing a String value causes sea-query to bind it as Value::String,
        // which SQLite stores as TEXT in the id column (a UUID/BLOB column).
        // This replicates what the buggy execute_unprepared migrations did.
        let broken_uuid = "018f1234-0000-7000-8000-000000000001";
        db.execute(
            &Query::insert()
                .into_table(Alias::new("permissions"))
                .columns([
                    Alias::new("id"),
                    Alias::new("name"),
                    Alias::new("description"),
                    Alias::new("created_at"),
                ])
                .values_panic([
                    broken_uuid.to_owned().into(), // String → VALUE::String → SQLite TEXT
                    "test_broken_perm".into(),
                    "repair test".into(),
                    "2026-01-01T00:00:00Z".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("injecting TEXT-uuid permission must succeed");

        // Inject a role_permissions row referencing the TEXT uuid.
        //
        // role_id is Uuid → Value::Uuid → BLOB (correct storage).
        // permission_id is String → Value::String → TEXT (the broken state).
        db.execute(
            &Query::insert()
                .into_table(Alias::new("role_permissions"))
                .columns([Alias::new("role_id"), Alias::new("permission_id")])
                .values_panic([
                    owner_role.id.into(),          // Uuid → BLOB
                    broken_uuid.to_owned().into(), // String → TEXT
                ])
                .to_owned(),
        )
        .await
        .expect("injecting TEXT-uuid role_permission must succeed");

        // Confirm the injection was TEXT before the repair.
        //
        // `typeof()` is a SQLite-specific function with no sea_query equivalent;
        // query_one_raw with a Statement is the approved exception for this.
        let typeof_before = db
            .query_one_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT typeof(id) FROM permissions WHERE name = 'test_broken_perm'",
            ))
            .await
            .unwrap()
            .expect("injected row must exist");
        let type_str_before: String = String::try_get_by_index(&typeof_before, 0).unwrap();
        assert_eq!(type_str_before, "text", "pre-condition: id should be TEXT");

        // Apply the remaining migration (the repair, index 19).
        Migrator::up(&db, None)
            .await
            .expect("repair migration must succeed");

        // After the repair, typeof(id) must be 'blob'.
        let typeof_after = db
            .query_one_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT typeof(id) FROM permissions WHERE name = 'test_broken_perm'",
            ))
            .await
            .unwrap()
            .expect("repaired row must still exist");
        let type_str_after: String = String::try_get_by_index(&typeof_after, 0).unwrap();
        assert_eq!(
            type_str_after, "blob",
            "after repair: permission id must be BLOB"
        );

        // The entity query must now succeed without ParseByteLength errors.
        role_permission::Entity::find()
            .all(&db)
            .await
            .expect("role_permissions entity query must succeed after repair migration");
    }

    /// Verify that the datetime repair migration converts `+00:00:00`-formatted
    /// `created_at` values to RFC 3339, making the row decodable by SeaORM.
    ///
    /// Steps:
    /// 1. Apply migrations 1–19 (all except the datetime repair).
    /// 2. Inject a permission with `created_at = '2026-03-02 22:33:15.239039 +00:00:00'`.
    /// 3. Apply migration 20 (the datetime repair).
    /// 4. Load the permission via the entity API — this decodes `OffsetDateTime`
    ///    and fails if the format is still broken.
    #[tokio::test]
    async fn repair_migration_fixes_created_at_format() {
        use crate::entity::permission;

        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();

        // Apply all migrations except the datetime repair (index 20).
        Migrator::up(&db, Some(19))
            .await
            .expect("first 19 migrations should succeed");

        // Inject a permission row with the broken created_at format.
        //
        // The id must be a BLOB (uuid.into()) because migration 19 (UUID repair)
        // has already run and FK checks expect BLOB ids in role_permissions.
        let perm_id = uuid::Uuid::now_v7();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("permissions"))
                .columns([
                    Alias::new("id"),
                    Alias::new("name"),
                    Alias::new("description"),
                    Alias::new("created_at"),
                ])
                .values_panic([
                    perm_id.into(),
                    "test_broken_datetime".into(),
                    "datetime repair test".into(),
                    // The exact format produced by time::OffsetDateTime::Display for UTC.
                    "2026-03-02 22:33:15.239039 +00:00:00".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("injecting broken-datetime permission must succeed");

        // Apply the datetime repair migration.
        Migrator::up(&db, None)
            .await
            .expect("datetime repair migration must succeed");

        // Loading the permission via the entity API decodes OffsetDateTime.
        // This fails with ColumnDecode if the format is still broken.
        permission::Entity::find()
            .filter(permission::Column::Name.eq("test_broken_datetime"))
            .one(&db)
            .await
            .expect("permission entity query must succeed after datetime repair")
            .expect("injected permission must still exist after repair");
    }

    /// Regression test: `run_migrations` must succeed against a file-based
    /// SQLite database even when the connection pool has multiple connections.
    ///
    /// The previous implementation called `Migrator::up(db, None)` directly,
    /// which allowed `sea-orm-migration` to dispatch successive DDL statements
    /// to different physical connections in the pool. On SQLite this caused
    /// spurious *"table or index already exists"* errors because schema changes
    /// made on one connection were not immediately visible to another connection
    /// through the pool's stale schema cache.
    ///
    /// The fix wraps `Migrator::up` in a transaction so all DDL statements
    /// share a single connection. This test pins the bug by using a real file
    /// (not `:memory:`) and a pool of 10 connections (the production default).
    #[tokio::test]
    async fn run_migrations_file_sqlite_pool() {
        let dir = tempfile::tempdir().expect("tmp dir");
        let db_path = dir.path().join("test.db");
        let url = format!("sqlite://{}?mode=rwc", db_path.display());

        let mut opt = ConnectOptions::new(url);
        opt.max_connections(10).min_connections(1);
        let db = Database::connect(opt).await.unwrap();

        run_migrations(&db)
            .await
            .expect("run_migrations must succeed on a file-based SQLite pool");

        // Verify the schema is fully usable after all migrations.
        software_item::Entity::find().count(&db).await.unwrap();
        software_ignore::Entity::find().count(&db).await.unwrap();
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
                sea_orm_migration::prelude::Expr::col((Alias::new("r"), Alias::new("id")))
                    .equals((Alias::new("rp"), Alias::new("role_id"))),
            )
            .join_as(
                sea_orm_migration::prelude::JoinType::InnerJoin,
                Alias::new("permissions"),
                Alias::new("p"),
                sea_orm_migration::prelude::Expr::col((Alias::new("p"), Alias::new("id")))
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
