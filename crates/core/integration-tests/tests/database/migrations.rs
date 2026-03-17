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

    let permissions = uptrakit_shared_db::entity::permission::Entity::find()
        .all(&harness.db)
        .await
        .expect("query permissions");
    assert!(
        !permissions.is_empty(),
        "built-in permissions should exist after migrations"
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
    assert_queryable!(permission::Entity);
    assert_queryable!(role_permission::Entity);
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
