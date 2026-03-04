use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{register_and_get_token, seed_permissions_for_owner};

/// Seed notification permissions and register an owner user, returning the
/// access token ready for authenticated requests.
async fn setup_with_notification_perms(app: &TestApp) -> String {
    seed_permissions_for_owner(
        &app.db,
        &["view_notifications", "manage_notifications"],
    )
    .await;
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
}
