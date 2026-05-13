//! End-to-end OAuth 2.1 + MCP Resource Server (RS) round-trip test.
//!
//! ## Spec reference: §19
//!
//! This test drives the full MCP OAuth flow against a live controller running
//! inside the `uptrakit-test:latest` Docker image:
//!
//! ```text
//! 1. Start controller container.
//! 2. Register a test user and obtain an upk_ API token via /api/v1/auth/register.
//! 3. Enable the MCP OAuth server via PUT /api/v1/settings/oauth
//!    (sets oauth.mcp_enabled = true, sets canonical_host).
//! 4. Register an OAuth client via POST /oauth/register (RFC 7591 DCR).
//! 5. Drive GET /oauth/authorize with PKCE code_challenge.
//!    - This step normally redirects to a consent UI.
//!    - INFRASTRUCTURE GAP: no headless-browser / server-side auto-approve helper
//!      exists yet; this step is a todo!() placeholder.
//! 6. Simulate consent approval via POST /oauth/consent/{request_id}/approve
//!    using the admin session cookie.
//!    - INFRASTRUCTURE GAP: ApiClient does not yet expose cookie-jar support
//!      needed to reuse the login session across redirects.
//! 7. Exchange the authorization code for an access token via POST /oauth/token.
//! 8. Call the MCP `get_current_user` tool at POST /mcp with
//!    `Authorization: Bearer <access_token>`.
//! 9. Assert: HTTP 200, tool result contains `auth_method = "OAuth"`.
//!
//! Steps 5–6 require either:
//! (a) A server-side test-only "auto-approve" endpoint (bypass consent for
//!     test clients), or
//! (b) A cookie-jar-aware HTTP client that can follow the redirect chain,
//!     submit the consent form, and capture the redirect back.
//!
//! Until one of those is implemented, steps 5–6 carry `todo!()` markers.
//! The rest of the test skeleton compiles and documents the full intended flow.

#![expect(
    clippy::todo,
    dead_code,
    reason = "infrastructure gaps documented in module-level doc; todo!() marks unimplemented steps; \
              shared test helpers include members not yet exercised by this stub test"
)]

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
/// authenticated MCP `get_current_user` tool call verified to have used
/// the `OAuth` auth method.
///
/// Steps 5–6 (consent UI redirect + form submission) require browser-automation
/// or a server-side test-bypass hook that does not yet exist. Those steps are
/// marked with `todo!()` — see module-level doc for the full gap analysis.
///
/// **Requires:** `uptrakit-test:latest` Docker image.
/// **Run:** `cargo test -p uptrakit-integration-tests -- --ignored`
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). \
            Steps 5-6 also require consent-bypass infrastructure not yet implemented. \
            Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn oauth_end_to_end_mcp_rs_round_trip() {
    // -----------------------------------------------------------------------
    // Step 1 — Start the controller container.
    // -----------------------------------------------------------------------
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;
    let port = controller.host_port();
    let _base_url = format!("https://127.0.0.1:{port}");

    // -----------------------------------------------------------------------
    // Step 2 — Register a test user; obtain an upk_ API token.
    // -----------------------------------------------------------------------
    let mut api_client = ApiClient::new(port);
    api_client.wait_for_ready(Duration::from_secs(30)).await;
    api_client
        .register_and_login_with_token(controller.registration_token())
        .await;

    // A raw reqwest client will be needed for OAuth endpoints once Step 3 is
    // implemented (see the step 3 gap comment below). It will be built as:
    //
    //   let http = Client::builder()
    //       .danger_accept_invalid_certs(true)
    //       .connect_timeout(Duration::from_secs(10))
    //       .timeout(Duration::from_secs(60))
    //       .redirect(reqwest::redirect::Policy::none())
    //       .build()
    //       .expect("build reqwest client");

    // -----------------------------------------------------------------------
    // Step 3 — Enable the MCP OAuth server.
    //
    // INFRASTRUCTURE GAP: `UptrakitClient` and `ApiClient` do not yet expose an
    // `update_oauth_settings` method. The raw HTTP call below documents the
    // intended wire format; the actual settings PUT path and request body must
    // match the server's `PUT /api/v1/settings/oauth` handler once it is
    // implemented (see the oauth.boot module for the relevant setting keys:
    // `oauth.mcp_enabled` and `oauth.canonical_host`).
    //
    //   PUT /api/v1/settings/oauth
    //   Body: { "mcp_enabled": true, "canonical_host": "127.0.0.1:<port>" }
    //
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Step 4 — Register an OAuth client via DCR (POST /oauth/register).
    //
    // INFRASTRUCTURE GAP: Step 3 must succeed first (OAuth must be enabled).
    // Until Step 3 is implemented the DCR call will return 404.
    //
    //   let resp = _http
    //       .post(format!("{_base_url}/oauth/register"))
    //       .json(&serde_json::json!({
    //           "client_name": "e2e-test-client",
    //           "redirect_uris": ["https://localhost/callback"],
    //           "grant_types": ["authorization_code"],
    //           "response_types": ["code"],
    //           "token_endpoint_auth_method": "none",
    //           "scope": "mcp:read"
    //       }))
    //       .send()
    //       .await
    //       .expect("POST /oauth/register");
    //   assert_eq!(resp.status().as_u16(), 201, ...);
    //   let _dcr_response: Value = resp.json().await.expect("parse DCR response body");
    //
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Step 5 — GET /oauth/authorize with PKCE.
    //
    // INFRASTRUCTURE GAP: this endpoint redirects to /login (unauthenticated)
    // and then to /oauth/consent/<id>. Automating the full redirect chain
    // requires either:
    //   (a) A server-side test-bypass endpoint (e.g. POST /oauth/test/auto-approve),
    //   (b) A cookie-jar-aware HTTP client that can follow the login + consent
    //       redirect chain and extract the `code` from the final redirect URI.
    // Neither is available yet.
    // -----------------------------------------------------------------------
    let (_code_verifier, _code_challenge) = generate_pkce_pair();

    todo!(
        "Step 5: GET /oauth/authorize + consent redirect chain. \
         Requires server-side auto-approve hook or cookie-jar HTTP helper. \
         See module-level doc for the full gap analysis."
    );

    // Steps 6–9 are unreachable until Step 5 is implemented; they are
    // documented here for reference.
    //
    // -----------------------------------------------------------------------
    // Step 6 — Approve consent via POST /oauth/consent/{request_id}/approve.
    //
    //   let approve_resp = http
    //       .post(format!("{base_url}/oauth/consent/{request_id}/approve"))
    //       .header("Authorization", format!("Bearer {api_token}"))
    //       .send()
    //       .await
    //       .expect("POST /oauth/consent/…/approve");
    //   let redirect_location = approve_resp
    //       .headers()
    //       .get("location")
    //       .expect("consent approve must return a redirect");
    //   let code = extract_code_from_redirect(redirect_location.to_str().unwrap());
    //
    // -----------------------------------------------------------------------
    // Step 7 — Exchange the authorization code for an access token.
    //
    //   let token_resp = http
    //       .post(format!("{base_url}/oauth/token"))
    //       .form(&[
    //           ("grant_type", "authorization_code"),
    //           ("code", &code),
    //           ("redirect_uri", "https://localhost/callback"),
    //           ("client_id", &client_id),
    //           ("code_verifier", &code_verifier),
    //           ("resource", &format!("{base_url}/mcp")),
    //       ])
    //       .send()
    //       .await
    //       .expect("POST /oauth/token");
    //   assert_eq!(token_resp.status().as_u16(), 200);
    //   let token_body: Value = token_resp.json().await.expect("parse token response");
    //   let access_token = token_body["access_token"]
    //       .as_str()
    //       .expect("access_token in response")
    //       .to_string();
    //
    // -----------------------------------------------------------------------
    // Step 8 — Call MCP `get_current_user` with the OAuth access token.
    //
    //   let mcp_resp = http
    //       .post(format!("{base_url}/mcp"))
    //       .header("Authorization", format!("Bearer {access_token}"))
    //       .header("Content-Type", "application/json")
    //       .json(&serde_json::json!({
    //           "jsonrpc": "2.0",
    //           "id": 1,
    //           "method": "tools/call",
    //           "params": {
    //               "name": "get_current_user",
    //               "arguments": {}
    //           }
    //       }))
    //       .send()
    //       .await
    //       .expect("POST /mcp (get_current_user)");
    //   assert_eq!(mcp_resp.status().as_u16(), 200,
    //       "MCP tool call must succeed with OAuth token");
    //
    // -----------------------------------------------------------------------
    // Step 9 — Verify McpAuthMethod::OAuth evidence in the response.
    //
    //   let mcp_body: Value = mcp_resp.json().await.expect("parse MCP response");
    //   // The get_current_user tool currently does not expose auth_method in its
    //   // JSON output; asserting HTTP 200 is the observable success signal for
    //   // the RS layer. A future `auth_method` field (spec §19) would allow:
    //   //
    //   //   let result = &mcp_body["result"]["content"][0]["text"];
    //   //   let user_info: Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
    //   //   assert_eq!(user_info["auth_method"], "OAuth",
    //   //       "MCP RS must record OAuth auth method");
}
