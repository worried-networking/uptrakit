//! Integration tests for `/api/v1/instance-plugins`.

#![cfg_attr(
    feature = "dashboard-icons",
    expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )
)]
#![cfg_attr(
    feature = "dashboard-icons",
    expect(clippy::panic, reason = "test code: panics on failure are acceptable")
)]

#[cfg(feature = "dashboard-icons")]
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
#[cfg(feature = "dashboard-icons")]
use uptrakit_shared_db::entity::system_audit_log;
#[cfg(feature = "dashboard-icons")]
use uptrakit_web_api_types::permissions::Permission;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
#[cfg(feature = "dashboard-icons")]
use crate::test_harness::fixtures::upsert_instance_plugin_setting;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Poll the system audit log up to 50 × 10 ms for the first row with the
/// given `action_type`, ordered by most-recent first.
///
/// Instance-plugin audit entries use `system_scope()` (no tenant_id) and are
/// therefore routed to `system_audit_logs`, not `audit_logs`.
#[cfg(feature = "dashboard-icons")]
async fn poll_system_audit_row(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> system_audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = system_audit_log::Entity::find()
            .filter(system_audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(system_audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query system audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("expected system audit row for action {action_type}");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// GET /api/v1/instance-plugins requires ManageGlobalSettings.
/// A token with only ViewSettings must be rejected with 403.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn list_requires_manage_global_settings() {
    let app = TestApp::new().await;
    let client = app.client();

    let viewer_token = app
        .jwt
        .create_access_token(
            uuid::Uuid::now_v7(),
            &[Permission::ViewSettings],
            "password",
            None,
            None,
        )
        .expect("mint viewer token");

    let status = client
        .get("/api/v1/instance-plugins")
        .bearer(&viewer_token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::FORBIDDEN);
}

/// GET /api/v1/instance-plugins returns all instance-scoped plugins.
/// Pre-seeding dashboard-icons as enabled must be reflected in `enabled`; the
/// catalog snapshot was built before the seed so `running_enabled` stays false.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn list_returns_all_instance_scoped_plugins_with_state() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Seed the setting so the snapshot sees it enabled.
    upsert_instance_plugin_setting(&app, "enhancement_dashboard_icons", true).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/instance-plugins")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("response should be an array");

    let dashboard_icons = entries
        .iter()
        .find(|e| e["plugin_type"] == "enhancement_dashboard_icons")
        .expect("enhancement_dashboard_icons should appear in list");

    assert_eq!(
        dashboard_icons["enabled"], true,
        "enabled must reflect the seeded value"
    );
    assert_eq!(
        dashboard_icons["running_enabled"], false,
        "running_enabled reflects catalog state at boot (InstancePluginStates::all_disabled)"
    );
}

/// PUT /api/v1/instance-plugins/{plugin_type}/enabled persists the flag and
/// emits an INSTANCE_PLUGIN_TOGGLED audit row.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn set_enabled_persists_and_audits() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/instance-plugins/enhancement_dashboard_icons/enabled",
            &serde_json::json!({ "enabled": true }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(
        body["enabled"], true,
        "response must reflect the new enabled state"
    );

    let row = poll_system_audit_row(
        &app.db,
        uptrakit_audit_log::AuditActionType::INSTANCE_PLUGIN_TOGGLED,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::INSTANCE_PLUGIN_TOGGLED,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("instance_plugin"));
    assert_eq!(
        row.target_id.as_deref(),
        Some("enhancement_dashboard_icons")
    );

    let details = row.details_json.expect("audit row must have details");
    assert_eq!(
        details["plugin_type"],
        serde_json::json!("enhancement_dashboard_icons")
    );
    assert_eq!(details["operation"], serde_json::json!("toggle"));
    assert_eq!(details["new_enabled"], serde_json::json!(true));
    assert_eq!(
        details["previous_enabled"],
        serde_json::Value::Null,
        "first toggle must have null previous_enabled"
    );
}

/// PUT .../enabled with an unrecognised plugin type returns 404.
#[tokio::test]
async fn set_enabled_for_unknown_plugin_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .put_json(
            "/api/v1/instance-plugins/totally_made_up/enabled",
            &serde_json::json!({ "enabled": true }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

/// PUT .../enabled for a plugin that exists but is Tenant-scoped must return
/// 404 — the endpoint must not leak existence through a different status code.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn set_enabled_for_tenant_scoped_plugin_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // `package_manager_apt` is a Tenant-scoped plugin in the default catalog.
    let status = client
        .put_json(
            "/api/v1/instance-plugins/package_manager_apt/enabled",
            &serde_json::json!({ "enabled": true }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

/// PUT .../config on a kill-switch-only instance plugin (instance_config = None)
/// must return 400, not 500.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn upsert_config_for_kill_switch_only_plugin_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/instance-plugins/enhancement_dashboard_icons/config",
            &serde_json::json!({ "config": {} }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("no instance configuration schema"),
        "unexpected error message: {msg}"
    );
}

/// PUT .../config with a non-object `config` field is rejected by the Validate
/// impl before route logic runs — the handler must return 400.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn upsert_config_validates_against_validate_trait() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, _body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/instance-plugins/enhancement_dashboard_icons/config",
            &serde_json::json!({ "config": "not-an-object" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

// TODO: upsert_config_validates_against_instance_config_schema_and_persists
//
// This test is deferred because `enhancement_dashboard_icons` has no
// `instance_config` descriptor (it is a kill-switch-only plugin), so covering
// schema validation and persistence would require introducing a synthetic test
// descriptor into the catalog — a significant harness change that is out of
// scope for the initial integration-test pass. Add a real instance_config
// descriptor to a future plugin and cover this path then.
