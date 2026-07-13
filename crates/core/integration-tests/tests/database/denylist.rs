//! Denylist DB-persistence tests across SQLite + Postgres.
//! The monotonic ON CONFLICT ... WHERE guard is dialect-sensitive, so it must
//! be verified on Postgres, not only SQLite.

#![expect(
    clippy::expect_used,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::revoked_token_user;
use uptrakit_web_api_auth::auth::token_denylist::TokenDenylist;
use uuid::Uuid;

use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_deny_user_monotonic_across_instances(harness: &TestHarness) {
    let user_id = Uuid::new_v4();

    let a = TokenDenylist::new_with_db(harness.db.clone());
    a.deny_user(user_id, 1_000, 1_900).await;

    // Fresh instance, stale (default-zero) in-memory entry, lower cutoff.
    let b = TokenDenylist::new_with_db(harness.db.clone());
    b.deny_user(user_id, 500, 1_400).await;

    let row = revoked_token_user::Entity::find_by_id(user_id)
        .one(&harness.db)
        .await
        .expect("query")
        .expect("row exists");
    assert_eq!(
        row.iat_cutoff, 1_000,
        "lower cutoff must not regress the row"
    );
    assert_eq!(row.purge_after, 1_900);

    // Cache-reconciliation regression (dialect-sensitive — this is the whole reason this
    // test must run on Postgres, not only SQLite: exec_without_returning()'s rows-affected
    // count on a suppressed `ON CONFLICT ... WHERE` upsert is the mechanism both engines
    // must agree on). B's own in-memory entry, provisionally set to 500 by its local gate,
    // must be reconciled upward to the DB's 1000 once the upsert no-ops.
    assert!(
        b.is_denied("unrelated-jti", &user_id, 750).await,
        "losing instance's cache must be reconciled upward to the winning DB cutoff"
    );
}

db_test!(
    deny_user_monotonic_across_instances,
    test_deny_user_monotonic_across_instances
);
