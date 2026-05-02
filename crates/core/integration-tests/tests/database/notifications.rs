#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use crate::database_helpers::fixtures::{register_and_get_token, seed_permissions_for_owner};
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn setup_with_notification_perms(harness: &TestHarness) -> String {
    seed_permissions_for_owner(&harness.db, &["view_notifications", "manage_notifications"]).await;
    let client = harness.client();
    register_and_get_token(&client).await
}

async fn test_create_webhook_channel(harness: &TestHarness) {
    let token = setup_with_notification_perms(harness).await;
    let client = harness.client();

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

db_test!(create_webhook_channel, test_create_webhook_channel);

async fn test_list_channels(harness: &TestHarness) {
    let token = setup_with_notification_perms(harness).await;
    let client = harness.client();

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

db_test!(list_channels, test_list_channels);

async fn test_delete_channel(harness: &TestHarness) {
    let token = setup_with_notification_perms(harness).await;
    let client = harness.client();

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

db_test!(delete_channel, test_delete_channel);

async fn test_create_notification_rule(harness: &TestHarness) {
    let token = setup_with_notification_perms(harness).await;
    let client = harness.client();

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

db_test!(create_notification_rule, test_create_notification_rule);
