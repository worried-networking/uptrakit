#![expect(
    clippy::expect_used,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::access_grants::{
    AccessGrantError, GrantSubject, GuardedMutation, LockoutVerdict, NewGrant, begin_guarded,
    check_lockout, delete_grant, insert_grant,
};
use uptrakit_shared_db::entity::user;
use uptrakit_shared_types::access::{ActionPattern, Selector};
use uuid::Uuid;

use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

fn covering_patterns() -> Vec<ActionPattern> {
    vec!["access:manage".parse::<ActionPattern>().expect("pattern")]
}

/// Direct `user` entity insert — zero roles, zero grants. NEVER use
/// `fixtures::register_user` for lockout-guard tests: the first HTTP
/// registration on a fresh harness triggers owner bootstrap
/// (`handle_first_user_setup` -> `assign_owner_roles`), which assigns
/// `settings_manager` — whose seed grant already covers `access:manage`
/// tenant-wide. With that coverage in play, the two staged covering grants
/// below would not be the only coverage and the concurrency assertions
/// could never discriminate a real regression.
async fn insert_active_user(db: &DatabaseConnection) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    user::ActiveModel {
        id: Set(id),
        email: Set(format!("{id}@access-lockout.test")
            .parse()
            .expect("valid test email")),
        first_name: Set("Lockout".to_string()),
        last_name: Set("Holder".to_string()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert active user");
    id
}

/// One shrink: guard + delete inside a single Immediate transaction,
/// mirroring the Plan 2 handler shape.
async fn guarded_delete(
    db: &DatabaseConnection,
    sentinel_tenant_id: Uuid,
    grant_id: Uuid,
) -> Result<LockoutVerdict, String> {
    let txn = begin_guarded(db).await.map_err(|e| format!("{e:?}"))?;
    let verdict = check_lockout(
        &txn,
        sentinel_tenant_id,
        &GuardedMutation::DeleteGrant { grant_id },
    )
    .await
    .map_err(|e| format!("{e:?}"))?;
    match verdict {
        LockoutVerdict::Permitted => {
            delete_grant(&txn, grant_id)
                .await
                .map_err(|e| format!("{e:?}"))?;
            txn.commit().await.map_err(|e| e.to_string())?;
        }
        LockoutVerdict::TenantLockout | LockoutVerdict::SystemLockout => {
            drop(txn); // rollback
        }
    }
    Ok(verdict)
}

/// Only the `_postgres` leg discriminates the *serialization* mechanism. SeaORM
/// forces `max_connections(1)` on SQLite pools, so under the `_sqlite` leg the
/// second `guarded_delete` cannot begin until the first has committed — the two
/// tasks never genuinely overlap, and the leg passes identically whether
/// `begin_guarded` opens the transaction `Immediate` or `Deferred`. A green
/// `_sqlite` leg is therefore evidence that `check_lockout`'s covering-holder
/// arithmetic is right, NOT that the sentinel lock still serializes concurrent
/// shrinks. Treat the `_postgres` leg as the load-bearing one.
async fn test_concurrent_shrinks_serialize_one_gets_lockout(harness: &TestHarness) {
    // Two users, each holding a covering grant: deleting either ALONE is
    // Permitted; deleting both would strip the last holder.
    let user_a = insert_active_user(&harness.db).await;
    let user_b = insert_active_user(&harness.db).await;
    let patterns = covering_patterns();
    let grant_a = insert_grant(
        &harness.db,
        NewGrant {
            subject: GrantSubject::User(user_a),
            tenant_id: Some(harness.tenant_id),
            patterns: &patterns,
            selector: Selector::All,
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("grant a");
    let grant_b = insert_grant(
        &harness.db,
        NewGrant {
            subject: GrantSubject::User(user_b),
            tenant_id: Some(harness.tenant_id),
            patterns: &patterns,
            selector: Selector::All,
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("grant b");

    let (ra, rb) = tokio::join!(
        guarded_delete(&harness.db, harness.tenant_id, grant_a),
        guarded_delete(&harness.db, harness.tenant_id, grant_b),
    );
    let verdicts = [ra.expect("shrink a"), rb.expect("shrink b")];
    let permitted = verdicts
        .iter()
        .filter(|v| **v == LockoutVerdict::Permitted)
        .count();
    let locked = verdicts
        .iter()
        .filter(|v| **v == LockoutVerdict::TenantLockout)
        .count();
    assert_eq!(
        permitted, 1,
        "exactly one shrink may pass, got {verdicts:?}"
    );
    assert_eq!(
        locked, 1,
        "the other must be a tenant lockout, got {verdicts:?}"
    );
}

db_test!(
    concurrent_shrinks_serialize_one_gets_lockout,
    test_concurrent_shrinks_serialize_one_gets_lockout
);

async fn test_missing_sentinel_is_hard_error(harness: &TestHarness) {
    let txn = begin_guarded(&harness.db).await.expect("begin");
    let err = check_lockout(
        &txn,
        Uuid::now_v7(), // no such tenants row
        &GuardedMutation::DeactivateUser {
            user_id: Uuid::now_v7(),
        },
    )
    .await
    .expect_err("missing sentinel must never pass through");
    assert!(matches!(
        err.current_context(),
        AccessGrantError::SentinelMissing
    ));
}

db_test!(
    missing_sentinel_is_hard_error,
    test_missing_sentinel_is_hard_error
);
