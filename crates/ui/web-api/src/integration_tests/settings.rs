use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use uptrakit_web_api_types::permissions::Permission;

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

#[tokio::test]
async fn github_provider_settings_forbids_missing_permission() {
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
        .expect("mint reduced-permission token");

    let (get_status, _): (_, serde_json::Value) = client
        .get("/api/v1/global-settings/providers/github")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(get_status, http::StatusCode::FORBIDDEN);

    let (put_status, _): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "ghp_test"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(put_status, http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn github_provider_settings_round_trip_masks_keeps_and_clears_token() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status_initial, initial_body): (_, serde_json::Value) = client
        .get("/api/v1/global-settings/providers/github")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status_initial, http::StatusCode::OK);
    assert_eq!(initial_body["has_auth_token"], false);
    assert!(initial_body.get("auth_token").is_none());
    assert!(initial_body.get("api_base_url").is_none());

    let (status_set, set_body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "ghp_secret123",
                "api_base_url": "https://ghe.example.com/api/v3"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status_set, http::StatusCode::OK);
    assert_eq!(set_body["has_auth_token"], true);
    assert_eq!(set_body["auth_token"], "***");
    assert_eq!(set_body["api_base_url"], "https://ghe.example.com/api/v3");

    let (status_keep, keep_body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "***"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status_keep, http::StatusCode::OK);
    assert_eq!(keep_body["has_auth_token"], true);
    assert_eq!(keep_body["auth_token"], "***");
    assert_eq!(keep_body["api_base_url"], "https://ghe.example.com/api/v3");

    let (status_clear, clear_body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "",
                "api_base_url": ""
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status_clear, http::StatusCode::OK);
    assert_eq!(clear_body["has_auth_token"], false);
    assert!(clear_body.get("auth_token").is_none());
    assert!(clear_body.get("api_base_url").is_none());

    let (status_get_after_clear, get_after_clear_body): (_, serde_json::Value) = client
        .get("/api/v1/global-settings/providers/github")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status_get_after_clear, http::StatusCode::OK);
    assert_eq!(get_after_clear_body["has_auth_token"], false);
    assert!(get_after_clear_body.get("auth_token").is_none());
    assert!(get_after_clear_body.get("api_base_url").is_none());
}
