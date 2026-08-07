//! Integration tests for the visibility predicate on
//! `GET /api/v1/plugin-type-settings/{plugin_type}`.
//!
//! # Why upsert/delete predicate-reject tests are absent
//!
//! The `PUT` and `DELETE` handlers are gated by the `CanManageSystemSettings`
//! action extractor *before* the visibility predicate runs.  A tenant user
//! lacking `system.settings:manage` authority is denied there and never
//! reaches the predicate.  An admin passes the predicate whenever the engine
//! allows `system.settings:manage` — `is_plugin_visible_to_user` asks the
//! `AccessEngine` for that action directly, so a role link alone is not
//! enough: the covering grant must also be present.  There is therefore no
//! reachable code path in the harness
//! test matrix where the predicate would return `false` for upsert/delete —
//! those tests would be vacuous duplicates of the existing extractor-gate tests.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uptrakit_shared_db::entity::user_role;
use uptrakit_shared_types::access::actions;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    open_registration, register_user, revoke_role_grants_covering, role_id_by_name,
    stage_user_with_only_role, stage_zero_role_user,
};
#[cfg(feature = "dashboard-icons")]
use crate::test_harness::fixtures::{register_admin_and_tenant_user, register_user_with_only_role};

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

/// Regression test: a `viewer` principal's role link is left untouched, but
/// every `access_grants` row covering `settings:read` / `system.settings:manage`
/// for that role is deleted and the cache invalidated (a role may hold more
/// than one seed grant row, so this loops rather than calling `.one()`). The
/// deny must be grant-scoped: still being linked to the `viewer` role must
/// not itself grant access once its covering grants are gone.
#[tokio::test]
async fn plugin_type_settings_list_forbidden_after_viewer_grants_revoked() {
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

    let viewer_role_id = role_id_by_name(&app, "viewer").await;

    let user_has_viewer_role = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(auth.user.id))
        .filter(user_role::Column::RoleId.eq(viewer_role_id))
        .one(&app.db)
        .await
        .expect("query user_role")
        .is_some();
    assert!(
        user_has_viewer_role,
        "staged user must actually be linked to the viewer role for this test's grant-deletion \
         to be a meaningful denial, not a 403 for the wrong reason"
    );

    revoke_role_grants_covering(
        &app,
        viewer_role_id,
        &[actions::SETTINGS_READ, actions::SYSTEM_SETTINGS_MANAGE],
    )
    .await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "engine must deny the plugin type settings list once the covering grant is revoked"
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
