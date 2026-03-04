use crate::test_harness::TestApp;

#[tokio::test]
async fn unauthenticated_endpoints_return_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let endpoints = [
        "/api/v1/auth/me",
        "/api/v1/services",
        "/api/v1/hosts",
        "/api/v1/software-items",
        "/api/v1/enrollment-tokens",
        "/api/v1/settings/registration",
        "/api/v1/notifications/channels",
        "/api/v1/notifications/rules",
        "/api/v1/plugin-types",
        "/api/v1/plugin-configs",
        "/api/v1/settings",
    ];

    for endpoint in endpoints {
        let status = client.get(endpoint).send_status().await;
        assert_eq!(
            status,
            http::StatusCode::UNAUTHORIZED,
            "{endpoint} should require authentication"
        );
    }
}

#[tokio::test]
async fn unknown_api_path_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .get("/api/v1/nonexistent")
        .header("accept", "application/json")
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn expired_jwt_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    // Build a JWT that has already expired by using a custom claim set with
    // exp in the past. The JwtManager only creates valid tokens, so we use
    // jsonwebtoken directly.
    let claims = serde_json::json!({
        "sub": uuid::Uuid::now_v7().to_string(),
        "jti": uuid::Uuid::now_v7().to_string(),
        "permissions": ["ViewSettings"],
        "auth_method": "password",
        "iat": 1_000_000,
        "exp": 1_000_001,
        "iss": "uptrakit",
        "aud": ["uptrakit"],
    });

    let header = jsonwebtoken::Header::default();
    let key = jsonwebtoken::EncodingKey::from_secret(b"integration-test-jwt-secret-key-do-not-use");
    let expired_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode expired token");

    let status = client
        .get("/api/v1/auth/me")
        .bearer(&expired_token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}
