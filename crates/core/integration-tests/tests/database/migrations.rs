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
