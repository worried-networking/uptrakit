#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

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

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uptrakit_shared_db::access_grants::{GrantSubject, delete_grant, load_grants_for_principal};
use uptrakit_shared_db::entity::{role, user_role};
use uptrakit_shared_types::access::actions;
use uuid::Uuid;

use crate::test_harness::TestApp;
#[cfg(feature = "dashboard-icons")]
use crate::test_harness::fixtures::register_admin_and_tenant_user;
use crate::test_harness::fixtures::{login_user, register_and_get_token, register_user};

// ── M1.5 fixtures ────────────────────────────────────────────────────────────

/// Register the first user (owner) and re-open registration so a second
/// user can sign up. Returns the owner's access token.
async fn open_registration(app: &TestApp) -> String {
    let client = app.client();
    let owner_token = register_and_get_token(&client).await;
    let reopen = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&owner_token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );
    owner_token
}

/// Register a fresh user (registration must already be open), strip its
/// auto-assigned `viewer` role, link ONLY `role_name`, invalidate the engine
/// cache, then re-login so the legacy JWT claim snapshot reflects the newly
/// linked role's legacy permission set. Returns `(user_id, access_token)`.
async fn register_user_with_only_role(
    app: &TestApp,
    email: &str,
    role_name: &str,
) -> (Uuid, String) {
    let client = app.client();
    let (status, auth) = register_user(&client, email, "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let user_id = auth.user.id;

    user_role::Entity::delete_many()
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&app.db)
        .await
        .expect("strip auto-assigned viewer role");

    let role_id = role::Entity::find()
        .filter(role::Column::Name.eq(role_name))
        .one(&app.db)
        .await
        .expect("query roles")
        .unwrap_or_else(|| panic!("seeded `{role_name}` role must exist"))
        .id;
    user_role::ActiveModel {
        tenant_id: Set(app.tenant_id),
        user_id: Set(user_id),
        role_id: Set(role_id),
        assigned_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&app.db)
    .await
    .expect("assign role");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);

    let (login_status, login_auth) = login_user(&client, email, "TestPassword123!").await;
    assert_eq!(login_status, http::StatusCode::OK, "re-login failed");
    (user_id, login_auth.access_token.expose_secret().to_string())
}

/// Owner + a fresh second user holding ONLY `role_name`. Returns
/// `(user_id, access_token)` for the second user.
async fn stage_user_with_only_role(app: &TestApp, role_name: &str) -> (Uuid, String) {
    open_registration(app).await;
    let email = format!("{role_name}-only@test.local");
    register_user_with_only_role(app, &email, role_name).await
}

/// Owner + a fresh second user with its auto-assigned `viewer` role
/// stripped and no replacement linked. Returns `(user_id, access_token)`.
async fn stage_zero_role_user(app: &TestApp) -> (Uuid, String) {
    let client = app.client();
    open_registration(app).await;
    let (status, auth) = register_user(&client, "zero-role@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let user_id = auth.user.id;
    user_role::Entity::delete_many()
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&app.db)
        .await
        .expect("strip auto-assigned roles");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);
    (user_id, auth.access_token.expose_secret().to_string())
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
    // enhancement.dashboard-icons is disabled → predicate rejects for tenant users.
    let status = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
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
        "enhancement.dashboard-icons",
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("seed plugin type settings row");

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
        .bearer(&admin_token)
        .send_json()
        .await;

    assert_eq!(
        status,
        http::StatusCode::OK,
        "admin must reach the settings row for a disabled instance-scoped plugin"
    );
    assert_eq!(
        body["plugin_type"], "enhancement.dashboard-icons",
        "response must identify the correct plugin type"
    );
}

/// `GET /api/v1/plugin-type-settings` (list) must include settings for disabled
/// Instance-scoped plugins when accessed by an admin. Tenant users get those
/// rows filtered out by the predicate (proved via the existing tests in
/// `plugin_configs.rs` and the disabled-plugin GET test above); this test
/// proves admins still see them in the list.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn admin_list_plugin_type_settings_includes_disabled_instance_plugin_after_seed() {
    let app = TestApp::new().await;
    let (admin_token, _tenant_token) = register_admin_and_tenant_user(&app).await;

    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        "enhancement.dashboard-icons",
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("seed plugin type settings row");

    let (status, body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&admin_token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK, "admin list must return 200");
    assert!(
        body.iter()
            .any(|row| row["plugin_type"] == "enhancement.dashboard-icons"),
        "admin list must include the disabled instance-scoped plugin's settings row"
    );
}

/// ADR-0033: a pending-restart-enabled plugin (live row enabled, boot-disabled
/// catalog) is NOT effectively enabled, so it stays hidden from tenant users.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn tenant_user_get_settings_for_pending_restart_enabled_plugin_returns_404() {
    let app = TestApp::new().await;
    let (_admin_token, tenant_token) = register_admin_and_tenant_user(&app).await;

    // TestApp's catalog boots with InstancePluginStates::all_disabled();
    // seeding the live row enabled produces exactly the pending-restart state.
    crate::test_harness::fixtures::upsert_instance_plugin_setting(
        &app,
        "enhancement.dashboard-icons",
        true,
    )
    .await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
        .bearer(&tenant_token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::NOT_FOUND,
        "pending-restart-enabled plugin must stay hidden from tenant users"
    );
}

/// M1.5 OR-gate weaker arm: a `settings_manager`-only principal (seed grant
/// covers only `settings:read`, never `system.settings:manage`) must still
/// be able to list plugin type settings — `authorize_any` accepts the first
/// matching arm.
#[tokio::test]
async fn settings_manager_only_user_can_list_plugin_type_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "settings_manager").await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::OK,
        "settings_manager (settings:read only) must be able to list plugin type settings"
    );
}

/// A principal holding zero role links (and therefore zero `access_grants`
/// coverage) must be denied.
#[tokio::test]
async fn zero_role_user_cannot_list_plugin_type_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_zero_role_user(&app).await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "zero-role user must be denied the plugin type settings list"
    );
}

/// Discriminating regression test for the M1.5 conversion: stages a `viewer`
/// principal whose legacy `role_permissions` snapshot (baked into the JWT at
/// login) still grants `view_settings`, but whose `AccessEngine` authority
/// for `settings:read` / `system.settings:manage` has been revoked (every
/// covering `access_grants` row deleted, cache invalidated — a role may hold
/// more than one seed grant row, so this loops rather than calling `.one()`).
/// Pre-conversion code (checking the legacy permission claim) answers from
/// the stale JWT and returns 200; the engine-gated code must reject with a
/// plain 403.
#[tokio::test]
async fn viewer_engine_deny_overrides_legacy_permission_for_plugin_type_settings_list() {
    let app = TestApp::new().await;
    let client = app.client();
    open_registration(&app).await;

    let (status, auth) =
        register_user(&client, "viewer-stripped@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let token = auth.access_token.expose_secret().to_string();

    let viewer_role_id = role::Entity::find()
        .filter(role::Column::Name.eq("viewer"))
        .one(&app.db)
        .await
        .expect("query roles")
        .expect("seeded viewer role")
        .id;

    let load = load_grants_for_principal(&app.db, app.tenant_id, Uuid::nil(), &[viewer_role_id])
        .await
        .expect("load viewer grants");
    let mut deleted_any = false;
    for grant in load.grants {
        if grant.subject == GrantSubject::Role(viewer_role_id)
            && grant.patterns.iter().any(|pattern| {
                pattern.matches(&actions::SETTINGS_READ)
                    || pattern.matches(&actions::SYSTEM_SETTINGS_MANAGE)
            })
        {
            delete_grant(&app.db, grant.id)
                .await
                .expect("delete viewer settings-covering grant");
            deleted_any = true;
        }
    }
    assert!(
        deleted_any,
        "expected at least one viewer grant row covering settings:read"
    );
    app.state
        .access_engine
        .invalidate_subjects(&[], &[viewer_role_id]);

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "engine must deny the plugin type settings list once the covering grant is revoked, \
         even though the legacy view_settings JWT claim is still present"
    );
}

/// Instance-scoped visibility admin override: a principal holding only
/// `system.settings:manage` (linked `system_administrator` role, no
/// `settings:read`) sees a not-effectively-enabled Instance-scoped plugin's
/// settings row in the list; a `viewer`-only principal (`settings:read`, no
/// `system.settings:manage`) does not.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn system_administrator_sees_disabled_instance_plugin_viewer_does_not() {
    let app = TestApp::new().await;
    open_registration(&app).await;
    let (_admin_id, admin_token) =
        register_user_with_only_role(&app, "sysadmin-only@test.local", "system_administrator")
            .await;
    let (_viewer_id, viewer_token) =
        register_user_with_only_role(&app, "viewer-only@test.local", "viewer").await;

    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        "enhancement.dashboard-icons",
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("seed plugin type settings row");

    let (admin_status, admin_body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&admin_token)
        .send_json()
        .await;
    assert_eq!(admin_status, http::StatusCode::OK);
    assert!(
        admin_body
            .iter()
            .any(|row| row["plugin_type"] == "enhancement.dashboard-icons"),
        "system_administrator (system.settings:manage) must see the disabled instance-scoped plugin"
    );

    let (viewer_status, viewer_body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&viewer_token)
        .send_json()
        .await;
    assert_eq!(viewer_status, http::StatusCode::OK);
    assert!(
        !viewer_body
            .iter()
            .any(|row| row["plugin_type"] == "enhancement.dashboard-icons"),
        "viewer (settings:read only, no system.settings:manage) must not see the disabled \
         instance-scoped plugin"
    );
}
