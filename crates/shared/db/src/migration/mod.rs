use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;
use uptrakit_db_tx::begin_immediate;

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
mod m20260309_000001_fix_permission_created_at_format;
mod m20260309_000002_simplify_autodiscovery_ignores;
mod m20260309_000003_host_tags;
mod m20260309_000003_unified_software_tracking;
mod m20260310_000001_data_encryption_keys;
mod m20260310_000002_granular_permissions;
mod m20260311_000001_update_history_status_index;
mod m20260311_000002_audit_log_permissions;
mod m20260311_000003_rename_extra_sans_to_sans;
mod m20260312_000001_system_enrollment_tokens;
mod m20260312_000002_discover_host_packages_task;
mod m20260312_000003_plugin_type_settings;
mod m20260313_000001_per_host_update_locking;
mod m20260316_000001_host_machine_id_partial_unique;
mod m20260317_000001_fix_hosts_count_desync;
mod m20260317_000002_test_plugin_configs_permission;
mod m20260318_000001_host_software_item_qualifier;
mod m20260318_000002_cron_to_interval;
mod m20260319_000001_software_items_sort_index;
mod m20260320_000001_update_history_interactive;
mod m20260321_000001_software_items_icon_url;
mod m20260321_000002_updates_queue;
mod m20260322_000001_hosts_lower_name_index;
mod m20260322_000002_hsi_updatable_index;
mod m20260322_000003_update_history_truncated;
mod m20260323_000001_notification_permissions;
mod m20260324_000001_hsi_installed_display_version;
mod m20260325_000001_hsip_plugin_type_index;
mod m20260326_000001_hsip_role_ordinal_index;
mod m20260328_000001_mqtt_states_pagination_indexes;
mod m20260329_000001_drop_mqtt_and_add_service_config;
mod m20260330_000001_embedded_service_visibility;
mod m20260331_000002_agent_ssh_migration_history_repair;
mod m20260401_000001_host_features;
mod m20260410_000001_oidc_private_network_issuers;
mod m20260414_000001_update_execution_ownership;
mod m20260416_000001_update_history_protection;
mod m20260417_000001_semantic_audit_logs;
mod m20260422_000001_email_change_request;
pub(super) mod m20260423_000001_permission_wire_safe;
pub(super) mod m20260424_000001_access_mcp_permission;
mod m20260430_000001_awaiting_restart_timeout;
mod m20260430_000002_awaiting_restart_since;
mod m20260430_000003_update_history_host_active_index;
mod m20260510_000001_instance_plugin_setting;
mod m20260512_000001_device_flow_rfc8628;
mod m20260512_000001_drop_file_keys;
mod m20260513_000001_oauth_clients;
mod m20260513_000002_oauth_consents;
mod m20260513_000003_oauth_authorization_requests;
mod m20260513_000004_oauth_authorization_codes;
mod m20260513_000005_oauth_refresh_tokens;
mod m20260513_000006_oauth_controller_instances;
mod m20260514_000001_audit_logs_v2;
mod m20260515_000001_normalize_cert_serial_uppercase;
mod m20260515_000002_update_history_item_active_index;
mod m20260516_000001_2fa;
mod m20260610_000001_service_merge_redirect;
mod m20260702_000001_hsi_discovery_provenance;
mod m20260727_000001_plugin_type_id_grammar;
mod m20260728_000001_access_grants_and_role_scope;
mod m20260728_000002_seed_access_grants;
mod m20260803_000001_seed_mcp_use_grants;
mod m20260807_000001_drop_permissions_tables;
mod m20260811_000001_materialize_mcp_enabled;
mod m20260811_000002_pending_flow_snapshot;
mod m20260812_000001_encrypt_plugin_configs_config;
mod m20260812_000002_encrypt_plugin_type_settings_config;
mod m20260812_000003_encrypt_instance_plugin_setting_config;

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
            Box::new(m20260316_000001_host_machine_id_partial_unique::Migration),
            Box::new(m20260317_000001_fix_hosts_count_desync::Migration),
            Box::new(m20260318_000001_host_software_item_qualifier::Migration),
            Box::new(m20260318_000002_cron_to_interval::Migration),
            Box::new(m20260309_000002_simplify_autodiscovery_ignores::Migration),
            Box::new(m20260309_000003_host_tags::Migration),
            Box::new(m20260309_000003_unified_software_tracking::Migration),
            Box::new(m20260319_000001_software_items_sort_index::Migration),
            Box::new(m20260310_000002_granular_permissions::Migration),
            Box::new(m20260320_000001_update_history_interactive::Migration),
            Box::new(m20260311_000003_rename_extra_sans_to_sans::Migration),
            Box::new(m20260321_000001_software_items_icon_url::Migration),
            Box::new(m20260312_000003_plugin_type_settings::Migration),
            Box::new(m20260321_000002_updates_queue::Migration),
            Box::new(m20260322_000001_hosts_lower_name_index::Migration),
            Box::new(m20260322_000002_hsi_updatable_index::Migration),
            Box::new(m20260322_000003_update_history_truncated::Migration),
            Box::new(m20260323_000001_notification_permissions::Migration),
            Box::new(m20260324_000001_hsi_installed_display_version::Migration),
            Box::new(m20260325_000001_hsip_plugin_type_index::Migration),
            Box::new(m20260326_000001_hsip_role_ordinal_index::Migration),
            Box::new(m20260328_000001_mqtt_states_pagination_indexes::Migration),
            Box::new(m20260329_000001_drop_mqtt_and_add_service_config::Migration),
            Box::new(m20260330_000001_embedded_service_visibility::Migration),
            Box::new(m20260331_000002_agent_ssh_migration_history_repair::Migration),
            Box::new(m20260317_000002_test_plugin_configs_permission::Migration),
            Box::new(m20260401_000001_host_features::Migration),
            Box::new(m20260410_000001_oidc_private_network_issuers::Migration),
            Box::new(m20260414_000001_update_execution_ownership::Migration),
            Box::new(m20260416_000001_update_history_protection::Migration),
            Box::new(m20260417_000001_semantic_audit_logs::Migration),
            Box::new(m20260422_000001_email_change_request::Migration),
            Box::new(m20260423_000001_permission_wire_safe::Migration),
            Box::new(m20260424_000001_access_mcp_permission::Migration),
            Box::new(m20260430_000001_awaiting_restart_timeout::Migration),
            Box::new(m20260430_000002_awaiting_restart_since::Migration),
            Box::new(m20260430_000003_update_history_host_active_index::Migration),
            Box::new(m20260510_000001_instance_plugin_setting::Migration),
            Box::new(m20260512_000001_device_flow_rfc8628::Migration),
            Box::new(m20260513_000001_oauth_clients::Migration),
            Box::new(m20260513_000002_oauth_consents::Migration),
            Box::new(m20260513_000003_oauth_authorization_requests::Migration),
            Box::new(m20260513_000004_oauth_authorization_codes::Migration),
            Box::new(m20260513_000005_oauth_refresh_tokens::Migration),
            Box::new(m20260513_000006_oauth_controller_instances::Migration),
            Box::new(m20260514_000001_audit_logs_v2::Migration),
            Box::new(m20260512_000001_drop_file_keys::Migration),
            Box::new(m20260515_000001_normalize_cert_serial_uppercase::Migration),
            Box::new(m20260515_000002_update_history_item_active_index::Migration),
            Box::new(m20260516_000001_2fa::Migration),
            Box::new(m20260610_000001_service_merge_redirect::Migration),
            Box::new(m20260702_000001_hsi_discovery_provenance::Migration),
            Box::new(m20260727_000001_plugin_type_id_grammar::Migration),
            Box::new(m20260728_000001_access_grants_and_role_scope::Migration),
            Box::new(m20260728_000002_seed_access_grants::Migration),
            Box::new(m20260803_000001_seed_mcp_use_grants::Migration),
            Box::new(m20260807_000001_drop_permissions_tables::Migration),
            Box::new(m20260812_000001_encrypt_plugin_configs_config::Migration),
            Box::new(m20260812_000002_encrypt_plugin_type_settings_config::Migration),
            Box::new(m20260812_000003_encrypt_instance_plugin_setting_config::Migration),
            Box::new(m20260811_000001_materialize_mcp_enabled::Migration),
            Box::new(m20260811_000002_pending_flow_snapshot::Migration),
        ]
    }
}

/// Run all pending core migrations (without plugin-contributed migrations).
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
///
/// For the controller, prefer [`run_migrations_with_plugins`] to also include
/// plugin-contributed migrations.
///
/// # Errors
///
/// Returns any [`sea_orm::DbErr`] from opening the transaction, applying a
/// migration, or committing.
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    run_migrations_with_plugins(db, Vec::new).await
}

/// Run all pending migrations one at a time, reporting which migration fails.
///
/// Useful for debugging migration failures on PostgreSQL, where a failed
/// statement inside a transaction causes all subsequent statements to fail with
/// `25P02` ("current transaction is aborted"), masking the original error.
///
/// Each migration runs in its own implicit transaction (SeaORM's default for PG),
/// so only the failing migration's error is reported.
///
/// **Not intended for production use.** Use [`run_migrations`] instead.
pub async fn run_migrations_debug(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    use sea_orm::ConnectionTrait;
    let migrations = Migrator::migrations();
    let total = migrations.len();
    for i in 0..total {
        match Migrator::up(db, Some(1)).await {
            Ok(()) => {}
            Err(e) => {
                // On PG, the error might be 25P02 (cascading from previous statement
                // in the same migration). Report which migration was being attempted.
                let name = migrations
                    .get(i)
                    .map(|m| m.name().to_string())
                    .unwrap_or_else(|| format!("migration #{i}"));
                // Try to clear the aborted transaction state by rolling back.
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "best-effort ROLLBACK to clear aborted transaction state; failure is intentionally ignored"
                )]
                #[expect(
                    clippy::disallowed_methods,
                    reason = "builder limitation: ROLLBACK is a transaction-control verb with no sea_query builder"
                )]
                let _ = db.execute_unprepared("ROLLBACK").await;
                return Err(sea_orm::DbErr::Custom(format!(
                    "migration {name} (#{}) failed: {e}",
                    i + 1
                )));
            }
        }
    }
    Ok(())
}

/// Run all pending migrations, including plugin-contributed ones.
///
/// Core and plugin migrations are combined into a single migrator so that
/// SeaORM sees the complete migration list and does not error on "missing"
/// entries.
///
/// Strategy by backend: file-backed SQLite runs on a dedicated
/// single-connection pool born with `foreign_keys=false`, followed by a
/// post-commit `PRAGMA foreign_key_check` gate; in-memory SQLite runs inside
/// the caller-pool transaction (FK ON) and is gated by the same check;
/// PostgreSQL runs inside a plain transaction, unchanged. On SQLite,
/// sea-orm-migration runs each individual migration with `use_txn = false`
/// (`should_use_transaction` is Postgres-only, `exec.rs:184–188` in the
/// pinned crate), so the runner's outer transaction is the ONLY transaction
/// and provides all-or-nothing atomicity.
///
/// `plugin_provider` is a closure (not a `Vec`) because
/// `Box<dyn MigrationTrait>` is not `Clone` and sea-orm-migration may call
/// `migrations()` more than once per run — each call must regenerate the
/// full list.
///
/// # Errors
///
/// Returns any [`sea_orm::DbErr`] from opening the transaction, applying a
/// migration, or committing. Also returns `DbErr::Custom` when the
/// post-commit `foreign_key_check` finds violations, and connection errors
/// from building the dedicated pool.
pub async fn run_migrations_with_plugins(
    db: &DatabaseConnection,
    plugin_provider: impl Fn() -> Vec<Box<dyn MigrationTrait>> + Send + Sync + 'static,
) -> Result<(), sea_orm::DbErr> {
    let migrator = CombinedMigrator {
        plugin_provider: Box::new(plugin_provider),
    };

    // Backend gate BEFORE get_sqlite_connection_pool(): that accessor
    // panics on non-SQLite backends.
    #[cfg(feature = "db-sqlite")]
    if db.get_database_backend() == sea_orm::DbBackend::Sqlite {
        if sqlite_main_db_file(db).await?.is_some() {
            return run_on_dedicated_sqlite_pool(db, &migrator).await;
        }
        // In-memory: a second pool would open a DIFFERENT empty database,
        // so keep the caller-pool transaction. FK stays ON here —
        // violations fail loudly at statement time — and the post-commit
        // gate still runs. (No currently-recreated parent table has
        // cascade-child rows populated by earlier migrations; the
        // file-backed production path is immune by construction.)
        use sea_orm_migration::MigratorTraitSelf as _;
        let txn = begin_immediate(db).await?;
        migrator.up(&txn, None).await?;
        txn.commit().await?;
        return sqlite_foreign_key_check(db).await;
    }

    // PostgreSQL (and any non-SQLite backend): unchanged.
    use sea_orm_migration::MigratorTraitSelf as _;
    let txn = begin_immediate(db).await?;
    migrator.up(&txn, None).await?;
    txn.commit().await
}

/// Migrator that combines core + plugin migrations into one list.
///
/// Instance-based (`MigratorTraitSelf`), so each run owns its migrator and
/// there is no cross-task state: no thread-local, no global slot, and
/// concurrent runs with different providers are trivially safe.
struct CombinedMigrator {
    plugin_provider: Box<dyn Fn() -> Vec<Box<dyn MigrationTrait>> + Send + Sync>,
}

impl sea_orm_migration::MigratorTraitSelf for CombinedMigrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        let core = <Migrator as MigratorTrait>::migrations();
        let core_names: std::collections::HashSet<String> =
            core.iter().map(|m| m.name().to_owned()).collect();
        let mut all = core;
        for m in (self.plugin_provider)() {
            if !core_names.contains(m.name()) {
                all.push(m);
            }
        }
        all
    }
}

/// Return the backing file path of the `main` SQLite database, or `None`
/// for in-memory/temporary databases.
///
/// Detection deliberately avoids `SqliteConnectOptions::get_filename()`:
/// sqlx rewrites `sqlite::memory:` URLs to an internal
/// `file:sqlx-in-memory-{n}` filename, which would misroute every in-memory
/// caller onto the file-backed path. `database_list` reports an empty
/// `file` column for in-memory databases and an absolute path for
/// file-backed ones (documented SQLite behavior, independent of sqlx
/// internals).
#[cfg(feature = "db-sqlite")]
async fn sqlite_main_db_file(db: &DatabaseConnection) -> Result<Option<String>, sea_orm::DbErr> {
    // SQLite exposes the read-only `database_list` pragma as a table-valued
    // function, so it selects like any other relation.
    let stmt = Query::select()
        .column(Alias::new("name"))
        .column(Alias::new("file"))
        .from_function(
            Func::cust(Alias::new("pragma_database_list")),
            Alias::new("database_list"),
        )
        .to_owned();
    let rows = db.query_all(&stmt).await?;
    for row in rows {
        let name = row.try_get::<String>("", "name")?;
        if name == "main" {
            let file = row.try_get::<String>("", "file")?;
            return Ok((!file.is_empty()).then_some(file));
        }
    }
    Ok(None)
}

/// Post-migration integrity gate: fail loud instead of silently committing
/// FK-inconsistent data.
///
/// Note: a cascade wipe is UNDETECTABLE here — cascades leave the database
/// FK-consistent. This gate catches orphan-class mistakes; cascade safety
/// on the file-backed path comes from the migration connection being born
/// with FK OFF.
#[cfg(feature = "db-sqlite")]
async fn sqlite_foreign_key_check(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    // SQLite exposes the read-only `foreign_key_check` pragma as a
    // table-valued function; an empty result set means "no violations".
    // Its first column is literally named `table`, a reserved word, so the
    // `Alias` quoting is load-bearing.
    let stmt = Query::select()
        .column(Alias::new("table"))
        .from_function(
            Func::cust(Alias::new("pragma_foreign_key_check")),
            Alias::new("foreign_key_check"),
        )
        .to_owned();
    let rows = db.query_all(&stmt).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut tables = rows
        .iter()
        .map(|row| row.try_get::<String>("", "table"))
        .collect::<Result<Vec<_>, _>>()?;
    tables.sort();
    tables.dedup();
    Err(sea_orm::DbErr::Custom(format!(
        "post-migration PRAGMA foreign_key_check found foreign key violations in: {}",
        tables.join(", ")
    )))
}

/// Run migrations on a dedicated single-connection pool born with
/// `foreign_keys=false`.
///
/// `PRAGMA foreign_keys` is a documented no-op inside a transaction, so FK
/// suspension must be a connection property established at connect time.
/// The dedicated pool preserves the two properties the single wrapping
/// transaction exists for (single-physical-connection DDL visibility,
/// all-or-nothing atomicity) while guaranteeing table-recreation migrations
/// never fire `ON DELETE CASCADE` via `DROP TABLE`. The caller's pool
/// (FK ON) is untouched.
#[cfg(feature = "db-sqlite")]
async fn run_on_dedicated_sqlite_pool(
    db: &DatabaseConnection,
    migrator: &CombinedMigrator,
) -> Result<(), sea_orm::DbErr> {
    use sea_orm::SqlxSqliteConnector;
    use sea_orm_migration::MigratorTraitSelf as _;
    use sqlx::sqlite::SqlitePoolOptions;

    // Clone-with-override: busy_timeout / journal_mode / synchronous are
    // plain struct fields on SqliteConnectOptions and carry over verbatim.
    let connect_opts = (*db.get_sqlite_connection_pool().connect_options())
        .clone()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_opts)
        .await
        .map_err(|e| {
            sea_orm::DbErr::Conn(sea_orm::RuntimeErr::SqlxError(std::sync::Arc::new(e)))
        })?;
    let migration_db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);

    // Run migration + post-commit gate, then ALWAYS close the dedicated pool
    // — even on a begin/up/commit error — so cleanup never depends on `Drop`.
    let result = async {
        let txn = begin_immediate(&migration_db).await?;
        migrator.up(&txn, None).await?;
        txn.commit().await?;
        sqlite_foreign_key_check(&migration_db).await
    }
    .await;
    if let Err(e) = migration_db.close().await {
        tracing::warn!(error = %e, "failed to close dedicated migration pool");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ConnectOptions, Database, DatabaseConnection, EntityTrait as _, PaginatorTrait as _,
    };

    use crate::entity::{
        audit_log, crl_cache, data_encryption_key, email_change_request, global_service_config,
        global_setting, host_discovery_allowlist, host_software_item, host_tag,
        host_tag_assignment, notification_channel, notification_log, notification_rule,
        plugin_config, plugin_type_setting, revoked_token_jti, revoked_token_user, service,
        software_ignore, software_item, system_audit_log, system_enrollment_token, system_service,
        system_service_certificate, tenant_discovery_allowlist, tenant_service_config,
        update_batch, update_history,
    };

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    async fn sqlite_table_sql(db: &DatabaseConnection, table: &str) -> String {
        let stmt = Query::select()
            .column(Alias::new("sql"))
            .from(Alias::new("sqlite_master"))
            .and_where(Expr::col(Alias::new("type")).eq("table"))
            .and_where(Expr::col(Alias::new("name")).eq(table))
            .to_owned();
        let row = db
            .query_one(&stmt)
            .await
            .expect("table lookup query should succeed")
            .expect("table should exist");
        row.try_get::<String>("", "sql")
            .expect("sqlite_master row should contain SQL text")
    }

    async fn sqlite_indexes(db: &DatabaseConnection, table: &str) -> Vec<String> {
        let stmt = Query::select()
            .column(Alias::new("name"))
            .from(Alias::new("sqlite_master"))
            .and_where(Expr::col(Alias::new("type")).eq("index"))
            .and_where(Expr::col(Alias::new("tbl_name")).eq(table))
            .to_owned();
        db.query_all(&stmt)
            .await
            .expect("index list query should succeed")
            .into_iter()
            .map(|row| {
                row.try_get::<String>("", "name")
                    .expect("index list row should contain index name")
            })
            .collect()
    }

    async fn legacy_audit_db_with_request_rows() -> DatabaseConnection {
        let db = test_db().await;
        let last_without_semantic = Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20260417_000001_semantic_audit_logs")
            .expect("semantic audit migration must exist")
            as u32;
        Migrator::up(&db, Some(last_without_semantic))
            .await
            .expect("legacy migrations should run");

        let tenant_id = uuid::Uuid::now_v7();
        let actor_id = uuid::Uuid::now_v7();
        let log_id = uuid::Uuid::now_v7();
        let occurred_at = time::OffsetDateTime::now_utc();

        db.execute(
            &Query::insert()
                .into_table(Alias::new("audit_logs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("actor_id"),
                    Alias::new("actor_type"),
                    Alias::new("auth_method"),
                    Alias::new("http_method"),
                    Alias::new("http_path"),
                    Alias::new("route_pattern"),
                    Alias::new("http_status"),
                    Alias::new("client_ip"),
                    Alias::new("user_agent"),
                    Alias::new("duration_ms"),
                    Alias::new("occurred_at"),
                ])
                .values_panic([
                    log_id.into(),
                    tenant_id.into(),
                    actor_id.into(),
                    "user".into(),
                    "password".into(),
                    "POST".into(),
                    "/api/v1/plugin-configs".into(),
                    "/api/v1/plugin-configs".into(),
                    201i32.into(),
                    "127.0.0.1".into(),
                    "test-agent".into(),
                    12i64.into(),
                    occurred_at.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("legacy audit row insert should succeed");

        db
    }

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
    async fn semantic_audit_migration_recreates_both_tables_and_drops_request_columns() {
        let db = test_db().await;
        Migrator::up(&db, None)
            .await
            .expect("migrations should run");

        let tenant_table_sql = sqlite_table_sql(&db, "audit_logs").await;
        let system_table_sql = sqlite_table_sql(&db, "system_audit_logs").await;
        let tenant_indexes = sqlite_indexes(&db, "audit_logs").await;
        let system_indexes = sqlite_indexes(&db, "system_audit_logs").await;

        // V2 columns present.
        assert!(tenant_table_sql.contains("action_type"));
        assert!(tenant_table_sql.contains("action_kind"));
        assert!(tenant_table_sql.contains("outcome"));
        assert!(tenant_table_sql.contains("before_snapshot"));
        assert!(tenant_table_sql.contains("after_snapshot"));
        assert!(tenant_table_sql.contains("correlation_id"));
        // V1 HTTP-request columns must be gone.
        assert!(!tenant_table_sql.contains("http_method"));
        assert!(system_table_sql.contains("action_type"));
        assert!(system_table_sql.contains("action_kind"));
        assert!(!system_table_sql.contains("http_path"));
        // V2 index names (V1 names were dropped when tables were recreated).
        assert!(tenant_indexes.contains(&"idx_audit_tenant_outcome_time".to_string()));
        assert!(system_indexes.contains(&"idx_system_audit_target_id_time".to_string()));
    }

    #[tokio::test]
    async fn semantic_audit_migration_drops_legacy_request_rows_instead_of_transforming_them() {
        let db = legacy_audit_db_with_request_rows().await;
        Migrator::up(&db, None)
            .await
            .expect("semantic migration should run");

        assert_eq!(
            audit_log::Entity::find()
                .count(&db)
                .await
                .expect("count should succeed"),
            0
        );
        assert_eq!(
            system_audit_log::Entity::find()
                .count(&db)
                .await
                .expect("count should succeed"),
            0
        );
    }

    /// Verify that the V2 CHECK constraint rejects an `event` row that also
    /// carries a `before_snapshot`.
    ///
    /// SQLite enforces CHECK constraints since version 3.25.0 (2018-09-15).
    /// All supported platforms ship a newer version, so no special PRAGMA is
    /// required — the constraint fires automatically on INSERT/UPDATE.
    #[tokio::test]
    async fn audit_v2_check_rejects_event_with_snapshots() {
        let db = test_db().await;
        Migrator::up(&db, None)
            .await
            .expect("migrations should run");

        let res = db
            .execute(
                &Query::insert()
                    .into_table(Alias::new("audit_logs"))
                    .columns([
                        Alias::new("id"),
                        Alias::new("tenant_id"),
                        Alias::new("occurred_at"),
                        Alias::new("actor_type"),
                        Alias::new("action_type"),
                        Alias::new("action_kind"),
                        Alias::new("outcome"),
                        Alias::new("before_snapshot"),
                    ])
                    .values_panic([
                        "00000000-0000-0000-0000-000000000001".into(),
                        "00000000-0000-0000-0000-000000000002".into(),
                        Expr::current_timestamp(),
                        "system".into(),
                        "auth.login".into(),
                        "event".into(),
                        "success".into(),
                        "{}".into(),
                    ])
                    .to_owned(),
            )
            .await;
        assert!(
            res.is_err(),
            "CHECK constraint must reject an event row that has before_snapshot set"
        );
    }

    /// Verify that the V2 CHECK constraint accepts a well-formed `event` row
    /// (both snapshot columns NULL) and a well-formed `stateful` row (both
    /// snapshot columns NOT NULL).
    #[tokio::test]
    async fn audit_v2_check_accepts_valid_event_and_stateful_rows() {
        let db = test_db().await;
        Migrator::up(&db, None)
            .await
            .expect("migrations should run");

        // Valid event row: no snapshots.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("audit_logs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("occurred_at"),
                    Alias::new("actor_type"),
                    Alias::new("action_type"),
                    Alias::new("action_kind"),
                    Alias::new("outcome"),
                ])
                .values_panic([
                    "00000000-0000-0000-0000-000000000010".into(),
                    "00000000-0000-0000-0000-000000000002".into(),
                    Expr::current_timestamp(),
                    "system".into(),
                    "auth.login".into(),
                    "event".into(),
                    "success".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("valid event row must be accepted");

        // Valid stateful row: both snapshots present.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("audit_logs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("occurred_at"),
                    Alias::new("actor_type"),
                    Alias::new("action_type"),
                    Alias::new("action_kind"),
                    Alias::new("outcome"),
                    Alias::new("before_snapshot"),
                    Alias::new("after_snapshot"),
                ])
                .values_panic([
                    "00000000-0000-0000-0000-000000000011".into(),
                    "00000000-0000-0000-0000-000000000002".into(),
                    Expr::current_timestamp(),
                    "system".into(),
                    "host.update".into(),
                    "stateful".into(),
                    "success".into(),
                    "{}".into(),
                    "{}".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("valid stateful row must be accepted");
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

        // Verify host_tags and host_tag_assignments tables exist.
        host_tag::Entity::find().count(&db).await.unwrap();
        host_tag_assignment::Entity::find()
            .count(&db)
            .await
            .unwrap();

        // Verify plugin_type_settings table exists.
        plugin_type_setting::Entity::find()
            .count(&db)
            .await
            .unwrap();

        // Verify tenant_service_config and global_service_config tables exist.
        tenant_service_config::Entity::find()
            .count(&db)
            .await
            .unwrap();
        global_service_config::Entity::find()
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

        // Verify 8 new built-in roles exist.
        for role_name in [
            "viewer",
            "operator",
            "service_manager",
            "software_manager",
            "host_manager",
            "settings_manager",
            "command_manager",
            "system_administrator",
        ] {
            let role_stmt = sea_orm_migration::prelude::Query::select()
                .expr(sea_orm_migration::prelude::Func::count(
                    sea_orm_migration::prelude::Expr::col(Alias::new("id")),
                ))
                .from(Alias::new("roles"))
                .and_where(sea_orm_migration::prelude::Expr::col(Alias::new("name")).eq(role_name))
                .to_owned();
            let role_rows = db.query_all(&role_stmt).await.unwrap();
            let role_count: i64 = {
                use sea_orm::TryGetable;
                role_rows
                    .first()
                    .map(|r| i64::try_get_by_index(r, 0).unwrap_or(0))
                    .unwrap_or(0)
            };
            assert_eq!(
                role_count, 1,
                "{role_name} role must exist after all migrations"
            );
        }

        // Verify email_change_requests table exists.
        email_change_request::Entity::find()
            .count(&db)
            .await
            .unwrap();
    }

    /// Verify that the repair migration converts TEXT-stored UUIDs to BLOBs.
    ///
    /// Steps:
    /// 1. Apply migrations 1–18 (all except the repair migration).
    /// 2. Manually inject a permission row with a TEXT-stored UUID and a
    ///    matching `role_permissions` row.
    /// 3. Apply exactly migration 19 (the repair; bounded — the M1.8 drop never runs here).
    /// 4. Assert `typeof(id) = 'blob'` for the injected permission.
    #[tokio::test]
    async fn repair_migration_fixes_text_uuid_storage() {
        use sea_orm::{ConnectionTrait as _, TryGetable as _};

        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();

        // Apply migrations 1–18 (skip the repair migration at index 19).
        Migrator::up(&db, Some(18))
            .await
            .expect("first 18 migrations should succeed");

        // Resolve the owner role with a column-list select — the entity API
        // expects the `is_built_in` column that does not exist yet at
        // migration step 18.
        let owner_role_id: uuid::Uuid = {
            let stmt = Query::select()
                .from(Alias::new("roles"))
                .column(Alias::new("id"))
                .and_where(Expr::col(Alias::new("name")).eq("owner"))
                .to_owned();
            let row = db
                .query_one(&stmt)
                .await
                .unwrap()
                .expect("owner role must exist after 18 migrations");
            use sea_orm::TryGetable;
            uuid::Uuid::try_get_by_index(&row, 0).unwrap()
        };

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
                    owner_role_id.into(),          // Uuid → BLOB
                    broken_uuid.to_owned().into(), // String → TEXT
                ])
                .to_owned(),
        )
        .await
        .expect("injecting TEXT-uuid role_permission must succeed");

        // Confirm the injection was TEXT before the repair.
        let typeof_stmt = Query::select()
            .expr(Func::cust(Alias::new("typeof")).arg(Expr::col(Alias::new("id"))))
            .from(Alias::new("permissions"))
            .and_where(Expr::col(Alias::new("name")).eq("test_broken_perm"))
            .to_owned();
        let typeof_before = db
            .query_one(&typeof_stmt)
            .await
            .unwrap()
            .expect("injected row must exist");
        let type_str_before: String = String::try_get_by_index(&typeof_before, 0).unwrap();
        assert_eq!(type_str_before, "text", "pre-condition: id should be TEXT");

        // Apply exactly the repair migration (index 19) — bounded so the
        // M1.8 drop at the tip never runs in this test.
        Migrator::up(&db, Some(1))
            .await
            .expect("repair migration must succeed");

        // After the repair, typeof(id) must be 'blob'.
        let typeof_after = db
            .query_one(&typeof_stmt)
            .await
            .unwrap()
            .expect("repaired row must still exist");
        let type_str_after: String = String::try_get_by_index(&typeof_after, 0).unwrap();
        assert_eq!(
            type_str_after, "blob",
            "after repair: permission id must be BLOB"
        );
    }

    /// Verify that the datetime repair migration converts `+00:00:00`-formatted
    /// `created_at` values to RFC 3339, making the row decodable by SeaORM.
    ///
    /// Steps:
    /// 1. Apply migrations 1–19 (all except the datetime repair).
    /// 2. Inject a permission with `created_at = '2026-03-02 22:33:15.239039 +00:00:00'`.
    /// 3. Apply exactly migration 20 (the datetime repair; bounded — the M1.8 drop never runs here).
    /// 4. Assert the stored `created_at` string parses as RFC 3339.
    #[tokio::test]
    async fn repair_migration_fixes_created_at_format() {
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

        // Apply exactly the datetime repair migration (index 20) — bounded
        // so the M1.8 drop at the tip never runs in this test.
        Migrator::up(&db, Some(1))
            .await
            .expect("datetime repair migration must succeed");

        // The stored string must now be RFC 3339 — parseable the way
        // SeaORM's OffsetDateTime decode would parse it.
        use sea_orm::TryGetable;
        let row = db
            .query_one(
                &Query::select()
                    .column(Alias::new("created_at"))
                    .from(Alias::new("permissions"))
                    .and_where(Expr::col(Alias::new("name")).eq("test_broken_datetime"))
                    .to_owned(),
            )
            .await
            .expect("select repaired row")
            .expect("injected permission must still exist after repair");
        let stored: String = String::try_get_by_index(&row, 0).expect("created_at string");
        time::OffsetDateTime::parse(&stored, &time::format_description::well_known::Rfc3339)
            .expect("created_at must be RFC 3339 after the repair");
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

    struct SentinelPluginMigration;

    impl MigrationName for SentinelPluginMigration {
        fn name(&self) -> &str {
            "m20990101_000001_sentinel_plugin"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for SentinelPluginMigration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(Alias::new("plugin_sentinel"))
                        .col(
                            ColumnDef::new(Alias::new("id"))
                                .integer()
                                .not_null()
                                .primary_key(),
                        )
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(
                    Table::drop()
                        .table(Alias::new("plugin_sentinel"))
                        .to_owned(),
                )
                .await
        }
    }

    fn sentinel_provider() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(SentinelPluginMigration)]
    }

    async fn has_sqlite_table(db: &DatabaseConnection, name: &str) -> bool {
        let stmt = Query::select()
            .column(Alias::new("name"))
            .from(Alias::new("sqlite_master"))
            .and_where(Expr::col(Alias::new("type")).eq("table"))
            .and_where(Expr::col(Alias::new("name")).eq(name))
            .to_owned();
        db.query_one(&stmt)
            .await
            .expect("sqlite_master query should succeed")
            .is_some()
    }

    /// Regression for the thread-local plugin-migration loss: on a multi-thread
    /// runtime the old code could resume `CombinedMigrator::up` on a worker
    /// thread whose thread-local was empty, silently dropping plugin migrations.
    /// Iterations make the thread-hop loss probable on the old code; the
    /// instance-based migrator passes deterministically.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn plugin_migrations_survive_thread_hops() {
        for _ in 0..8 {
            let db = test_db().await;
            run_migrations_with_plugins(&db, sentinel_provider)
                .await
                .expect("combined migrations should run");
            assert!(
                has_sqlite_table(&db, "plugin_sentinel").await,
                "plugin migration must be applied regardless of worker thread"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_runs_each_get_plugin_migrations() {
        async fn run_one() -> bool {
            let db = test_db().await;
            run_migrations_with_plugins(&db, sentinel_provider)
                .await
                .expect("combined migrations should run");
            has_sqlite_table(&db, "plugin_sentinel").await
        }
        let h1 = tokio::spawn(run_one());
        let h2 = tokio::spawn(run_one());
        let (a, b) = (h1.await.expect("task a"), h2.await.expect("task b"));
        assert!(a, "first concurrent run must apply plugin migrations");
        assert!(b, "second concurrent run must apply plugin migrations");
    }

    /// Regression for the FK-suspension no-op: `m20260414_000001` drops and
    /// recreates `update_history`; with FK enforcement ON inside the migration
    /// transaction, the implicit DELETE of `DROP TABLE` cascade-wiped all
    /// `update_output_lines` rows. Runs on a FILE-backed DB (the production
    /// path); asserts through the ORIGINAL caller pool to also pin cross-pool
    /// schema visibility. Fails on the pre-fix runner, passes with the
    /// dedicated FK-OFF migration pool.
    #[tokio::test]
    async fn file_backed_recreation_does_not_cascade_wipe_child_rows() {
        use sea_orm::TryGetable;

        let dir = tempfile::tempdir().expect("tmp dir");
        let db_path = dir.path().join("cascade.db");
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let mut opt = ConnectOptions::new(url);
        // Single connection: the setup below calls `Migrator::up` with a
        // partial step count directly (bypassing the transaction-wrapped
        // runner) to stop short of the recreation migration. That raw,
        // partial-step call is the pre-existing multi-connection DDL
        // visibility bug this module's docs describe (`run_migrations`
        // exists to wrap around it) — a multi-connection pool here would
        // fail this fixture's setup before ever reaching the code path
        // under test. The dedicated migration pool the fix opens in
        // `run_on_dedicated_sqlite_pool` always uses `max_connections(1)`
        // regardless of this pool's size, so the production path under
        // test is unaffected by this connection count.
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.expect("file db");

        // Apply everything strictly before the update_history recreation.
        let before_recreation = Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20260414_000001_update_execution_ownership")
            .expect("recreation migration must exist") as u32;
        Migrator::up(&db, Some(before_recreation))
            .await
            .expect("migrations before the recreation should run");

        // The initial migration seeds a default tenant — reuse it (FK ON here).
        let tenant_row = db
            .query_one(
                &Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("tenants"))
                    .to_owned(),
            )
            .await
            .expect("tenant query should succeed")
            .expect("default tenant is seeded by the initial migration");
        let tenant_id = uuid::Uuid::try_get_by_index(&tenant_row, 0).expect("tenant id");

        let now = time::OffsetDateTime::now_utc();
        let host_id = uuid::Uuid::now_v7();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("hosts"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("machine_id"),
                    Alias::new("hostname"),
                    Alias::new("friendly_name"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    host_id.into(),
                    tenant_id.into(),
                    "cascade-test-machine".into(),
                    "cascade-test-host".into(),
                    "Cascade Test Host".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("host insert should succeed");

        let item_id = uuid::Uuid::now_v7();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("software_items"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("name"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    item_id.into(),
                    tenant_id.into(),
                    "cascade-test-item".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("software item insert should succeed");

        let history_id = uuid::Uuid::now_v7();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("update_history"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("host_id"),
                    Alias::new("software_item_id"),
                    Alias::new("status"),
                    Alias::new("created_at"),
                ])
                .values_panic([
                    history_id.into(),
                    tenant_id.into(),
                    host_id.into(),
                    item_id.into(),
                    "completed".into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("update history insert should succeed");

        for i in 0..2 {
            db.execute(
                &Query::insert()
                    .into_table(Alias::new("update_output_lines"))
                    .columns([
                        Alias::new("id"),
                        Alias::new("update_history_id"),
                        Alias::new("stream"),
                        Alias::new("output"),
                        Alias::new("created_at"),
                    ])
                    .values_panic([
                        uuid::Uuid::now_v7().into(),
                        history_id.into(),
                        "stdout".into(),
                        format!("line {i}").into(),
                        now.into(),
                    ])
                    .to_owned(),
            )
            .await
            .expect("output line insert should succeed");
        }

        // Guard against false-RED: prove the children exist BEFORE the
        // recreation, so a silently-failed fixture insert cannot masquerade
        // as the cascade bug.
        let rows_before = db
            .query_all(
                &Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("update_output_lines"))
                    .and_where(Expr::col(Alias::new("update_history_id")).eq(history_id))
                    .to_owned(),
            )
            .await
            .expect("pre-migration child row query should succeed");
        assert_eq!(rows_before.len(), 2, "fixture must insert both child rows");

        // Run the remaining migrations through the real runner.
        run_migrations(&db)
            .await
            .expect("remaining migrations should run");

        // Children must survive; query through the ORIGINAL caller pool.
        let rows = db
            .query_all(
                &Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("update_output_lines"))
                    .and_where(Expr::col(Alias::new("update_history_id")).eq(history_id))
                    .to_owned(),
            )
            .await
            .expect("child row query should succeed");
        assert_eq!(
            rows.len(),
            2,
            "update_output_lines children must survive the update_history recreation"
        );
    }

    /// The post-commit gate must FAIL LOUD, naming the offending table.
    #[tokio::test]
    async fn foreign_key_check_reports_violations() {
        // Single-connection pool so the FK-OFF pragma and the inserts share a
        // physical connection.
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1);
        let db = Database::connect(opt).await.expect("db");
        #[expect(
            clippy::disallowed_methods,
            reason = "builder limitation: PRAGMA foreign_keys toggle has no sea_query equivalent"
        )]
        db.execute_unprepared("PRAGMA foreign_keys = OFF")
            .await
            .expect("fk off");
        db.execute(
            &Table::create()
                .table(Alias::new("parent"))
                .col(ColumnDef::new(Alias::new("id")).integer().primary_key())
                .to_owned(),
        )
        .await
        .expect("parent table");
        db.execute(
            &Table::create()
                .table(Alias::new("child"))
                .col(ColumnDef::new(Alias::new("id")).integer().primary_key())
                .col(ColumnDef::new(Alias::new("parent_id")).integer().not_null())
                .foreign_key(
                    ForeignKey::create()
                        .from(Alias::new("child"), Alias::new("parent_id"))
                        .to(Alias::new("parent"), Alias::new("id")),
                )
                .to_owned(),
        )
        .await
        .expect("child table");
        db.execute(
            &Query::insert()
                .into_table(Alias::new("child"))
                .columns([Alias::new("id"), Alias::new("parent_id")])
                .values_panic([1.into(), 999.into()])
                .to_owned(),
        )
        .await
        .expect("orphan insert");

        let err = sqlite_foreign_key_check(&db)
            .await
            .expect_err("orphaned child row must be reported");
        assert!(
            err.to_string().contains("child"),
            "error must name the offending table, got: {err}"
        );
    }

    /// And the success path: a clean DB passes the gate.
    #[tokio::test]
    async fn foreign_key_check_passes_on_clean_db() {
        let db = test_db().await;
        run_migrations(&db).await.expect("migrations");
        sqlite_foreign_key_check(&db)
            .await
            .expect("freshly migrated DB must be FK-consistent");
    }

    /// `PRAGMA database_list` probe: empty `file` for in-memory, path for file.
    #[tokio::test]
    async fn sqlite_main_db_file_detects_memory_vs_file() {
        let mem = test_db().await;
        assert!(
            sqlite_main_db_file(&mem)
                .await
                .expect("probe should succeed")
                .is_none(),
            "in-memory DB must probe as None"
        );

        let dir = tempfile::tempdir().expect("tmp dir");
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("probe.db").display()
        );
        let file_db = Database::connect(ConnectOptions::new(url))
            .await
            .expect("file db");
        assert!(
            sqlite_main_db_file(&file_db)
                .await
                .expect("probe should succeed")
                .is_some(),
            "file-backed DB must probe as Some(path)"
        );
    }
}

#[cfg(test)]
mod repair_migration_tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection};

    use super::*;

    async fn open_test_db() -> DatabaseConnection {
        Database::connect("sqlite::memory:").await.expect("db")
    }

    #[tokio::test]
    async fn repair_migration_converts_monolithic_row_to_individual_rows() {
        let db = open_test_db().await;

        db.execute(
            &Table::create()
                .table(Alias::new("seaql_migrations"))
                .if_not_exists()
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .text()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("applied_at"))
                        .integer()
                        .not_null(),
                )
                .to_owned(),
        )
        .await
        .expect("create table");

        db.execute(
            &Query::insert()
                .into_table(Alias::new("seaql_migrations"))
                .columns([Alias::new("version"), Alias::new("applied_at")])
                .values_panic([
                    "m20260331_000001_ssh_agent_tables".into(),
                    1711929600.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("insert old row");

        let migration = m20260331_000002_agent_ssh_migration_history_repair::Migration;
        let schema_manager = sea_orm_migration::SchemaManager::new(&db);
        migration.up(&schema_manager).await.expect("repair up");

        let old_row = db
            .query_one(
                &Query::select()
                    .expr(Expr::val(1))
                    .from(Alias::new("seaql_migrations"))
                    .and_where(
                        Expr::col(Alias::new("version")).eq("m20260331_000001_ssh_agent_tables"),
                    )
                    .to_owned(),
            )
            .await
            .expect("query");
        assert!(old_row.is_none(), "old monolithic row must be deleted");

        let rows: Vec<sea_orm::QueryResult> = db
            .query_all(
                &Query::select()
                    .column(Alias::new("version"))
                    .from(Alias::new("seaql_migrations"))
                    .order_by(Alias::new("version"), sea_orm::sea_query::Order::Asc)
                    .to_owned(),
            )
            .await
            .expect("query all");
        assert_eq!(rows.len(), 13, "must have exactly 13 individual rows");
    }

    #[tokio::test]
    async fn repair_migration_is_noop_on_fresh_install() {
        let db = open_test_db().await;
        db.execute(
            &Table::create()
                .table(Alias::new("seaql_migrations"))
                .if_not_exists()
                .col(
                    ColumnDef::new(Alias::new("version"))
                        .text()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("applied_at"))
                        .integer()
                        .not_null(),
                )
                .to_owned(),
        )
        .await
        .expect("create table");

        let migration = m20260331_000002_agent_ssh_migration_history_repair::Migration;
        let schema_manager = sea_orm_migration::SchemaManager::new(&db);
        migration
            .up(&schema_manager)
            .await
            .expect("repair up no-op");

        let rows: Vec<sea_orm::QueryResult> = db
            .query_all(
                &Query::select()
                    .expr(Expr::val(1))
                    .from(Alias::new("seaql_migrations"))
                    .to_owned(),
            )
            .await
            .expect("query");
        assert!(
            rows.is_empty(),
            "no-op on fresh install must leave table empty"
        );
    }
}
