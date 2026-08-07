//! Cross-cutting M1.6a management-API integration tests (Plan 2, Task 7).
//!
//! Tasks 3-6 landed extensive per-family test modules inline in
//! `routes/access_grants.rs`, `routes/roles.rs`, and `routes/users.rs`. This
//! module has two jobs those per-family modules cannot do on their own:
//!
//! 1. Prove the pieces **compose**: a single `access:manage` principal
//!    drives a full grant + role + assignment lifecycle end to end
//!    ([`end_to_end_grant_role_assignment_flow_under_access_manage`]).
//! 2. Close gaps in the negative-direction (E2) matrix: every operation in
//!    `access_grants.rs`, `roles.rs`, and `users.rs`, cross-checked against
//!    its declared `security(("oauth2" = [...]))` scope, diffed against the
//!    per-family tests those files already carry. Only the routes that diff
//!    found ACTUALLY MISSING get a new test here (duplicating an existing
//!    assertion is a defect per the Task 6 ledger, not extra safety):
//!
//!    - `roles.rs`: all five routes (list/get/create/update/delete) are
//!      already covered against a `users:manage`-only principal by
//!      `users_manage_only_principal_gets_403_on_role_routes_incl_reads`.
//!      No gap.
//!    - `access_grants.rs`: `users_manage_only_principal_gets_403_on_grant_routes`
//!      covers only `GET /api/v1/access/grants` (list) and
//!      `POST /api/v1/access/grants` (create). `GET/PUT/DELETE
//!      /api/v1/access/grants/{id}` were never asserted against a
//!      `users:manage`-only principal — closed by
//!      [`users_manage_only_principal_gets_403_on_grant_by_id_routes`].
//!    - `users.rs`: `split_users_manage_can_lifecycle_but_not_assign_and_vice_versa`
//!      covers `GET /api/v1/users` and `PUT /api/v1/users/{id}/active`
//!      against an `access:manage`-only principal. `GET /api/v1/permissions`
//!      and `GET /api/v1/users/{id}` were never asserted against an
//!      `access:manage`-only principal — closed by
//!      [`access_manage_only_principal_gets_403_on_user_read_routes`].
//!    - `users.rs`'s self-service routes (`PUT .../profile`,
//!      `POST/DELETE .../email`, `PUT .../password`) carry no
//!      `users:manage`/`access:manage` scope at all (`security(("bearer_token"
//!      = []))` / per-request `oauth2 = []`) — out of scope for this matrix.
//!    - The access-preset endpoints (`GET /api/v1/access-presets`,
//!      `POST /api/v1/users/{id}/apply-preset`) were retired in M1.6b —
//!      [`deleted_preset_routes_are_gone_not_stubbed`] proves they 404/405
//!      rather than silently reappearing.
//!
//! E11 needs no new test here: seed-read pairing (`settings_manager` ->
//! `settings:read`, `operator` -> `services:read`) is already pinned by
//! `crates/shared/db/src/migration/m20260728_000002_seed_access_grants.rs:392-409`
//! (verified against the file at authoring time).
//!
//! Staging trap (mistake-ledger, restated): `register_user`/
//! `register_and_get_token` trigger owner bootstrap on the FIRST
//! registration, so a principal staged that way holds every permission and
//! a guard test built on it would pass on inherited authority rather than
//! the thing under test. Every non-owner principal below is staged via
//! [`stage_user_with_grant`]/[`stage_zero_role_user`] (strip-then-grant,
//! re-login AFTER the strip) or [`role_id_by_name`]/[`link_role`] against a
//! role explicitly built in the test — never the raw post-registration
//! token.

use http::StatusCode;
use serde_json::json;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    open_registration, role_id_by_name, stage_user_with_grant, stage_zero_role_user,
};
use uptrakit_web_api_types::access_grants::AccessGrantResponse;
use uptrakit_web_api_types::roles::RoleResponse;

/// Mandatory end-to-end flow (task's done-when, written regardless of
/// per-file coverage): one `access:manage` principal creates a role, grants
/// it tenant-plane `users:manage` authority, assigns it to a target user,
/// observes the target gain authority on its NEXT request (same token, no
/// relogin -- proving the assignment endpoint's own cache invalidation
/// fired), unassigns the role, and deletes it -- authority must be gone
/// both immediately after unassignment and again after the delete.
///
/// Discriminating by construction: the target starts with zero roles/grants
/// of its own (`stage_zero_role_user`), so every `OK` in this test can only
/// come from the role built and wired up in-test, and every `FORBIDDEN` can
/// only come from that authority having been actually withdrawn -- deleting
/// any one step (grant creation, role assignment, unassignment, or role
/// deletion) flips a later assertion.
#[tokio::test]
async fn end_to_end_grant_role_assignment_flow_under_access_manage() {
    let app = TestApp::new().await;
    let client = app.client();
    let owner_token = open_registration(&app).await;

    let (target_id, target_token) = stage_zero_role_user(&app).await;

    // Baseline: the target holds no authority yet.
    let status = client
        .get("/api/v1/users")
        .bearer(&target_token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "zero-role baseline must deny before any wiring"
    );

    // 1. Create the role.
    let (status, role): (StatusCode, RoleResponse) = client
        .post_json(
            "/api/v1/roles",
            &json!({ "name": "e2e-flow-role", "description": null }),
        )
        .bearer(&owner_token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::CREATED, "role creation must succeed");

    // 2. Grant the role tenant-plane `users:manage` authority.
    let (status, _grant): (StatusCode, AccessGrantResponse) = client
        .post_json(
            "/api/v1/access/grants",
            &json!({
                "subject_type": "role",
                "subject_id": role.id,
                "patterns": ["users:manage"]
            }),
        )
        .bearer(&owner_token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::CREATED, "grant creation must succeed");

    // 3. Assign the role to the target user.
    let status = client
        .put_json(
            &format!("/api/v1/users/{target_id}/roles"),
            &json!({ "role_ids": [role.id] }),
        )
        .bearer(&owner_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::OK, "role assignment must succeed");

    // 4. Observe authority on the NEXT request: same token, no relogin.
    let status = client
        .get("/api/v1/users")
        .bearer(&target_token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "assigning the role must grant authority without relogin"
    );

    // 5. Unassign: replace with `viewer` (read-only, `*:read` only -- see
    // SEED_GRANTS in m20260728_000002_seed_access_grants.rs) rather than an
    // empty list, because `UpdateUserRolesRequest::validate` rejects an
    // empty `role_ids`. `viewer` carries no `users:manage`, so this remains
    // discriminating.
    let viewer_id = role_id_by_name(&app, "viewer").await;
    let status = client
        .put_json(
            &format!("/api/v1/users/{target_id}/roles"),
            &json!({ "role_ids": [viewer_id] }),
        )
        .bearer(&owner_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::OK, "unassignment must succeed");

    let status = client
        .get("/api/v1/users")
        .bearer(&target_token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "unassigning the role must revoke authority immediately"
    );

    // 6. Delete the role. No lockout risk: it is no longer assigned to
    // anyone and its only pattern (`users:manage`) is not access:manage or
    // system.access:manage.
    let status = client
        .delete(&format!("/api/v1/roles/{}", role.id))
        .bearer(&owner_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "role deletion must succeed");

    // 7. Authority remains gone post-delete.
    let status = client
        .get("/api/v1/users")
        .bearer(&target_token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "authority must remain revoked after role deletion"
    );
}

/// E2 gap-fill: `GET/PUT/DELETE /api/v1/access/grants/{id}` under a
/// `users:manage`-only principal. Tasks 3-6 asserted only `GET`
/// (list)/`POST` (create) for this principal shape
/// (`access_grants.rs::users_manage_only_principal_gets_403_on_grant_routes`);
/// deleting the `CanManageAccess` extractor on any of the three by-id
/// handlers would leave this test green with the old behavior, so each
/// assertion targets a real, pre-existing grant row (not a random UUID) to
/// keep the extractor gate -- not a 404 -- the thing under test.
#[tokio::test]
async fn users_manage_only_principal_gets_403_on_grant_by_id_routes() {
    let app = TestApp::new().await;
    let client = app.client();
    let owner_token = open_registration(&app).await;
    let (victim_id, _victim_token) = stage_zero_role_user(&app).await;

    let (status, grant): (StatusCode, AccessGrantResponse) = client
        .post_json(
            "/api/v1/access/grants",
            &json!({
                "subject_type": "user",
                "subject_id": victim_id,
                "patterns": ["hosts:read"]
            }),
        )
        .bearer(&owner_token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::CREATED, "fixture grant must be created");

    let (_id, token) = stage_user_with_grant(
        &app,
        "grants-by-id-users-mgr@test.local",
        &["users:manage"],
        Some(app.tenant_id),
    )
    .await;

    let status = client
        .get(&format!("/api/v1/access/grants/{}", grant.id))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "GET by id must require access:manage"
    );

    let status = client
        .put_json(
            &format!("/api/v1/access/grants/{}", grant.id),
            &json!({ "patterns": ["hosts:read"], "description": null }),
        )
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "PUT by id must require access:manage"
    );

    let status = client
        .delete(&format!("/api/v1/access/grants/{}", grant.id))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "DELETE by id must require access:manage"
    );
}

/// E2 gap-fill: `GET /api/v1/permissions` and `GET /api/v1/users/{id}`
/// under an `access:manage`-only principal. Tasks 3-6 asserted only
/// `GET /api/v1/users` (list)/`PUT /api/v1/users/{id}/active` for this
/// principal shape
/// (`users.rs::split_users_manage_can_lifecycle_but_not_assign_and_vice_versa`);
/// deleting `CanManageUsers` on either of these two read handlers would
/// leave this test green with the old behavior.
#[tokio::test]
async fn access_manage_only_principal_gets_403_on_user_read_routes() {
    let app = TestApp::new().await;
    let client = app.client();
    let _owner_token = open_registration(&app).await;
    let (target_id, _target_token) = stage_zero_role_user(&app).await;

    let (_id, token) = stage_user_with_grant(
        &app,
        "access-mgr-user-reads@test.local",
        &["access:manage"],
        Some(app.tenant_id),
    )
    .await;

    let status = client
        .get("/api/v1/permissions")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "listing permissions must require users:manage"
    );

    let status = client
        .get(&format!("/api/v1/users/{target_id}"))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "getting a user must require users:manage"
    );
}

/// E9: the retired access-preset endpoints must be gone, not stubbed —
/// `GET /api/v1/access-presets` and `POST /api/v1/users/{id}/apply-preset`
/// were deleted in M1.6b; this asserts the routes 404/405 rather than
/// silently reappearing (e.g. via a router merge or a partial revert).
#[tokio::test]
async fn deleted_preset_routes_are_gone_not_stubbed() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_zero_role_user(&app).await;
    let client = app.client();
    let status = client
        .get("/api/v1/access-presets")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let status = client
        .post_json(
            &format!("/api/v1/users/{}/apply-preset", uuid::Uuid::now_v7()),
            &json!({"preset": "owner"}),
        )
        .bearer(&token)
        .send_status()
        .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
        "expected 404/405, got {status}"
    );
}
