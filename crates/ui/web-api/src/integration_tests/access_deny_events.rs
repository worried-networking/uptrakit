//! Router-level tests for the M1.6b deny-Event funnel
//! (`middleware::action::record_access_denied`, spec §4): a qualifying
//! denial (`system.*` resource, or `commands:manage`/`access:manage`/
//! `mcp:use`) must emit an `access.denied` audit Event, scoped by the
//! denied action's plane; an ordinary or mixed-gate denial must not.
//!
//! Every principal below is staged via [`stage_zero_role_user`] — never
//! `register_user`/`register_and_get_token` for a deny leg, since the first
//! registration triggers owner bootstrap and grants everything, which would
//! make a deny assertion pass vacuously (mistake-ledger, restated per this
//! module's own brief).

#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]

use http::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use uptrakit_shared_db::entity::{audit_log, system_audit_log};

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::stage_zero_role_user;

/// Polls the tenant-scoped `audit_logs` table up to 50 × 10 ms for the
/// first `access.denied` row, since `emit_event` is async/fire-and-forget.
/// Mirrors `tenant_audit_row_for_action` in `routes/surfaces.rs`'s test
/// module (the canonical denied-audit idiom).
async fn poll_tenant_access_denied_row(
    db: &sea_orm::DatabaseConnection,
) -> Option<audit_log::Model> {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(
                audit_log::Column::ActionType
                    .eq(uptrakit_audit_log::AuditActionType::ACCESS_DENIED),
            )
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query audit rows")
        {
            return Some(row);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    None
}

/// Polls the tenant-scoped `audit_logs` table up to 50 × 10 ms until at
/// least `expected` `access.denied` rows exist, then returns the observed
/// count (which may exceed `expected` if extra rows landed) — callers
/// assert the exact value themselves, so an over-count is still caught.
async fn poll_tenant_access_denied_count(db: &sea_orm::DatabaseConnection, expected: u64) -> u64 {
    let mut count = 0;
    for _ in 0..50 {
        count = audit_log::Entity::find()
            .filter(
                audit_log::Column::ActionType
                    .eq(uptrakit_audit_log::AuditActionType::ACCESS_DENIED),
            )
            .count(db)
            .await
            .expect("count audit rows");
        if count >= expected {
            return count;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    count
}

/// Polls the global `system_audit_logs` table up to 50 × 10 ms for the
/// first `access.denied` row.
async fn poll_system_access_denied_row(
    db: &sea_orm::DatabaseConnection,
) -> Option<system_audit_log::Model> {
    for _ in 0..50 {
        if let Some(row) = system_audit_log::Entity::find()
            .filter(
                system_audit_log::Column::ActionType
                    .eq(uptrakit_audit_log::AuditActionType::ACCESS_DENIED),
            )
            .order_by_desc(system_audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query system audit rows")
        {
            return Some(row);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    None
}

/// A qualifying tenant-plane denial (`access:manage`, in the explicit
/// policy set) must emit a tenant-scoped `access.denied` Event.
///
/// `GET /api/v1/roles` gates on `CanManageAccess` (single-action
/// `action_extractor!` arm) — a zero-role principal is denied `NoGrant` on
/// the only action in the gate, so the gate is trivially "all qualifying".
#[tokio::test]
async fn qualifying_deny_emits_access_denied_event() {
    let app = TestApp::new().await;
    let client = app.client();
    let (user_id, token) = stage_zero_role_user(&app).await;

    let status = client
        .get("/api/v1/roles")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "zero-role user must be denied"
    );

    let row = poll_tenant_access_denied_row(&app.db)
        .await
        .expect("qualifying deny (access:manage) must emit an access.denied Event");
    assert_eq!(row.tenant_id, app.tenant_id);
    assert_eq!(row.actor_id, Some(user_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("action"));
    assert_eq!(row.target_id.as_deref(), Some("access:manage"));
    let details = row.details_json.expect("details_json present");
    assert_eq!(
        details.get("actions").and_then(|v| v.as_array()),
        Some(&vec![serde_json::Value::String(
            "access:manage".to_string()
        )])
    );
    assert_eq!(
        details.get("reason").and_then(|v| v.as_str()),
        Some("no_grant")
    );
}

/// Neither an ordinary denial nor a mixed OR-gate denial (some qualifying,
/// some not) may emit an `access.denied` Event.
///
/// - `GET /api/v1/hosts` gates on `hosts:read` alone (`CanReadHosts`) — not
///   in the qualifying set: ordinary denial.
/// - `GET /api/v1/plugin-types` gates on `authorize_any([software:read,
///   settings:read, system.settings:manage])` — `system.settings:manage`
///   qualifies but the other two don't, so the gate is mixed and must not
///   emit either (spec §4 OR-gate rule: ALL alternatives must qualify).
///
/// Absence claims are only sound between two positive controls: a fixed
/// `sleep` followed by a single absence query would still pass if a buggy
/// predicate emitted an Event slowly (past the sleep budget under load).
/// Instead this test sandwiches the two negative probes between a
/// qualifying deny (`GET /api/v1/roles`, `access:manage`) fired before and
/// after them, and polls for the exact row count each time. The audit
/// dispatcher is a single FIFO consumer processing this client's requests
/// in the order they were made, so by the time the *second* qualifying
/// deny's row is observed, any row a buggy (b)/(c) probe had incorrectly
/// enqueued is guaranteed to already be visible too — an exact count of 2
/// (not 3 or 4) at that point is what proves the negative probes emitted
/// nothing, structurally rather than incidentally.
#[tokio::test]
async fn ordinary_and_mixed_gate_denials_emit_no_event() {
    let app = TestApp::new().await;
    let client = app.client();
    let (_user_id, token) = stage_zero_role_user(&app).await;

    // (a) First positive control: a qualifying deny must land exactly one
    // access.denied row before we probe anything else.
    let status = client
        .get("/api/v1/roles")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "zero-role user must be denied access:manage (first probe)"
    );
    let count = poll_tenant_access_denied_count(&app.db, 1).await;
    assert_eq!(
        count, 1,
        "first qualifying deny must emit exactly one access.denied Event"
    );

    // (b) Ordinary single-action deny — must add nothing.
    let status = client
        .get("/api/v1/hosts")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "zero-role user must be denied hosts:read"
    );

    // (c) Mixed OR-gate deny — must add nothing either (ALL alternatives
    // must qualify for the gate itself to qualify).
    let status = client
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "zero-role user must be denied the mixed plugin-types gate"
    );

    // (d) Second positive control, closing the sandwich: a second
    // qualifying deny must bring the total to exactly 2 — not 3 or 4 — or
    // (b)/(c) leaked a row.
    let status = client
        .get("/api/v1/roles")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "zero-role user must be denied access:manage (second probe)"
    );
    let count = poll_tenant_access_denied_count(&app.db, 2).await;
    assert_eq!(
        count, 2,
        "the ordinary and mixed-gate probes must not have added any access.denied row \
         between the two qualifying-deny controls"
    );

    let rows = audit_log::Entity::find()
        .filter(
            audit_log::Column::ActionType.eq(uptrakit_audit_log::AuditActionType::ACCESS_DENIED),
        )
        .order_by_asc(audit_log::Column::OccurredAt)
        .all(&app.db)
        .await
        .expect("query audit rows");
    assert_eq!(
        rows.len(),
        2,
        "expected exactly two access.denied rows total"
    );
    for row in &rows {
        let details = row.details_json.as_ref().expect("details_json present");
        let actions: Vec<&str> = details
            .get("actions")
            .and_then(|v| v.as_array())
            .expect("actions array present")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(
            actions,
            vec!["access:manage"],
            "no access.denied row may mention hosts:read or any plugin-types mixed-gate \
             alternative — got {actions:?}"
        );
    }
}

/// A qualifying system-plane denial (`system.audit:read`, resource
/// `system.audit` is `system.*`) must emit a SYSTEM-scoped `access.denied`
/// Event — landing in `system_audit_logs`, not the tenant-scoped
/// `audit_logs` table.
///
/// `GET /api/v1/system-audit-logs` gates on `CanReadSystemAudit`
/// (single-action `action_extractor!` arm).
#[tokio::test]
async fn system_plane_deny_emits_system_scoped_event() {
    let app = TestApp::new().await;
    let client = app.client();
    let (user_id, token) = stage_zero_role_user(&app).await;

    let status = client
        .get("/api/v1/system-audit-logs")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "zero-role user must be denied"
    );

    let row = poll_system_access_denied_row(&app.db)
        .await
        .expect("qualifying system-plane deny (system.audit:read) must emit a system-scoped access.denied Event");
    assert_eq!(row.actor_id, Some(user_id));
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("action"));
    assert_eq!(row.target_id.as_deref(), Some("system.audit:read"));

    // The tenant-scoped table must NOT also carry this row (it's
    // system-scoped, not duplicated across both tables).
    let tenant_row = poll_tenant_access_denied_row(&app.db).await;
    assert!(
        tenant_row.is_none(),
        "system-scoped deny must not also land in the tenant-scoped audit_logs table"
    );
}
