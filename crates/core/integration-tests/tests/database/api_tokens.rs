use crate::database_helpers::fixtures::register_and_get_token;
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_create_api_token(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/auth/api-tokens",
            &serde_json::json!({ "name": "CI API Token" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(body["id"].as_str().is_some());
    assert!(body["token"].as_str().is_some());
}

db_test!(create_api_token, test_create_api_token);

async fn test_list_api_tokens(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    client
        .post_json(
            "/api/v1/auth/api-tokens",
            &serde_json::json!({ "name": "Listed Token" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/auth/api-tokens")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["tokens"].as_array().expect("tokens array");
    assert_eq!(items.len(), 1);
}

db_test!(list_api_tokens, test_list_api_tokens);

async fn test_revoke_api_token(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/auth/api-tokens",
            &serde_json::json!({ "name": "To Revoke" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/auth/api-tokens/{id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

db_test!(revoke_api_token, test_revoke_api_token);

async fn test_authenticate_with_api_token(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/auth/api-tokens",
            &serde_json::json!({ "name": "Auth Token" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let api_token = created["token"].as_str().expect("api token");

    // Use the API token to authenticate.
    let (status, _body): (_, serde_json::Value) = client
        .get("/api/v1/services")
        .bearer(api_token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
}

db_test!(
    authenticate_with_api_token,
    test_authenticate_with_api_token
);
