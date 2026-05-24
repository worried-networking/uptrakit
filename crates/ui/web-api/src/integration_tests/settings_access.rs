#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel sends in tests drop results intentionally"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use crate::test_harness::http_client::TestClient;

fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

/// Read the current ETag for `/api/v1/settings/access` so tests that issue PUTs
/// do not hard-code a version number (fragile if `TestApp::new()` ever bumps it
/// during initialisation).
async fn current_access_etag(client: &TestClient, token: &str) -> String {
    let res = client
        .get("/api/v1/settings/access")
        .bearer(token)
        .send()
        .await;
    res.headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("W/\"settings-v0\"")
        .to_string()
}

#[tokio::test]
async fn get_access_settings_returns_200_with_expected_fields() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings/access")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body["mode"].as_str().is_some(),
        "mode field must be present"
    );
    assert!(
        body["password_auth_enabled"].as_bool().is_some(),
        "password_auth_enabled field must be present"
    );
    assert!(
        body["two_factor_required"].as_bool().is_some(),
        "two_factor_required field must be present"
    );
    assert!(
        body["require_token_for_oidc"].as_bool().is_some(),
        "require_token_for_oidc field must be present"
    );
}

#[tokio::test]
async fn get_access_settings_returns_etag_header() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let res = client
        .get("/api/v1/settings/access")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(res.status(), http::StatusCode::OK);
    assert!(
        res.headers().get("etag").is_some(),
        "GET /api/v1/settings/access must include ETag header"
    );
}

#[tokio::test]
async fn get_access_settings_requires_auth() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();

    let (status, _): (_, serde_json::Value) =
        client.get("/api/v1/settings/access").send_json().await;

    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn update_access_settings_open_mode_returns_200() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");
}

#[tokio::test]
async fn update_access_settings_returns_etag_header() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let res = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send()
        .await;

    assert_eq!(res.status(), http::StatusCode::OK);
    assert!(
        res.headers().get("etag").is_some(),
        "PUT response must include ETag so clients can chain subsequent saves without a GET"
    );
}

#[tokio::test]
async fn update_access_settings_invite_without_token_returns_422() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let (status, _): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "invite" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn update_access_settings_invite_with_token_returns_200() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "invite", "token": "secret123" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "invite");
}

#[tokio::test]
async fn update_access_settings_persists_on_get() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let put_status = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open", "password_auth_enabled": true }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_status()
        .await;
    assert_eq!(put_status, http::StatusCode::OK);

    let (get_status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings/access")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(get_status, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");
}

#[tokio::test]
async fn get_combined_settings_excludes_access_fields() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body.get("registration").is_none(),
        "registration must be absent from combined settings (moved to /settings/access)"
    );
    assert!(
        body.get("authentication").is_none(),
        "authentication must be absent from combined settings (moved to /settings/access)"
    );
    assert!(
        body.get("agent_certificates").is_some(),
        "agent_certificates must be present"
    );
    assert!(
        body.get("enrollment_tokens").is_some(),
        "enrollment_tokens must be present"
    );
    assert!(
        body.get("multi_tenancy_enabled").is_some(),
        "multi_tenancy_enabled must be present"
    );
}
