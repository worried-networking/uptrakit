use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;

#[tokio::test]
async fn get_registration_settings_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings/registration")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    // Default mode is "Closed" after first user completes initial setup.
    assert!(body["mode"].as_str().is_some());
}

#[tokio::test]
async fn update_registration_settings_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");

    // Verify persistence by reading again.
    let (s2, body2): (_, serde_json::Value) = client
        .get("/api/v1/settings/registration")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(s2, http::StatusCode::OK);
    assert_eq!(body2["mode"], "open");
}

#[tokio::test]
async fn get_combined_settings_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, _body): (_, serde_json::Value) = client
        .get("/api/v1/settings")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
}
