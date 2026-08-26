//! End-to-end OAuth 2.1 + MCP Resource Server (RS) round-trip test.
//!
//! ## Spec reference: §19
//!
//! This test drives the full MCP OAuth flow against a live controller running
//! inside the `uptrakit-test:latest` Docker image:
#![expect(
    dead_code,
    reason = "shared test helpers are used by other test binaries (system, cert_rotation, \
              spiffe_identity) — dead_code is expected from this binary's perspective"
)]
//!
//! ```text
//! 1. Start controller container.
//! 2. Register a test user and obtain an upk_ API token via /api/v1/auth/register.
//! 3. Enable the MCP OAuth server via PUT /api/v1/global-settings/oauth
//!    (sets mcp_enabled = true, canonical_host, and dcr_enabled = true).
//! 4. Force reexec so OAuth boots live; wait for new generation via GET /healthz.
//! 5. Register an OAuth client via POST /oauth/register (RFC 7591 DCR).
//! 6. Drive GET /oauth/authorize with PKCE code_challenge.
//! 7. Bypass consent UI via POST /oauth/test/auto-approve/{request_id}.
//! 8. Exchange the authorization code for an access token via POST /oauth/token.
//! 9. Call the MCP `get_current_user` tool at POST /mcp with
//!    `Authorization: Bearer <access_token>`.
//! ```

use std::time::Duration;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, test_network_name};

mod helpers;

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Generate a PKCE S256 `code_verifier` and `code_challenge` pair.
///
/// Returns `(verifier, challenge)` where `challenge = BASE64URL(SHA256(verifier))`.
fn generate_pkce_pair() -> (String, String) {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    // 43-character ASCII verifier is the minimum allowed by RFC 7636 §4.1.
    let verifier = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_string();
    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
    (verifier, challenge)
}

// ---------------------------------------------------------------------------
// E2E test — OAuth + MCP RS round-trip (spec §19)
// ---------------------------------------------------------------------------

/// Full OAuth 2.1 authorization-code + PKCE round-trip followed by an
/// authenticated MCP `get_current_user` tool call.
///
/// **Requires:** `uptrakit-test:latest` Docker image.
/// **Run:** `cargo test -p uptrakit-integration-tests -- oauth_end_to_end_mcp_rs_round_trip --nocapture`
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). \
            Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn oauth_end_to_end_mcp_rs_round_trip() {
    // -----------------------------------------------------------------------
    // Step 1 — Start the controller container.
    // -----------------------------------------------------------------------
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;
    let port = controller.host_port();

    // -----------------------------------------------------------------------
    // Step 2 — Register a test user; obtain an upk_ API token.
    // -----------------------------------------------------------------------
    let mut api_client = ApiClient::new(port);
    api_client.wait_for_ready(Duration::from_secs(30)).await;
    api_client
        .register_and_login_with_token(controller.registration_token())
        .await;

    let http = crate::helpers::api_client::test_client_builder()
        .danger_accept_invalid_certs(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build reqwest client");

    // -----------------------------------------------------------------------
    // Step 3 — Enable the MCP OAuth server via an explicit mcp_enabled opt-in.
    // -----------------------------------------------------------------------
    api_client
        .update_oauth_settings(&format!("127.0.0.1:{port}"))
        .await;

    // -----------------------------------------------------------------------
    // Step 4 — Force reexec so OAuth boots live, then wait for new generation.
    // -----------------------------------------------------------------------
    let current_gen: u64 = {
        let healthz_resp = http
            .get(format!("https://127.0.0.1:{port}/healthz"))
            .send()
            .await
            .expect("GET /healthz");
        healthz_resp
            .headers()
            .get("x-reexec-generation")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };
    api_client.force_reexec().await;
    api_client
        .wait_for_generation(current_gen + 1, Duration::from_secs(30))
        .await;

    // -----------------------------------------------------------------------
    // Step 5 — Register an OAuth client via Dynamic Client Registration.
    // -----------------------------------------------------------------------
    let dcr_resp = http
        .post(format!("https://127.0.0.1:{port}/oauth/register"))
        .json(&serde_json::json!({
            "client_name": "e2e-test-client",
            "redirect_uris": ["https://localhost/callback"],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "scope": "mcp:read"
        }))
        .send()
        .await
        .expect("POST /oauth/register");
    assert_eq!(
        dcr_resp.status().as_u16(),
        201,
        "DCR must return 201, got: {}",
        dcr_resp.status()
    );
    let dcr_body: serde_json::Value = dcr_resp.json().await.expect("parse DCR response");
    let client_id = dcr_body["client_id"]
        .as_str()
        .expect("client_id in DCR response")
        .to_string();

    // -----------------------------------------------------------------------
    // Step 6 — Drive GET /oauth/authorize with PKCE.
    // -----------------------------------------------------------------------
    let (code_verifier, code_challenge) = generate_pkce_pair();

    let api_token = api_client
        .api_token
        .as_deref()
        .expect("api_token populated after register_and_login_with_token");
    let auth_resp = http
        .get(format!("https://127.0.0.1:{port}/oauth/authorize"))
        .header("Authorization", format!("Bearer {api_token}"))
        .query(&[
            ("response_type", "code"),
            ("client_id", &client_id),
            ("redirect_uri", "https://localhost/callback"),
            ("scope", "mcp:read"),
            ("state", "test-state-e2e"),
            ("code_challenge", &code_challenge),
            ("code_challenge_method", "S256"),
            ("resource", &format!("https://127.0.0.1:{port}/mcp")),
        ])
        .send()
        .await
        .expect("GET /oauth/authorize");

    let location = auth_resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("authorize must 302 to /oauth/consent/<id>")
        .to_string();
    assert!(
        location.starts_with("/oauth/consent/"),
        "expected /oauth/consent/<id>, got: {location}"
    );
    assert!(
        !location.contains("code="),
        "consent must not be pre-granted — revoke existing grants or use a fresh client"
    );

    let request_id = location.trim_start_matches("/oauth/consent/").to_string();

    // -----------------------------------------------------------------------
    // Step 7 — Bypass consent UI via test-utils endpoint.
    // -----------------------------------------------------------------------
    let code = api_client.auto_approve_consent(&request_id).await;

    // -----------------------------------------------------------------------
    // Step 8 — Exchange the authorization code for an access token.
    // -----------------------------------------------------------------------
    let token_resp = http
        .post(format!("https://127.0.0.1:{port}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", "https://localhost/callback"),
            ("client_id", client_id.as_str()),
            ("code_verifier", code_verifier.as_str()),
            ("resource", &format!("https://127.0.0.1:{port}/mcp")),
        ])
        .send()
        .await
        .expect("POST /oauth/token");
    assert_eq!(
        token_resp.status().as_u16(),
        200,
        "token exchange must return 200, got: {}",
        token_resp.status()
    );
    let token_body: serde_json::Value = token_resp.json().await.expect("parse token response");
    let access_token = token_body["access_token"]
        .as_str()
        .expect("access_token in token response")
        .to_string();

    // -----------------------------------------------------------------------
    // Step 9a — Initialize the MCP session.
    // -----------------------------------------------------------------------
    let init_resp = http
        .post(format!("https://127.0.0.1:{port}/mcp"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"mcp-e2e-test","version":"1.0"}}}"#)
        .send()
        .await
        .expect("POST /mcp initialize");
    let init_status = init_resp.status();
    let init_session_id_raw = init_resp
        .headers()
        .get("Mcp-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let init_body = init_resp.text().await.expect("read init response body");
    assert_eq!(
        init_status.as_u16(),
        200,
        "initialize must return 200, got: {}; body: {init_body}",
        init_status
    );
    let session_id =
        init_session_id_raw.expect("Mcp-Session-Id header must be present after initialize");

    // -----------------------------------------------------------------------
    // Step 9b — Acknowledge the session.
    // -----------------------------------------------------------------------
    let notif_resp = http
        .post(format!("https://127.0.0.1:{port}/mcp"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", &session_id)
        .body(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .send()
        .await
        .expect("POST /mcp notifications/initialized");
    assert_eq!(
        notif_resp.status().as_u16(),
        202,
        "notifications/initialized must return 202, got: {}",
        notif_resp.status()
    );

    // -----------------------------------------------------------------------
    // Step 9c — Call the tool; assert HTTP 200 and verify result content.
    // -----------------------------------------------------------------------
    let mcp_resp = http
        .post(format!("https://127.0.0.1:{port}/mcp"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", &session_id)
        .header("Mcp-Protocol-Version", "2025-03-26")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "get_current_user",
                "arguments": {}
            }
        }))
        .send()
        .await
        .expect("POST /mcp tools/call with OAuth bearer");
    assert_eq!(
        mcp_resp.status().as_u16(),
        200,
        "MCP tool call with OAuth token must return 200, got: {}",
        mcp_resp.status()
    );

    // Step 9d — Verify the SSE body contains user identity.
    let body_text = mcp_resp.text().await.expect("read tools/call body");
    assert!(
        body_text.contains("email"),
        "tools/call result must contain 'email' field; body: {body_text}"
    );
}
