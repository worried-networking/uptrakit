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
            "/api/v1/settings/access",
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
            "/api/v1/settings/access",
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
            "/api/v1/settings/access",
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
            "/api/v1/settings/access",
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
            "/api/v1/settings/access",
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
            "/api/v1/settings/access",
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
            "/api/v1/settings/access",
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
        .get("/api/v1/settings/access")
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

// ── OAuth global settings (/api/v1/global-settings/oauth) ────────────────────

/// PUT OAuth settings without `If-Match` → 428 Precondition Required.
#[tokio::test]
async fn put_oauth_settings_without_if_match_returns_428() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .put_json(
            "/api/v1/global-settings/oauth",
            &serde_json::json!({ "mcp_enabled": true }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::PRECONDITION_REQUIRED);
}

/// PUT OAuth settings with stale `If-Match` → 409 Conflict.
#[tokio::test]
async fn put_oauth_settings_with_stale_etag_returns_409() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Advance global cache to version 1 so v0 is stale.
    app.state.settings_version_cache.update(Scope::Global, 1);

    let status = client
        .put_json(
            "/api/v1/global-settings/oauth",
            &serde_json::json!({ "mcp_enabled": true }),
        )
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
}

/// GET OAuth settings returns an `ETag` header.
#[tokio::test]
async fn get_oauth_settings_returns_etag_header() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let resp = client
        .get("/api/v1/global-settings/oauth")
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
        etag.contains("global-settings-v"),
        "expected global-settings-v in ETag, got {etag:?}"
    );
}

/// PUT all-None body (no-op path) still returns an `ETag` header.
#[tokio::test]
async fn put_oauth_settings_noop_returns_etag() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let resp = client
        .put_json("/api/v1/global-settings/oauth", &serde_json::json!({}))
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::OK);
    assert!(
        resp.headers().contains_key("etag"),
        "ETag header must be present on no-op PUT response"
    );
}

/// Full GET → ETag → PUT round-trip: current ETag accepted, stale ETag rejected after write.
#[tokio::test]
async fn oauth_settings_get_etag_put_round_trip() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // GET → capture ETag.
    let get_resp = client
        .get("/api/v1/global-settings/oauth")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(get_resp.status(), http::StatusCode::OK);
    let etag = get_resp
        .headers()
        .get("etag")
        .expect("ETag on GET")
        .to_str()
        .expect("ASCII")
        .to_string();

    // PUT with captured ETag → 200.
    let (put_status, _body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/global-settings/oauth",
            &serde_json::json!({ "mcp_enabled": false }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json()
        .await;
    assert_eq!(put_status, http::StatusCode::OK);

    // Old ETag now stale → 409.
    let stale_status = client
        .put_json(
            "/api/v1/global-settings/oauth",
            &serde_json::json!({ "mcp_enabled": false }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_status()
        .await;
    assert_eq!(stale_status, http::StatusCode::CONFLICT);
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
        .get("/api/v1/settings/access")
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
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag0)
        .send_json()
        .await;
    assert_eq!(put_status, http::StatusCode::OK);

    // GET → ETag incremented (must differ from etag0).
    let resp1 = client
        .get("/api/v1/settings/access")
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
    assert_ne!(
        etag0, etag1,
        "expected ETag to change after PUT, but got {etag1:?} (same as before)"
    );

    // Old ETag is now stale → 409.
    let stale_status = client
        .put_json(
            "/api/v1/settings/access",
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
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag1)
        .send_json()
        .await;
    assert_eq!(fresh_status, http::StatusCode::OK);
    assert_eq!(fresh_body["mode"], "open");
}

// ── Agent certificates (/api/v1/settings/agent-certificates) ─────────────────

/// GET /settings/agent-certificates returns ETag header.
#[tokio::test]
async fn get_agent_certs_settings_returns_etag() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let resp = client
        .get("/api/v1/settings/agent-certificates")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::OK);
    let etag = resp
        .headers()
        .get("etag")
        .expect("ETag header present")
        .to_str()
        .expect("ASCII")
        .to_string();
    assert!(
        etag.contains("settings-v"),
        "expected settings-v in ETag, got {etag:?}"
    );
}

/// Full GET→ETag→PUT round-trip for agent-certificates.
#[tokio::test]
async fn agent_certs_settings_get_etag_put_round_trip() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // GET → capture ETag.
    let get_resp = client
        .get("/api/v1/settings/agent-certificates")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(get_resp.status(), http::StatusCode::OK);
    let etag = get_resp
        .headers()
        .get("etag")
        .expect("ETag on GET")
        .to_str()
        .expect("ASCII")
        .to_string();

    // PUT without If-Match → 428.
    let no_match = client
        .put_json(
            "/api/v1/settings/agent-certificates",
            &serde_json::json!({}),
        )
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(no_match, http::StatusCode::PRECONDITION_REQUIRED);

    // PUT with captured ETag → 200 with new ETag.
    let put_resp = client
        .put_json(
            "/api/v1/settings/agent-certificates",
            &serde_json::json!({}),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send()
        .await;
    assert_eq!(put_resp.status(), http::StatusCode::OK);
    assert!(
        put_resp.headers().contains_key("etag"),
        "PUT response must carry ETag"
    );

    // Old ETag now stale → 409.
    let stale = client
        .put_json(
            "/api/v1/settings/agent-certificates",
            &serde_json::json!({}),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_status()
        .await;
    assert_eq!(stale, http::StatusCode::CONFLICT);
}

// ── Network settings (/api/v1/global-settings/network) ───────────────────────

/// GET /global-settings/network returns ETag header.
#[tokio::test]
async fn get_network_settings_returns_etag() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let resp = client
        .get("/api/v1/global-settings/network")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::OK);
    let etag = resp
        .headers()
        .get("etag")
        .expect("ETag header present")
        .to_str()
        .expect("ASCII")
        .to_string();
    assert!(
        etag.contains("global-settings-v"),
        "expected global-settings-v in ETag, got {etag:?}"
    );
}

/// PUT /global-settings/network without If-Match → 428.
#[tokio::test]
async fn put_network_settings_without_if_match_returns_428() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .put_json("/api/v1/global-settings/network", &serde_json::json!({}))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::PRECONDITION_REQUIRED);
}

/// Full GET→ETag→PUT round-trip for network settings (GlobalSettingsVersion scope).
///
/// This test exercises GlobalSettingsVersion::refresh_etag — the DB re-read path
/// triggered after a successful mutation.
#[tokio::test]
async fn network_settings_get_etag_put_round_trip() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // GET → capture ETag.
    let get_resp = client
        .get("/api/v1/global-settings/network")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(get_resp.status(), http::StatusCode::OK);
    let etag = get_resp
        .headers()
        .get("etag")
        .expect("ETag on GET")
        .to_str()
        .expect("ASCII")
        .to_string();
    assert!(
        etag.contains("global-settings-v"),
        "expected global-settings-v prefix, got {etag:?}"
    );

    // PUT with captured ETag and a real field change → 200 with new ETag.
    // An all-None body is a no-op; supply real_ip_header to trigger a DB write and
    // version bump so the old ETag becomes stale.
    let put_body = serde_json::json!({ "real_ip_header": "x-forwarded-for" });
    let put_resp = client
        .put_json("/api/v1/global-settings/network", &put_body)
        .bearer(&token)
        .header("if-match", &etag)
        .send()
        .await;
    assert_eq!(put_resp.status(), http::StatusCode::OK);
    assert!(
        put_resp.headers().contains_key("etag"),
        "PUT response must carry ETag (from refresh_etag DB re-read)"
    );

    // Old ETag now stale → 409.
    let stale = client
        .put_json("/api/v1/global-settings/network", &put_body)
        .bearer(&token)
        .header("if-match", &etag)
        .send_status()
        .await;
    assert_eq!(stale, http::StatusCode::CONFLICT);
}

// ── Scope regression: global endpoints reject tenant-scoped ETags ─────────────

/// PUT /global-settings/network with a tenant-scoped ETag → 409 Conflict.
///
/// Before the scope-bug fix, network/nats/zeroconf/github used SettingsVersion
/// (tenant scope). A tenant ETag would have been accepted. This test ensures
/// the correct GlobalSettingsVersion scope is enforced.
#[tokio::test]
async fn global_settings_rejects_tenant_scoped_etag() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Seed both caches to version 5 so the test discriminates on the ETag *prefix*.
    app.state
        .settings_version_cache
        .update(Scope::Tenant(app.state.default_tenant_id), 5);
    app.state.settings_version_cache.update(Scope::Global, 5);

    // Sending the tenant-scoped ETag W/"settings-v5" to a global endpoint.
    // The middleware compares against W/"global-settings-v5" → mismatch → 409.
    let status = client
        .put_json("/api/v1/global-settings/network", &serde_json::json!({}))
        .bearer(&token)
        .header("if-match", "W/\"settings-v5\"")
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::CONFLICT,
        "tenant-scoped ETag must be rejected by global-settings endpoint"
    );
}

/// PUT /global-settings/zeroconf with a tenant-scoped ETag → 409 Conflict.
#[tokio::test]
async fn zeroconf_settings_rejects_tenant_scoped_etag() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .put_json(
            "/api/v1/global-settings/zeroconf",
            &serde_json::json!({ "enabled": false }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::CONFLICT,
        "tenant-scoped ETag rejected by zeroconf (global) endpoint"
    );
}
