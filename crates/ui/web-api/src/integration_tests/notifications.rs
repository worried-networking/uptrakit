#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{register_and_get_token, seed_permissions_for_owner};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use uptrakit_shared_db::entity::audit_log;

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

/// Seed notification permissions and register an owner user, returning the
/// access token ready for authenticated requests.
async fn setup_with_notification_perms(app: &TestApp) -> String {
    seed_permissions_for_owner(&app.db, &["view_notifications", "manage_notifications"]).await;
    let client = app.client();
    register_and_get_token(&client).await
}

#[tokio::test]
async fn create_webhook_channel_returns_201() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/channels",
            &serde_json::json!({
                "name": "CI Webhook",
                "channel_type": "webhook",
                "config": {
                    "url": "https://example.com/hook"
                }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(body["id"].as_str().is_some());
    assert_eq!(body["name"], "CI Webhook");

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("notification_channel"));
    assert_eq!(row.target_display.as_deref(), Some("CI Webhook"));
    let details = row.details_json.expect("details");
    assert_eq!(details["channel_type"], serde_json::json!("webhook"));
    assert_eq!(details["enabled"], serde_json::json!(true));
}

#[tokio::test]
async fn list_channels_returns_created() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();

    // Create a channel.
    client
        .post_json(
            "/api/v1/notifications/channels",
            &serde_json::json!({
                "name": "Listed Channel",
                "channel_type": "webhook",
                "config": { "url": "https://example.com/hook" }
            }),
        )
        .bearer(&token)
        .send_status()
        .await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/notifications/channels")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn create_rule_returns_201() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();

    // Create a channel first.
    let (_, channel): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/channels",
            &serde_json::json!({
                "name": "Rule Channel",
                "channel_type": "webhook",
                "config": { "url": "https://example.com/hook" }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let channel_id = channel["id"].as_str().expect("channel id");

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/rules",
            &serde_json::json!({
                "channel_id": channel_id,
                "event_type": "update_available"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(body["id"].as_str().is_some());

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_CREATE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_CREATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("notification_rule"));
    let details = row.details_json.expect("details");
    assert_eq!(details["channel_id"], channel["id"]);
    assert_eq!(details["event_type"], serde_json::json!("update_available"));
    assert_eq!(details["enabled"], serde_json::json!(true));
}

#[tokio::test]
async fn delete_channel_returns_204() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();

    let (_, channel): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/channels",
            &serde_json::json!({
                "name": "To Delete",
                "channel_type": "webhook",
                "config": { "url": "https://example.com/hook" }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = channel["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/notifications/channels/{id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_DELETE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_DELETE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("notification_channel"));
    assert_eq!(row.target_id.as_deref(), Some(id));
}

#[tokio::test]
async fn update_missing_channel_writes_denied_audit_event() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();
    let missing_id = uuid::Uuid::now_v7();

    let status = client
        .put_json(
            &format!("/api/v1/notifications/channels/{missing_id}"),
            &serde_json::json!({
                "name": "Renamed",
            }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_UPDATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.target_id.as_deref(),
        Some(missing_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("channel_not_found")
    );
}

#[tokio::test]
async fn update_rule_invalid_body_writes_validation_failed_audit_event() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();

    let (_, channel): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/channels",
            &serde_json::json!({
                "name": "Rule Channel",
                "channel_type": "webhook",
                "config": { "url": "https://example.com/hook" }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let channel_id = channel["id"].as_str().expect("channel id");
    let (_, rule): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/rules",
            &serde_json::json!({
                "channel_id": channel_id,
                "event_type": "update_available"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let rule_id = rule["id"].as_str().expect("rule id");
    let status = client
        .put_json(
            &format!("/api/v1/notifications/rules/{rule_id}"),
            &serde_json::json!({
                "host_id": 123
            }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_UPDATE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
}

#[tokio::test]
async fn delete_missing_rule_writes_denied_audit_event() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();
    let missing_id = uuid::Uuid::now_v7();

    let status = client
        .delete(&format!("/api/v1/notifications/rules/{missing_id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    assert_eq!(
        row.target_id.as_deref(),
        Some(missing_id.to_string().as_str())
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["reason_code"], serde_json::json!("rule_not_found"));
}

#[tokio::test]
async fn delete_rule_db_failure_writes_failed_audit_event() {
    let app = TestApp::new().await;
    let token = setup_with_notification_perms(&app).await;
    let client = app.client();

    let (_, channel): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/channels",
            &serde_json::json!({
                "name": "Rule Channel",
                "channel_type": "webhook",
                "config": { "url": "https://example.com/hook" }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let channel_id = channel["id"].as_str().expect("channel id");
    let (_, rule): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/notifications/rules",
            &serde_json::json!({
                "channel_id": channel_id,
                "event_type": "update_available"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    app.db
        .execute_unprepared("DROP TABLE notification_rules;")
        .await
        .expect("drop notification_rules table");

    let rule_id = rule["id"].as_str().expect("rule id");
    let status = client
        .delete(&format!("/api/v1/notifications/rules/{rule_id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::INTERNAL_SERVER_ERROR);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_DELETE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Failed.as_str()
    );
    assert_eq!(row.target_id.as_deref(), Some(rule_id));
    let details = row.details_json.expect("details");
    assert_eq!(
        details["reason_code"],
        serde_json::json!("rule_delete_failed")
    );
}
