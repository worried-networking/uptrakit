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
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
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
#[tokio::test]
async fn ordinary_and_mixed_gate_denials_emit_no_event() {
    let app = TestApp::new().await;
    let client = app.client();
    let (_user_id, token) = stage_zero_role_user(&app).await;

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

    // Give any (incorrect) fire-and-forget emission a real chance to land
    // before asserting its absence — mirrors the poll budget used for the
    // positive-path assertions elsewhere in this module.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let tenant_row = audit_log::Entity::find()
        .filter(
            audit_log::Column::ActionType.eq(uptrakit_audit_log::AuditActionType::ACCESS_DENIED),
        )
        .order_by_desc(audit_log::Column::OccurredAt)
        .one(&app.db)
        .await
        .expect("query audit rows");
    assert!(
        tenant_row.is_none(),
        "ordinary/mixed-gate denials must not emit an access.denied Event, got: {tenant_row:?}"
    );
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
