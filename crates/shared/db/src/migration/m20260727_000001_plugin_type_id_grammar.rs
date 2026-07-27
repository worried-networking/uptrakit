//! Best-effort remap of persisted plugin type IDs to the dot-separated
//! kebab-case grammar (ADR-0031 amendment). Unmatched values are left
//! as-is — they already match nothing in the catalog. One setwise UPDATE
//! per (table, value-pair); no per-row processing.

use sea_orm_migration::prelude::*;

const PLUGIN_TYPE_RENAMES: &[(&str, &str)] = &[
    ("package_manager_apk", "package-manager.apk"),
    ("package_manager_apt", "package-manager.apt"),
    ("package_manager_cargo", "package-manager.cargo"),
    ("package_manager_dnf", "package-manager.dnf"),
    ("package_manager_homebrew", "package-manager.homebrew"),
    ("package_manager_mas", "package-manager.mas"),
    ("package_manager_npm", "package-manager.npm"),
    ("package_manager_pacman", "package-manager.pacman"),
    ("package_manager_pkg", "package-manager.pkg"),
    ("package_manager_routeros", "package-manager.routeros"),
    ("package_manager_skills", "package-manager.skills"),
    ("package_manager_snap", "package-manager.snap"),
    ("releases_docker", "releases.docker"),
    ("releases_forgejo", "releases.forgejo"),
    ("releases_github", "releases.github"),
    ("releases_gitlab", "releases.gitlab"),
    ("hook_shell", "hook.shell"),
    ("hook_systemd", "hook.systemd"),
    ("infrastructure_proxmox", "infrastructure.proxmox"),
    ("generic_shell", "generic.shell"),
    (
        "discovery_proxmox_helper_scripts",
        "discovery.proxmox-helper-scripts",
    ),
    (
        "discovery_uptrakit_self_update",
        "discovery.uptrakit-self-update",
    ),
    ("enhancement_dashboard_icons", "enhancement.dashboard-icons"),
    ("email", "notifications.email"),
    ("telegram", "notifications.telegram"),
    ("webhook", "notifications.webhook"),
];

/// (table, column) pairs holding plugin type ID values. Verified against
/// the entity files; `instance_plugin_setting` is singular and its column
/// is `plugin_type_id`; `notification_rules.plugin_type` is nullable.
const TABLE_COLUMNS: &[(&str, &str)] = &[
    ("plugin_configs", "plugin_type"),
    ("plugin_type_settings", "plugin_type"),
    ("instance_plugin_setting", "plugin_type_id"),
    ("host_software_item_plugins", "plugin_type"),
    ("tenant_discovery_allowlist", "plugin_type"),
    ("host_discovery_allowlist", "plugin_type"),
    ("notification_rules", "plugin_type"),
];

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        remap(manager, false).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        remap(manager, true).await
    }
}

async fn remap(manager: &SchemaManager<'_>, reverse: bool) -> Result<(), DbErr> {
    for (table, column) in TABLE_COLUMNS {
        for (old, new) in PLUGIN_TYPE_RENAMES {
            let (from, to) = if reverse { (*new, *old) } else { (*old, *new) };
            let stmt = Query::update()
                .table(Alias::new(*table))
                .value(Alias::new(*column), to)
                .and_where(Expr::col(Alias::new(*column)).eq(from))
                .to_owned();
            manager.exec_stmt(stmt).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;

    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    fn migration_index() -> u32 {
        Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20260727_000001_plugin_type_id_grammar")
            .expect("plugin_type_id_grammar migration must be registered") as u32
    }

    /// Seed one legacy-value row in each of the 7 `TABLE_COLUMNS` tables plus
    /// one `plugin_configs` row with an unrecognized value, run the new
    /// migration, and assert every legacy value was remapped while the
    /// unrecognized value was left untouched.
    #[tokio::test]
    async fn plugin_type_id_grammar_remaps_legacy_values() {
        use sea_orm::TryGetable;

        let db = test_db().await;
        let index = migration_index();

        // Migrate up to (not including) the new migration, so the seeded
        // columns are verified to actually exist first.
        Migrator::up(&db, Some(index))
            .await
            .expect("migrations before plugin_type_id_grammar must apply");

        // FK enforcement is ON for in-memory SQLite (`sqlite::memory:` defaults
        // to `foreign_keys=true` in sqlx), so every seed row below must satisfy
        // its real foreign keys. Reuse the default tenant seeded by the initial
        // migration, matching the pattern already used in `mod.rs`'s
        // `file_backed_recreation_does_not_cascade_wipe_child_rows` test.
        //
        // Every seed row below is built via the typed `Query::insert()` builder
        // (never raw SQL string interpolation): a UUID column has BLOB storage
        // affinity when populated through sea_query's typed bind, but a
        // string-interpolated `'{uuid}'` literal in raw SQL stores as TEXT — the
        // FK check then compares BLOB against TEXT and never matches, even
        // though the values are logically identical. This is the same TEXT/BLOB
        // UUID mismatch class covered by `mod.rs`'s
        // `repair_migration_fixes_text_uuid_storage` test.
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

        // plugin_configs: legacy value.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("plugin_configs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("plugin_type"),
                    Alias::new("name"),
                    Alias::new("config"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    uuid::Uuid::now_v7().into(),
                    tenant_id.into(),
                    "releases_docker".into(),
                    "cfg-1".into(),
                    "{}".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("plugin_configs legacy seed row");

        // plugin_configs: unknown value, must remain untouched.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("plugin_configs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("plugin_type"),
                    Alias::new("name"),
                    Alias::new("config"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    uuid::Uuid::now_v7().into(),
                    tenant_id.into(),
                    "custom_thing".into(),
                    "cfg-2".into(),
                    "{}".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("plugin_configs custom_thing seed row");

        // plugin_type_settings: legacy value.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("plugin_type_settings"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("plugin_type"),
                    Alias::new("config"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    uuid::Uuid::now_v7().into(),
                    tenant_id.into(),
                    "package_manager_apt".into(),
                    "{}".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("plugin_type_settings legacy seed row");

        // instance_plugin_setting: legacy value. Primary key is `plugin_type_id`
        // itself (no surrogate `id`, no foreign keys); the other columns all
        // have defaults.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("instance_plugin_setting"))
                .columns([Alias::new("plugin_type_id")])
                .values_panic(["hook_systemd".into()])
                .to_owned(),
        )
        .await
        .expect("instance_plugin_setting legacy seed row");

        // Parent row for host_discovery_allowlist's host_id foreign key.
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
                    "grammar-test-machine".into(),
                    "grammar-test-host".into(),
                    "Grammar Test Host".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("hosts seed row");

        // host_software_item_plugins: legacy value. Its only foreign key
        // (`plugin_config_id`) is left NULL, and `host_id`/`software_item_id`/
        // `host_software_item_id` carry no FK constraint in the current schema
        // (only an index), but they're still typed UUID columns, so bind them
        // as UUIDs rather than string-interpolating.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("host_software_item_plugins"))
                .columns([
                    Alias::new("id"),
                    Alias::new("host_id"),
                    Alias::new("software_item_id"),
                    Alias::new("host_software_item_id"),
                    Alias::new("role"),
                    Alias::new("plugin_type"),
                    Alias::new("package_identifier"),
                    Alias::new("execution_site"),
                    Alias::new("ordinal"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    uuid::Uuid::now_v7().into(),
                    host_id.into(),
                    uuid::Uuid::now_v7().into(),
                    uuid::Uuid::now_v7().into(),
                    "detect_version".into(),
                    "generic_shell".into(),
                    "pkg".into(),
                    "agent".into(),
                    0.into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("host_software_item_plugins legacy seed row");

        // tenant_discovery_allowlist: legacy value.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("tenant_discovery_allowlist"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("plugin_type"),
                    Alias::new("created_at"),
                ])
                .values_panic([
                    uuid::Uuid::now_v7().into(),
                    tenant_id.into(),
                    "infrastructure_proxmox".into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("tenant_discovery_allowlist legacy seed row");

        // host_discovery_allowlist: legacy value.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("host_discovery_allowlist"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("host_id"),
                    Alias::new("plugin_type"),
                    Alias::new("created_at"),
                ])
                .values_panic([
                    uuid::Uuid::now_v7().into(),
                    tenant_id.into(),
                    host_id.into(),
                    "discovery_proxmox_helper_scripts".into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("host_discovery_allowlist legacy seed row");

        // Parent row for notification_rules' channel_id foreign key.
        let channel_id = uuid::Uuid::now_v7();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("notification_channels"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("name"),
                    Alias::new("channel_type"),
                    Alias::new("config"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    channel_id.into(),
                    tenant_id.into(),
                    "grammar-test-channel".into(),
                    "webhook".into(),
                    "{}".into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("notification_channels seed row");

        // notification_rules: legacy value.
        db.execute(
            &Query::insert()
                .into_table(Alias::new("notification_rules"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("channel_id"),
                    Alias::new("event_type"),
                    Alias::new("plugin_type"),
                    Alias::new("enabled"),
                    Alias::new("created_at"),
                ])
                .values_panic([
                    uuid::Uuid::now_v7().into(),
                    tenant_id.into(),
                    channel_id.into(),
                    "update_available".into(),
                    "webhook".into(),
                    true.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("notification_rules legacy seed row");

        // Run the migration under test.
        Migrator::up(&db, Some(index + 1))
            .await
            .expect("plugin_type_id_grammar migration must apply");

        async fn value(db: &DatabaseConnection, table: &str, column: &str) -> String {
            let row = db
                .query_one(
                    &Query::select()
                        .column(Alias::new(column))
                        .from(Alias::new(table))
                        .to_owned(),
                )
                .await
                .expect("value query should succeed")
                .unwrap_or_else(|| panic!("{table} row should exist"));
            row.try_get::<String>("", column)
                .unwrap_or_else(|_| panic!("{table}.{column} should be a string"))
        }

        assert_eq!(
            value(&db, "plugin_type_settings", "plugin_type").await,
            "package-manager.apt"
        );
        assert_eq!(
            value(&db, "instance_plugin_setting", "plugin_type_id").await,
            "hook.systemd"
        );
        assert_eq!(
            value(&db, "host_software_item_plugins", "plugin_type").await,
            "generic.shell"
        );
        assert_eq!(
            value(&db, "tenant_discovery_allowlist", "plugin_type").await,
            "infrastructure.proxmox"
        );
        assert_eq!(
            value(&db, "host_discovery_allowlist", "plugin_type").await,
            "discovery.proxmox-helper-scripts"
        );
        assert_eq!(
            value(&db, "notification_rules", "plugin_type").await,
            "notifications.webhook"
        );

        let plugin_configs_rows = db
            .query_all(
                &Query::select()
                    .column(Alias::new("plugin_type"))
                    .from(Alias::new("plugin_configs"))
                    .order_by(Alias::new("name"), sea_orm::Order::Asc)
                    .to_owned(),
            )
            .await
            .expect("plugin_configs query should succeed");
        let plugin_configs_values: Vec<String> = plugin_configs_rows
            .iter()
            .map(|row| {
                row.try_get::<String>("", "plugin_type")
                    .expect("plugin_configs.plugin_type should be a string")
            })
            .collect();
        assert_eq!(
            plugin_configs_values,
            vec!["releases.docker", "custom_thing"],
            "legacy value must be remapped and unrecognized value must be left untouched"
        );
    }
}
