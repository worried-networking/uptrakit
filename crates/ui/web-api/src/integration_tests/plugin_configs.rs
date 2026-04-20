use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{insert_host, link_service_host, register_and_get_token};
#[cfg(feature = "dashboard-icons")]
use sea_orm::{ActiveModelTrait, Set};
#[cfg(not(feature = "dashboard-icons"))]
use sea_orm::{ActiveModelTrait, Set};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::collections::BTreeSet;
use uptrakit_internal_wire::{ControllerMessage, TestPluginConfigResultPayload};
use uptrakit_shared_db::entity::audit_log;
#[cfg(feature = "dashboard-icons")]
use uptrakit_shared_db::entity::plugin_config;
use uptrakit_shared_db::entity::service;
use uptrakit_web_api_types::permissions::Permission;
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

#[tokio::test]
async fn list_plugin_types_allows_view_settings_without_view_software() {
    let app = TestApp::new().await;
    let client = app.client();

    let token = app
        .jwt
        .create_access_token(
            uuid::Uuid::now_v7(),
            &[Permission::ViewSettings],
            "password",
            None,
        )
        .expect("mint settings-only token");

    let (status, body): (_, serde_json::Value) = client
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

#[tokio::test]
async fn list_plugin_types_allows_manage_global_settings() {
    let app = TestApp::new().await;
    let client = app.client();

    let token = app
        .jwt
        .create_access_token(
            uuid::Uuid::now_v7(),
            &[Permission::ManageGlobalSettings],
            "password",
            None,
        )
        .expect("mint global-settings token");

    let (status, body): (_, serde_json::Value) = client
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

#[tokio::test]
async fn list_plugin_type_settings_allows_manage_global_settings() {
    let app = TestApp::new().await;
    let client = app.client();

    let token = app
        .jwt
        .create_access_token(
            uuid::Uuid::now_v7(),
            &[Permission::ManageGlobalSettings],
            "password",
            None,
        )
        .expect("mint global-settings token");

    let (status, body): (_, serde_json::Value) = client
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
                "plugin_type": "releases_github",
                "config": {}
            }),
        )
        .bearer(&token)
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
    assert_eq!(details["plugin_type"], serde_json::json!("releases_github"));
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
                "plugin_type": "releases_github",
                "config": {}
            }),
        )
        .bearer(&token)
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
    assert_eq!(details["plugin_type"], serde_json::json!("releases_github"));
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
                "plugin_type": "releases_github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/plugin-configs/{id}"))
        .bearer(&token)
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
    assert_eq!(details["plugin_type"], serde_json::json!("releases_github"));
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
                "plugin_type": "generic_shell",
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
        .find(|entry| entry["plugin_type"] == "enhancement_dashboard_icons")
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
                "plugin_type": "enhancement_dashboard_icons",
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
                "plugin_type": "enhancement_dashboard_icons",
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
            "/api/v1/plugin-type-settings/enhancement_dashboard_icons",
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
        .get("/api/v1/plugin-type-settings/enhancement_dashboard_icons")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(missing_status, http::StatusCode::NOT_FOUND);
}

#[cfg(feature = "dashboard-icons")]
async fn insert_plugin_config_for_dashboard_icons(app: &TestApp) -> Uuid {
    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();

    plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(app.tenant_id),
        name: Set("Existing Dashboard Icons Config".to_string()),
        plugin_type: Set("enhancement_dashboard_icons".to_string()),
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
