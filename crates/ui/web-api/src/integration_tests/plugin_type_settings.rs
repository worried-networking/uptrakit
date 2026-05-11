//! Integration tests for the visibility predicate on
//! `GET /api/v1/plugin-type-settings/{plugin_type}`.
//!
//! # Why upsert/delete predicate-reject tests are absent
//!
//! The `PUT` and `DELETE` handlers are gated by `CanManageGlobalSettings`
//! middleware *before* the visibility predicate runs.  A tenant viewer
//! (ViewSettings only) receives 403 at the permission gate and never reaches
//! the predicate.  An admin (ManageGlobalSettings) always passes the predicate
//! because `is_plugin_visible_to_user` returns `true` when the user has that
//! permission.  There is therefore no reachable code path in the harness
//! test matrix where the predicate would return `false` for upsert/delete —
//! those tests would be vacuous duplicates of the existing permission-gate tests.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panics on failure are acceptable"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_user;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Register the admin (first user), re-open registration, then register a
/// second user who gets the built-in "user" role (ViewSettings but NOT
/// ManageGlobalSettings).  Returns `(admin_token, tenant_token)`.
async fn register_admin_and_tenant_user(app: &TestApp) -> (String, String) {
    let client = app.client();

    // First user → owner role (all permissions including ManageGlobalSettings).
    let (status, admin_auth) = register_user(&client, "owner@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "admin registration failed"
    );
    let admin_token = admin_auth.access_token.expose_secret().to_string();

    // Re-open registration so the second user can sign up.
    let reopen = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&admin_token)
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );

    // Second user → built-in "user" role: ViewSettings but NOT ManageGlobalSettings.
    let (status, tenant_auth) =
        register_user(&client, "tenant@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "tenant user registration failed"
    );
    let tenant_token = tenant_auth.access_token.expose_secret().to_string();

    (admin_token, tenant_token)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// A tenant user (ViewSettings, no ManageGlobalSettings) GETting settings for
/// a disabled Instance-scoped plugin must receive 404 — the predicate hides
/// the plugin, returning the same "not found" response regardless of whether
/// a settings row exists, to prevent existence leakage.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn tenant_user_get_plugin_type_settings_for_disabled_instance_plugin_returns_404() {
    let app = TestApp::new().await;
    let (_admin_token, tenant_token) = register_admin_and_tenant_user(&app).await;

    // The snapshot defaults to all_disabled() at TestApp boot, so
    // enhancement_dashboard_icons is disabled → predicate rejects for tenant users.
    let status = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement_dashboard_icons")
        .bearer(&tenant_token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::NOT_FOUND,
        "tenant user must receive 404 for disabled instance-scoped plugin settings"
    );
}

/// An admin (ManageGlobalSettings) GETting settings for a disabled
/// Instance-scoped plugin after seeding a settings row must receive 200 —
/// the predicate passes for admins regardless of the enabled/disabled state.
///
/// Without a seeded row the admin would also get 404 ("No settings found"),
/// which is indistinguishable from the tenant-user predicate-reject 404.
/// We seed a row here so the test proves the admin *can* reach the row.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn admin_get_plugin_type_settings_for_disabled_instance_plugin_returns_200_after_seed() {
    let app = TestApp::new().await;
    let (admin_token, _tenant_token) = register_admin_and_tenant_user(&app).await;

    // Seed a settings row directly through the query layer so the handler can return it.
    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        "enhancement_dashboard_icons",
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("seed plugin type settings row");

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement_dashboard_icons")
        .bearer(&admin_token)
        .send_json()
        .await;

    assert_eq!(
        status,
        http::StatusCode::OK,
        "admin must reach the settings row for a disabled instance-scoped plugin"
    );
    assert_eq!(
        body["plugin_type"], "enhancement_dashboard_icons",
        "response must identify the correct plugin type"
    );
}
