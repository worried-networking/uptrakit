use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;

#[tokio::test]
async fn create_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();
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
    // The plaintext token is returned on creation.
    assert!(body["token"].as_str().is_some());
    assert_eq!(body["name"], "CI Token");
}

#[tokio::test]
async fn list_returns_created() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create a token first.
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
    // Plaintext token must NOT be returned in list.
    assert!(items[0].get("token").is_none() || items[0]["token"].is_null());
}

#[tokio::test]
async fn revoke_returns_204() {
    let app = TestApp::new().await;
    let client = app.client();
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
