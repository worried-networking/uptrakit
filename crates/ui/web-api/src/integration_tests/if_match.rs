#![expect(
    clippy::let_underscore_must_use,
    reason = "crypto provider install returns Result; already-installed is not an error in tests"
)]

use uptrakit_config_reload::config::Scope;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;

fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

/// PUT without `If-Match` header → 428 Precondition Required.
#[tokio::test]
async fn put_settings_without_if_match_returns_428() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::PRECONDITION_REQUIRED);
}

/// PUT with stale `If-Match` → 409 Conflict.
///
/// The cache is pre-seeded to version 1 so that the client's `W/"settings-v0"`
/// is treated as stale.
#[tokio::test]
async fn put_settings_with_stale_etag_returns_409() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Advance cache to version 1 so v0 is stale.
    app.state
        .settings_version_cache
        .update(Scope::Tenant(app.state.default_tenant_id), 1);

    let status = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
}

/// PUT with the current `If-Match` ETag → 200 OK.
#[tokio::test]
async fn put_settings_with_current_etag_returns_200() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Cache at version 0 (default); W/"settings-v0" must be accepted.
    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");
}

/// After the cache is advanced to version 1, W/"settings-v1" is accepted and
/// W/"settings-v0" is rejected, showing that the version discriminates.
#[tokio::test]
async fn etag_version_discriminates_correctly() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Advance to version 2 so both v0 and v1 are stale but v2 is current.
    app.state
        .settings_version_cache
        .update(Scope::Tenant(app.state.default_tenant_id), 2);

    // v0 is stale → 409.
    let s0 = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(s0, http::StatusCode::CONFLICT);

    // v1 is stale → 409.
    let s1 = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v1\"")
        .send_status()
        .await;
    assert_eq!(s1, http::StatusCode::CONFLICT);

    // v2 is current → 200.
    let (s2, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v2\"")
        .send_json()
        .await;
    assert_eq!(s2, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");
}

/// Weak ETag prefix (`W/`) is stripped before comparison.
#[tokio::test]
async fn weak_etag_prefix_stripped_before_comparison() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Both `W/"settings-v0"` and `"settings-v0"` (without W/) should match.
    let (status, _): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", "\"settings-v0\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
}

/// Full GET→ETag→PUT round-trip: ETag from GET is accepted by PUT.
#[tokio::test]
async fn get_returns_etag_header() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let resp = client
        .get("/api/v1/settings/registration")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::OK);
    let etag = resp
        .headers()
        .get("etag")
        .expect("ETag header present")
        .to_str()
        .expect("ETag is ASCII")
        .to_string();
    assert!(
        etag.contains("settings-v0"),
        "expected settings-v0 in ETag, got {etag:?}"
    );
}

/// After a successful PUT, the ETag version increments so the old value is stale.
#[tokio::test]
async fn put_increments_etag_version() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // GET → current ETag (v0).
    let resp0 = client
        .get("/api/v1/settings/registration")
        .bearer(&token)
        .send()
        .await;
    let etag0 = resp0
        .headers()
        .get("etag")
        .expect("ETag on first GET")
        .to_str()
        .expect("ASCII")
        .to_string();

    // PUT with current ETag → 200.
    let (put_status, _body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag0)
        .send_json()
        .await;
    assert_eq!(put_status, http::StatusCode::OK);

    // GET → ETag incremented to v1.
    let resp1 = client
        .get("/api/v1/settings/registration")
        .bearer(&token)
        .send()
        .await;
    let etag1 = resp1
        .headers()
        .get("etag")
        .expect("ETag on second GET")
        .to_str()
        .expect("ASCII")
        .to_string();
    assert!(
        etag1.contains("settings-v1"),
        "expected settings-v1 after PUT, got {etag1:?}"
    );

    // Old ETag is now stale → 409.
    let stale_status = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag0)
        .send_status()
        .await;
    assert_eq!(stale_status, http::StatusCode::CONFLICT);

    // New ETag → 200.
    let (fresh_status, fresh_body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag1)
        .send_json()
        .await;
    assert_eq!(fresh_status, http::StatusCode::OK);
    assert_eq!(fresh_body["mode"], "open");
}
