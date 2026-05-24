#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel sends in tests drop results intentionally"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use http_body_util::BodyExt;
use uptrakit_web_api_types::permissions::Permission;

fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

async fn seed_invalid_github_provider_record(
    db: &sea_orm::DatabaseConnection,
) -> uptrakit_shared_db::provider_settings::Result<()> {
    uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
        db,
        None,
        Some("https://ghe.example.com/api/v3"),
    )
    .await
}

#[tokio::test]
async fn get_combined_settings_returns_ok_shape() {
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
        body.get("agent_certificates").is_some(),
        "agent_certificates missing from combined settings"
    );
    assert!(
        body.get("enrollment_tokens").is_some(),
        "enrollment_tokens missing from combined settings"
    );
    assert!(
        body.get("multi_tenancy_enabled").is_some(),
        "multi_tenancy_enabled missing from combined settings"
    );
}

#[tokio::test]
async fn github_provider_settings_forbids_missing_permission() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();

    let token = app
        .jwt
        .create_access_token(
            uuid::Uuid::now_v7(),
            &[Permission::ViewSettings],
            "password",
            None,
            None,
        )
        .expect("mint reduced-permission token");

    let (get_status, _): (_, serde_json::Value) = client
        .get("/api/v1/global-settings/providers/github")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(get_status, http::StatusCode::FORBIDDEN);

    // The ETag middleware runs before permission extractors; add If-Match so the
    // middleware passes through and the permission check can produce 403.
    let (put_status, _): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "ghp_test"
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send_json()
        .await;
    assert_eq!(put_status, http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn github_provider_settings_round_trip_masks_keeps_and_clears_token() {
    ensure_crypto_provider();
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

    // GET the current ETag before the first write.
    let get_resp = client
        .get("/api/v1/global-settings/providers/github")
        .bearer(&token)
        .send()
        .await;
    let etag = get_resp
        .headers()
        .get("etag")
        .expect("ETag on GET")
        .to_str()
        .expect("ASCII")
        .to_string();

    let set_resp = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "ghp_secret123",
                "api_base_url": "https://ghe.example.com/api/v3"
            }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send()
        .await;
    assert_eq!(set_resp.status(), http::StatusCode::OK);
    let set_etag = set_resp
        .headers()
        .get("etag")
        .expect("ETag on first PUT")
        .to_str()
        .expect("ASCII")
        .to_string();
    let set_body: serde_json::Value = serde_json::from_slice(
        &set_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(set_body["has_auth_token"], true);
    assert_eq!(set_body["auth_token"], "***");
    assert_eq!(set_body["api_base_url"], "https://ghe.example.com/api/v3");

    let keep_resp = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "***"
            }),
        )
        .bearer(&token)
        .header("if-match", &set_etag)
        .send()
        .await;
    assert_eq!(keep_resp.status(), http::StatusCode::OK);
    let keep_etag = keep_resp
        .headers()
        .get("etag")
        .expect("ETag on second PUT")
        .to_str()
        .expect("ASCII")
        .to_string();
    let keep_body: serde_json::Value = serde_json::from_slice(
        &keep_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(keep_body["has_auth_token"], true);
    assert_eq!(keep_body["auth_token"], "***");
    assert_eq!(keep_body["api_base_url"], "https://ghe.example.com/api/v3");

    let clear_resp = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "",
                "api_base_url": ""
            }),
        )
        .bearer(&token)
        .header("if-match", &keep_etag)
        .send()
        .await;
    assert_eq!(clear_resp.status(), http::StatusCode::OK);
    let clear_body: serde_json::Value = serde_json::from_slice(
        &clear_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
    )
    .unwrap();
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

#[tokio::test]
async fn github_provider_settings_update_invalidates_runtime() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let runtime = app.state.global_providers().github();
    let _ = runtime.github_client().await.expect("initial client");
    let first_generation = runtime
        .cached_generation_for_tests()
        .expect("initial generation");

    let (status, _body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "ghp_rotated",
                "api_base_url": "https://ghe.example.com/api/v3"
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::OK);

    let _ = runtime.github_client().await.expect("rebuilt client");
    let second_generation = runtime
        .cached_generation_for_tests()
        .expect("rotated generation");
    assert_ne!(first_generation, second_generation);
}

#[tokio::test]
async fn github_provider_settings_invalid_update_returns_400_without_persisting_invalid_values() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, _body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "",
                "api_base_url": "https://ghe.example.com/api/v3"
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::BAD_REQUEST);

    let (get_status, body): (_, serde_json::Value) = client
        .get("/api/v1/global-settings/providers/github")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(get_status, http::StatusCode::OK);
    assert_eq!(body["has_auth_token"], false);
    assert!(body.get("api_base_url").is_none());
}

#[tokio::test]
async fn github_provider_settings_trims_whitespace_only_auth_token_to_clear() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/providers/github",
            &serde_json::json!({
                "auth_token": "   "
            }),
        )
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["has_auth_token"], false);
    assert!(body.get("api_base_url").is_none());
}

#[tokio::test]
async fn system_alerts_include_invalid_global_github_provider_record() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    seed_invalid_github_provider_record(&app.db)
        .await
        .expect("seed invalid provider record");

    let client = app.client();
    let token = register_and_get_token(&client).await;
    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/system/alerts")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let alerts = body["alerts"].as_array().expect("alerts array");
    assert!(alerts.iter().any(|alert| {
        alert["id"] == "global_github_provider_invalid"
            && alert["severity"] == "error"
            && alert["message"]
                .as_str()
                .is_some_and(|message| message.contains("api_base_url requires auth_token"))
    }));
}
