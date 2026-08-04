#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]
#![expect(
    clippy::string_slice,
    reason = "test code: slice indexes are at validated boundaries"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    insert_host, link_service_host, login_user, register_and_get_token, register_user,
};
#[cfg(feature = "dashboard-icons")]
use sea_orm::{ActiveModelTrait, Set};
#[cfg(not(feature = "dashboard-icons"))]
use sea_orm::{ActiveModelTrait, Set};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::collections::BTreeSet;
use uptrakit_shared_db::access_grants::{GrantSubject, delete_grant, load_grants_for_principal};
use uptrakit_shared_db::entity::audit_log;
#[cfg(feature = "dashboard-icons")]
use uptrakit_shared_db::entity::plugin_config;
use uptrakit_shared_db::entity::{role, service, user_role};
use uptrakit_shared_types::access::actions;
use uptrakit_wire::{ControllerMessage, TestPluginConfigResultPayload};
use uuid::Uuid;

async fn tenant_audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected tenant audit row");
}

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

#[tokio::test]
async fn list_plugin_types_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    // The response is an array of plugin type metadata.
    assert!(body.as_array().is_some());
    assert!(
        !body.as_array().expect("array").is_empty(),
        "at least one plugin type should be registered"
    );
}

/// M1.5 OR-gate weaker arm: a `settings_manager`-only principal (seed grant
/// covers only `settings:read`, never `software:read` or
/// `system.settings:manage`) must still be able to list plugin types.
#[tokio::test]
async fn list_plugin_types_allows_view_settings_without_view_software() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "settings_manager").await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body.as_array().is_some(),
        "plugin types response should be array"
    );
}

/// A `system_administrator`-only principal (seed grant `system.*:*`, which
/// covers `system.settings:manage` but neither `settings:read` nor
/// `software:read`) must still be able to list plugin types.
#[tokio::test]
async fn list_plugin_types_allows_manage_global_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "system_administrator").await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body.as_array().is_some(),
        "plugin types response should be array"
    );
}

/// Same OR-gate arm proof as above, for the `/api/v1/plugin-type-settings`
/// list route.
#[tokio::test]
async fn list_plugin_type_settings_allows_manage_global_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "system_administrator").await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body.as_array().is_some(),
        "plugin type settings response should be array"
    );
}

/// A principal holding zero role links (and therefore zero `access_grants`
/// coverage) must be denied.
#[tokio::test]
async fn zero_role_user_cannot_list_plugin_types() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_zero_role_user(&app).await;

    let status = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "zero-role user must be denied the plugin types list"
    );
}

/// Discriminating regression test for the M1.5 conversion: stages a `viewer`
/// principal whose legacy `role_permissions` snapshot (baked into the JWT at
/// login) still grants `view_software` and `view_settings`, but whose
/// `AccessEngine` authority for `software:read` / `settings:read` /
/// `system.settings:manage` has been revoked (every covering `access_grants`
/// row deleted, cache invalidated — a role may hold more than one seed grant
/// row, so this loops rather than calling `.one()`). Pre-conversion code
/// (checking the legacy permission claim) answers from the stale JWT and
/// returns 200; the engine-gated code must reject with a plain 403.
#[tokio::test]
async fn viewer_engine_deny_overrides_legacy_permission_for_plugin_types_list() {
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
                pattern.matches(&actions::SOFTWARE_READ)
                    || pattern.matches(&actions::SETTINGS_READ)
                    || pattern.matches(&actions::SYSTEM_SETTINGS_MANAGE)
            })
        {
            delete_grant(&app.db, grant.id)
                .await
                .expect("delete viewer software/settings-covering grant");
            deleted_any = true;
        }
    }
    assert!(
        deleted_any,
        "expected at least one viewer grant row covering software:read/settings:read"
    );
    app.state
        .access_engine
        .invalidate_subjects(&[], &[viewer_role_id]);

    let status = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "engine must deny the plugin types list once the covering grant is revoked, even \
         though the legacy view_software/view_settings JWT claim is still present"
    );
}

#[tokio::test]
async fn create_config_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "My GitHub Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(body["id"].as_str().is_some());
    assert_eq!(body["name"], "My GitHub Config");

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_display.as_deref(), Some("My GitHub Config"));
    let details = row.details_json.expect("details");
    assert_eq!(details["plugin_type"], serde_json::json!("releases.github"));
    assert_eq!(
        details["config_name"],
        serde_json::json!("My GitHub Config")
    );
    assert_eq!(details["enabled"], serde_json::json!(true));
    assert_eq!(details["contains_command_fields"], serde_json::json!(false));
}

#[tokio::test]
async fn update_config_returns_200_and_writes_audit_event() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Mutable Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");
    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "name": "Mutable Config Updated",
                "enabled": false
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["name"], "Mutable Config Updated");
    assert_eq!(body["enabled"], false);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_id.as_deref(), Some(id));
    assert_eq!(
        row.target_display.as_deref(),
        Some("Mutable Config Updated")
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["plugin_type"], serde_json::json!("releases.github"));
    assert_eq!(
        details["config_name"],
        serde_json::json!("Mutable Config Updated")
    );
    assert_eq!(details["enabled"], serde_json::json!(false));
    assert_eq!(details["contains_command_fields"], serde_json::json!(false));
}

#[tokio::test]
async fn delete_config_returns_204() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "To Delete Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/plugin-configs/{id}"))
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_id.as_deref(), Some(id));
    assert_eq!(row.target_display.as_deref(), Some("To Delete Config"));
    let details = row.details_json.expect("details");
    assert_eq!(details["plugin_type"], serde_json::json!("releases.github"));
    assert_eq!(
        details["config_name"],
        serde_json::json!("To Delete Config")
    );
    assert_eq!(details["enabled"], serde_json::json!(true));
}

async fn insert_service_with_id(
    app: &TestApp,
    id: Uuid,
    status: service::ServiceStatus,
) -> service::Model {
    let now = time::OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(id),
        tenant_id: Set(app.tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("host-{}", &id.to_string()[..8])),
        friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
        ip_address: Set(Some("10.0.0.1".to_string())),
        status: Set(status),
        enrollment_secret_hash: Set(format!("secret-{id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(None),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(None),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert service")
}

#[tokio::test]
async fn test_plugin_config_prefers_active_agent_when_stale_link_exists() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let host = insert_host(&app.db, app.tenant_id).await;
    let stale_service =
        insert_service_with_id(&app, Uuid::from_u128(1), service::ServiceStatus::Approved).await;
    let active_service =
        insert_service_with_id(&app, Uuid::from_u128(2), service::ServiceStatus::Approved).await;

    link_service_host(&app.db, stale_service.id, host.id).await;
    link_service_host(&app.db, active_service.id, host.id).await;

    service::ActiveModel {
        id: Set(stale_service.id),
        deactivated_at: Set(Some(time::OffsetDateTime::now_utc())),
        ..stale_service.into()
    }
    .update(&app.db)
    .await
    .expect("deactivate stale service");

    let (mut rx, _handle) = app
        .state
        .service_connections
        .register(active_service.id, BTreeSet::new(), None, None, None)
        .await;
    let proxy = app.state.config_test_proxy.clone();
    tokio::spawn(async move {
        match rx.recv().await {
            Some(ControllerMessage::TestPluginConfig(payload)) => {
                let request_id = payload.request_id.clone();
                proxy.complete(
                    &request_id,
                    TestPluginConfigResultPayload::new(request_id.clone(), true, 1),
                );
            }
            other => panic!("unexpected message: {other:?}"),
        }
    });

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs/test",
            &serde_json::json!({
                "plugin_type": "generic.shell",
                "config": { "version_command": "echo 1.0.0" },
                "host_id": host.id,
                "test_kind": "version_detection"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["success"], true);
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn list_plugin_types_includes_dashboard_icons_type_settings() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("plugin types should be an array");
    let dashboard_icons = entries
        .iter()
        .find(|entry| entry["plugin_type"] == "enhancement.dashboard-icons")
        .expect("dashboard icons plugin type should be present");

    assert_eq!(dashboard_icons["display_name"], "Dashboard Icons");
    assert_eq!(
        dashboard_icons["supports_plugin_configs"],
        serde_json::json!(false)
    );
    let type_fields = dashboard_icons["type_settings_form_fields"]
        .as_array()
        .expect("type settings fields should be an array");
    assert!(
        type_fields
            .iter()
            .any(|field| field["key"] == "enabled" && field["field_type"] == "toggle"),
        "dashboard icons type settings should expose an enabled toggle"
    );
    assert_eq!(
        dashboard_icons["type_settings_sample"],
        serde_json::json!({ "enabled": true })
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn create_config_rejects_config_model_none_plugin_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Should Be Rejected",
                "plugin_type": "enhancement.dashboard-icons",
                "config": {}
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("does not support per-instance plugin configs"),
        "unexpected error message: {msg}"
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn update_config_rejects_existing_config_model_none_plugin_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let config_id = insert_plugin_config_for_dashboard_icons(&app).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{config_id}"),
            &serde_json::json!({
                "name": "Rejected Update"
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("does not support per-instance plugin configs"),
        "unexpected error message: {msg}"
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn test_config_rejects_config_model_none_plugin_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs/test",
            &serde_json::json!({
                "plugin_type": "enhancement.dashboard-icons",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("does not support per-instance plugin configs"),
        "unexpected error message: {msg}"
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn upsert_type_settings_rejects_invalid_dashboard_icons_enabled_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/plugin-type-settings/enhancement.dashboard-icons",
            &serde_json::json!({
                "config": {
                    "enabled": "false"
                }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("Invalid plugin type settings"),
        "unexpected error message: {msg}"
    );

    let missing_status = client
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(missing_status, http::StatusCode::NOT_FOUND);
}

/// Register the admin (first user), re-open registration, then register a
/// second user who gets the built-in "user" role (ViewSettings but NOT
/// ManageGlobalSettings).  Returns `(admin_token, tenant_token)`.
#[cfg(feature = "dashboard-icons")]
async fn register_admin_and_tenant_user(app: &TestApp) -> (String, String) {
    let client = app.client();

    // First registered user becomes owner (all permissions including ManageGlobalSettings).
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
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&admin_token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );

    // Second user gets the built-in "user" role: ViewSettings but NOT ManageGlobalSettings.
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

/// A tenant user (ViewSettings, no ManageGlobalSettings) must not see a
/// disabled Instance-scoped plugin in the `GET /api/v1/plugin-types` list.
///
/// `enhancement.dashboard-icons` is Instance-scoped and the snapshot defaults
/// to `all_disabled()` at TestApp boot, so it must be absent from the response.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn tenant_user_does_not_see_disabled_instance_plugin_in_plugin_types_list() {
    let app = TestApp::new().await;
    let (_admin_token, tenant_token) = register_admin_and_tenant_user(&app).await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&tenant_token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("plugin-types must be an array");
    assert!(
        !entries
            .iter()
            .any(|e| e["plugin_type"] == "enhancement.dashboard-icons"),
        "tenant user must not see disabled instance-scoped plugin; entries: {entries:?}"
    );
}

/// An admin (ManageGlobalSettings) must see a disabled Instance-scoped plugin
/// in the `GET /api/v1/plugin-types` list — the predicate passes for owners.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn admin_sees_disabled_instance_plugin_in_plugin_types_list() {
    let app = TestApp::new().await;
    let (admin_token, _tenant_token) = register_admin_and_tenant_user(&app).await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&admin_token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("plugin-types must be an array");
    assert!(
        entries
            .iter()
            .any(|e| e["plugin_type"] == "enhancement.dashboard-icons"),
        "admin must see disabled instance-scoped plugin; entries: {entries:?}"
    );
}

#[cfg(feature = "dashboard-icons")]
async fn insert_plugin_config_for_dashboard_icons(app: &TestApp) -> Uuid {
    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();

    plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(app.tenant_id),
        name: Set("Existing Dashboard Icons Config".to_string()),
        plugin_type: Set("enhancement.dashboard-icons".to_string()),
        config: Set(serde_json::json!({})),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert plugin config");

    id
}
