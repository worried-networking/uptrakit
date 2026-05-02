#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use crate::database_helpers::fixtures::register_and_get_token;
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_create_enrollment_token(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/enrollment-tokens",
            &serde_json::json!({ "name": "CI Token" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(body["id"].as_str().is_some());
    assert!(body["token"].as_str().is_some());
    assert_eq!(body["name"], "CI Token");
}

db_test!(create_enrollment_token, test_create_enrollment_token);

async fn test_list_enrollment_tokens_hides_secret(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    client
        .post_json(
            "/api/v1/enrollment-tokens",
            &serde_json::json!({ "name": "List Token" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/enrollment-tokens")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("data array");
    assert_eq!(items.len(), 1);
    assert!(items[0].get("token").is_none() || items[0]["token"].is_null());
}

db_test!(
    list_enrollment_tokens_hides_secret,
    test_list_enrollment_tokens_hides_secret
);

async fn test_revoke_enrollment_token(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/enrollment-tokens",
            &serde_json::json!({ "name": "To Revoke" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/enrollment-tokens/{id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

db_test!(revoke_enrollment_token, test_revoke_enrollment_token);
