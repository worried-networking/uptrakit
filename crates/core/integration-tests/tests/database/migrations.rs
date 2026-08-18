#![expect(
    clippy::expect_used,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

use sea_orm::EntityTrait;

async fn test_migrations_run_and_tables_exist(harness: &TestHarness) {
    // If we get here, migrations ran successfully during harness setup.
    // Verify key tables are queryable.
    let tenants = uptrakit_shared_db::entity::tenant::Entity::find()
        .all(&harness.db)
        .await
        .expect("query tenants");
    assert!(!tenants.is_empty(), "default tenant should exist");

    let roles = uptrakit_shared_db::entity::role::Entity::find()
        .all(&harness.db)
        .await
        .expect("query roles");
    assert!(
        !roles.is_empty(),
        "built-in roles should exist after migrations"
    );
}

db_test!(
    migrations_run_and_tables_exist,
    test_migrations_run_and_tables_exist
);

async fn test_all_core_entities_queryable(harness: &TestHarness) {
    // Verify that all major entity types can be queried without errors.
    // This catches schema mismatches between entity definitions and migrations.
    macro_rules! assert_queryable {
        ($entity:ty) => {
            <$entity>::find()
                .all(&harness.db)
                .await
                .unwrap_or_else(|e| panic!("failed to query {}: {e}", stringify!($entity)));
        };
    }

    use uptrakit_shared_db::entity::*;

    assert_queryable!(tenant::Entity);
    assert_queryable!(user::Entity);
    assert_queryable!(role::Entity);
    assert_queryable!(service::Entity);
    assert_queryable!(host::Entity);
    assert_queryable!(service_host::Entity);
    assert_queryable!(software_item::Entity);
    assert_queryable!(host_software_item::Entity);
    assert_queryable!(enrollment_token::Entity);
    assert_queryable!(system_enrollment_token::Entity);
    assert_queryable!(api_token::Entity);
    assert_queryable!(notification_channel::Entity);
    assert_queryable!(notification_rule::Entity);
    assert_queryable!(plugin_config::Entity);
    assert_queryable!(plugin_type_setting::Entity);
    assert_queryable!(instance_plugin_setting::Entity);
    assert_queryable!(setting::Entity);
    assert_queryable!(global_setting::Entity);
    assert_queryable!(update_history::Entity);
    assert_queryable!(update_batch::Entity);
    assert_queryable!(audit_log::Entity);
    assert_queryable!(system_audit_log::Entity);
    assert_queryable!(scheduled_task::Entity);
    assert_queryable!(host_tag::Entity);
    assert_queryable!(host_tag_assignment::Entity);
    assert_queryable!(ca_certificate::Entity);
    assert_queryable!(system_service::Entity);
    assert_queryable!(global_service_config::Entity);
    assert_queryable!(tenant_service_config::Entity);
}

db_test!(
    all_core_entities_queryable,
    test_all_core_entities_queryable
);

async fn test_access_grant_storage_migrated(harness: &TestHarness) {
    use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
    use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, Set};

    // Seed grants present: exactly eight role-subject rows from the M1.2 seed
    // migration (content equality is covered by the shared-db suite; the
    // engine-owned entity must not be used here — builders only). Scoped to
    // `description IS NULL`, the same discriminator the seed migration's own
    // test uses: later additive backfills mark their rows (the M1.5 `mcp:use`
    // backfill adds one marked row per access_mcp role), so an unscoped count
    // grows with every backfill and this assertion stops meaning "the M1.2
    // seed landed".
    let rows = harness
        .db
        .query_all(
            &Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("access_grants"))
                .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                .and_where(Expr::col(Alias::new("created_by")).is_null())
                .and_where(Expr::col(Alias::new("description")).is_null())
                .to_owned(),
        )
        .await
        .expect("query access_grants");
    assert_eq!(rows.len(), 8, "eight M1.2 seed grants must exist");

    // Per-scope role-name uniqueness on the live backend.
    let tenants = uptrakit_shared_db::entity::tenant::Entity::find()
        .all(&harness.db)
        .await
        .expect("query tenants");
    let tenant_id = tenants.first().expect("default tenant exists").id;
    let now = time::OffsetDateTime::now_utc();
    let dup = uptrakit_shared_db::entity::role::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        name: Set("viewer".to_string()),
        description: Set(None),
        is_built_in: Set(false),
        created_at: Set(now),
        tenant_id: Set(None),
    }
    .insert(&harness.db)
    .await;
    assert!(
        dup.is_err(),
        "duplicate global role name must violate uix_roles_global_name"
    );
    uptrakit_shared_db::entity::role::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        name: Set("viewer".to_string()),
        description: Set(None),
        is_built_in: Set(false),
        created_at: Set(now),
        tenant_id: Set(Some(tenant_id)),
    }
    .insert(&harness.db)
    .await
    .expect("tenant scope may reuse a global role name");
}

db_test!(
    access_grant_storage_migrated,
    test_access_grant_storage_migrated
);

/// The M1.8 drop migration's `down()` recreates the legacy permission
/// tables schema-only. PostgreSQL validates FK targets at creation time,
/// so a wrong parent/child creation order in `down()` fails only on the
/// Postgres leg — the shared-db in-memory tests cannot catch it.
async fn test_drop_permissions_down_recreates_schema(harness: &TestHarness) {
    use sea_orm::ConnectionTrait;
    use sea_orm::sea_query::{Alias, Query};
    use sea_orm_migration::MigratorTrait as _;
    use uptrakit_shared_db::migration::Migrator;

    let probe = |table: &str, col: &str| {
        Query::select()
            .column(Alias::new(col))
            .from(Alias::new(table))
            .to_owned()
    };
    const TABLES: [(&str, &str); 2] = [("permissions", "id"), ("role_permissions", "role_id")];

    // Tip state: both tables dropped by the M1.8 migration.
    for (table, col) in TABLES {
        assert!(
            harness.db.query_all(&probe(table, col)).await.is_err(),
            "{table} must not exist at tip"
        );
    }

    // down(): schema-only recreation — parent (`permissions`) before FK
    // child (`role_permissions`); PostgreSQL enforces the order here.
    //
    // Migrator::down(&db, Some(1)) would revert whichever migration is
    // LAST-APPLIED, not necessarily the M1.8 drop migration — compute the
    // step count from this migration's registered index to the tip so
    // appending a later migration cannot silently retarget this call.
    // `Migration` itself is `pub(super)` in the db crate and unreachable
    // from this crate, so unlike the in-crate down-test this cannot call
    // its down() directly and must go through `Migrator::down`.
    let total = Migrator::migrations().len();
    let idx = Migrator::migrations()
        .iter()
        .position(|m| m.name() == "m20260807_000001_drop_permissions_tables")
        .expect("drop migration must be registered");
    let steps =
        <u32 as std::convert::TryFrom<usize>>::try_from(total - idx).expect("steps fit u32");
    Migrator::down(&harness.db, Some(steps))
        .await
        .expect("drop migration down() must succeed");
    for (table, col) in TABLES {
        let rows = harness
            .db
            .query_all(&probe(table, col))
            .await
            .expect("recreated table must be queryable");
        assert!(rows.is_empty(), "{table} must be recreated empty");
    }

    // Re-applying the migration drops them again.
    Migrator::up(&harness.db, None)
        .await
        .expect("re-apply drop migration");
    for (table, col) in TABLES {
        assert!(
            harness.db.query_all(&probe(table, col)).await.is_err(),
            "{table} must be dropped again after re-up"
        );
    }
}

db_test!(
    drop_permissions_down_recreates_schema,
    test_drop_permissions_down_recreates_schema
);

/// Combined down-refuse coverage for all three `ENC:`-column migrations
/// (`plugin_configs`, `plugin_type_settings`, `instance_plugin_setting`).
///
/// This test intentionally lives in the LAST of the three per-entity
/// sub-passes: writing it after only one or two of the migrations existed
/// would silently exercise `Migrator::down(&db, Some(1))` against whatever
/// migration happened to be the tip at the time, not necessarily the
/// intended target — and a later migration appended after it would then
/// retarget the same call without the test noticing. All three migrations
/// must already be registered (tip-most, in landing order) for the
/// step-counted `Migrator::down` calls below to hit their intended targets.
///
/// Each `Migration` type is `pub(super)` in `uptrakit-shared-db` and
/// unreachable from this crate (unlike the in-crate per-migration tests),
/// so this test drives reverts through `Migrator::down`/`Migrator::up`
/// rather than calling `.down()` directly.
async fn test_encrypt_config_columns_down(harness: &TestHarness) {
    use sea_orm::ConnectionTrait;
    use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
    use sea_orm_migration::MigratorTrait as _;
    use uptrakit_shared_db::migration::Migrator;
    use uuid::Uuid;

    const MIGRATION_NAMES: [&str; 3] = [
        "m20260812_000001_encrypt_plugin_configs_config",
        "m20260812_000002_encrypt_plugin_type_settings_config",
        "m20260812_000003_encrypt_instance_plugin_setting_config",
    ];
    let all_migrations = Migrator::migrations();
    let total = all_migrations.len();
    let idx1 = all_migrations
        .iter()
        .position(|m| m.name() == MIGRATION_NAMES[0])
        .expect("plugin_configs encryption migration must be registered");
    let block: Vec<&str> = all_migrations
        .iter()
        .skip(idx1)
        .take(MIGRATION_NAMES.len())
        .map(|m| m.name())
        .collect();
    assert_eq!(
        block, MIGRATION_NAMES,
        "the three ENC: migrations must be consecutive and in this exact order in \
         Migrator::migrations() — this test drives them with step-counted Migrator::down/up \
         calls computed relative to idx1 (not the absolute tip), so migrations appended after \
         them (e.g. m20260811_000001_materialize_mcp_enabled, which lands last in the vec \
         despite its earlier filename date) are tolerated as long as this block stays intact"
    );
    // Number of migrations registered after the three-migration ENC block (0 originally;
    // trailing migrations, appended per the "new migrations always go last" rule, add to this).
    let after_block =
        u32::try_from(total - (idx1 + MIGRATION_NAMES.len())).expect("after_block fits u32");

    // Every timestamp column below is `timestamptz` on Postgres, which rejects a bare string
    // literal (unlike SQLite, which accepts it via type affinity) — always pass a typed
    // `OffsetDateTime`, as a real `ActiveModel` would.
    let now = time::OffsetDateTime::now_utc();

    async fn seed_plugin_config(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        name: &str,
        config: &str,
        now: time::OffsetDateTime,
    ) {
        db.execute(
            &Query::insert()
                .into_table(Alias::new("plugin_configs"))
                .columns([
                    Alias::new("id"),
                    Alias::new("tenant_id"),
                    Alias::new("name"),
                    Alias::new("plugin_type"),
                    Alias::new("config"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    Uuid::now_v7().into(),
                    tenant_id.into(),
                    name.into(),
                    "releases_docker".into(),
                    config.into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("seed plugin_configs row");
    }

    async fn seed_plugin_type_setting(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        plugin_type: &str,
        config: &str,
        now: time::OffsetDateTime,
    ) {
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
                    Uuid::now_v7().into(),
                    tenant_id.into(),
                    plugin_type.into(),
                    config.into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("seed plugin_type_settings row");
    }

    async fn seed_instance_plugin_setting(
        db: &sea_orm::DatabaseConnection,
        plugin_type_id: &str,
        config: &str,
        now: time::OffsetDateTime,
    ) {
        db.execute(
            &Query::insert()
                .into_table(Alias::new("instance_plugin_setting"))
                .columns([
                    Alias::new("plugin_type_id"),
                    Alias::new("enabled"),
                    Alias::new("config"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    plugin_type_id.into(),
                    true.into(),
                    config.into(),
                    now.into(),
                ])
                .to_owned(),
        )
        .await
        .expect("seed instance_plugin_setting row");
    }

    // Phase 1: at tip, seed one plaintext row per table, then revert all
    // three encryption migrations in one shot — proves a clean three-step
    // down over plaintext data on both backends.
    //
    // `plugin_configs.name` is unique per (tenant_id, name) among
    // non-deactivated rows, and `plugin_type_settings.plugin_type` is unique
    // per (tenant_id, plugin_type); the down/up round trip only changes
    // column TYPES, so these rows survive into phase 2 below — phase 2 must
    // use distinct key values or it collides with these unique indexes.
    seed_plugin_config(
        &harness.db,
        harness.tenant_id,
        "subpass3-combined-test-plain",
        r#"{"foo":"bar"}"#,
        now,
    )
    .await;
    seed_plugin_type_setting(
        &harness.db,
        harness.tenant_id,
        "subpass3-combined-test-plain",
        r#"{"foo":"bar"}"#,
        now,
    )
    .await;
    seed_instance_plugin_setting(&harness.db, "subpass3.plain", r#"{"foo":"bar"}"#, now).await;

    // Revert the ENC block plus any migrations registered after it (e.g. a later,
    // unrelated data-migration appended to the tip), landing right before idx1.
    Migrator::down(
        &harness.db,
        Some(u32::try_from(MIGRATION_NAMES.len()).expect("fits u32") + after_block),
    )
    .await
    .expect("down over plaintext rows must succeed on both backends");

    // Re-apply exactly the three ENC migrations so the tip is back at
    // m20260812_000003 for phase 2 — not `up(&db, None)`, which would also
    // re-apply any migrations registered after the block.
    Migrator::up(
        &harness.db,
        Some(u32::try_from(MIGRATION_NAMES.len()).expect("fits u32")),
    )
    .await
    .expect("re-apply the three encryption migrations");

    // Phase 2: seed an ENC:-prefixed row per table, using key values distinct
    // from phase 1's (still-present) rows, and confirm each migration's
    // single-step down refuses, one at a time, walking the tip backward:
    // instance_plugin_setting -> plugin_type_settings -> plugin_configs.
    seed_instance_plugin_setting(&harness.db, "subpass3.enc", "ENC:v3:deadbeef", now).await;
    seed_plugin_type_setting(
        &harness.db,
        harness.tenant_id,
        "subpass3-combined-test-enc",
        "ENC:v3:deadbeef",
        now,
    )
    .await;
    seed_plugin_config(
        &harness.db,
        harness.tenant_id,
        "subpass3-combined-test-enc",
        "ENC:v3:deadbeef",
        now,
    )
    .await;

    // instance_plugin_setting (tip: m20260812_000003).
    let err = Migrator::down(&harness.db, Some(1))
        .await
        .expect_err("down must refuse while instance_plugin_setting holds an ENC: row");
    let msg = err.to_string();
    assert!(
        msg.contains("instance_plugin_setting"),
        "error must name instance_plugin_setting; got: {msg}"
    );
    harness
        .db
        .execute(
            &Query::delete()
                .from_table(Alias::new("instance_plugin_setting"))
                .and_where(Expr::col(Alias::new("plugin_type_id")).eq("subpass3.enc"))
                .to_owned(),
        )
        .await
        .expect("delete the ENC: instance_plugin_setting row");
    Migrator::down(&harness.db, Some(1))
        .await
        .expect("instance_plugin_setting down must succeed once no ENC: rows remain");

    // plugin_type_settings (tip: m20260812_000002).
    let err = Migrator::down(&harness.db, Some(1))
        .await
        .expect_err("down must refuse while plugin_type_settings holds an ENC: row");
    let msg = err.to_string();
    assert!(
        msg.contains("plugin_type_settings"),
        "error must name plugin_type_settings; got: {msg}"
    );
    harness
        .db
        .execute(
            &Query::delete()
                .from_table(Alias::new("plugin_type_settings"))
                .and_where(Expr::col(Alias::new("config")).eq("ENC:v3:deadbeef"))
                .to_owned(),
        )
        .await
        .expect("delete the ENC: plugin_type_settings row");
    Migrator::down(&harness.db, Some(1))
        .await
        .expect("plugin_type_settings down must succeed once no ENC: rows remain");

    // plugin_configs (tip: m20260812_000001).
    let err = Migrator::down(&harness.db, Some(1))
        .await
        .expect_err("down must refuse while plugin_configs holds an ENC: row");
    let msg = err.to_string();
    assert!(
        msg.contains("plugin_configs"),
        "error must name plugin_configs; got: {msg}"
    );
    // No cleanup needed: the refused down() returns its error before
    // performing any schema change. By this point m20260812_000003 and
    // m20260812_000002 were already reverted (and left reverted) above;
    // only m20260812_000001 (plugin_configs) is still applied. The harness
    // is torn down (dropped SQLite file / discarded testcontainer) right
    // after this test, so no re-migration to full tip is needed here.
}

db_test!(
    encrypt_config_columns_down,
    test_encrypt_config_columns_down
);
