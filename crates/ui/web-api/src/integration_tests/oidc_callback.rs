//! Integration tests for the `GET /api/v1/auth/oidc/callback` error paths.
//!
//! These tests cover the five early-exit paths that can be exercised without a
//! real OIDC provider (no token exchange required).  Each test verifies that the
//! handler issues the correct `302` redirect with the expected `error` query
//! parameter.
//!
//! Token-exchange paths (requiring a real JWT / PKCE flow) are deferred to a
//! future task.

#[cfg(feature = "oidc")]
use crate::test_harness::TestApp;

/// Helper: extract the `Location` header from a response and return it as a
/// `String`.  Panics if the header is absent.
#[cfg(feature = "oidc")]
fn location(resp: &http::Response<axum::body::Body>) -> String {
    resp.headers()
        .get(http::header::LOCATION)
        .unwrap_or_else(|| panic!("expected Location header, got none"))
        .to_str()
        .expect("Location header is not valid UTF-8")
        .to_string()
}

// ── Test: provider sends `error` param ─────────────────────────────────────

/// When the OIDC provider returns an `error` query parameter (e.g. the user
/// denied consent), the handler must redirect to `/login?error=oidc_denied`.
#[cfg(feature = "oidc")]
#[tokio::test]
async fn oidc_callback_provider_error_redirects_to_oidc_denied() {
    let app = TestApp::new().await;
    let client = app.client();

    let resp = client
        .get("/api/v1/auth/oidc/callback?error=access_denied")
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(location(&resp), "/login?error=oidc_denied");
}

// ── Test: missing `code` parameter ─────────────────────────────────────────

/// When neither `code` nor `state` are present the handler redirects to
/// `/login?error=oidc_missing_params`.
#[cfg(feature = "oidc")]
#[tokio::test]
async fn oidc_callback_missing_code_redirects_to_oidc_missing_params() {
    let app = TestApp::new().await;
    let client = app.client();

    let resp = client
        .get("/api/v1/auth/oidc/callback?state=some_state")
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(location(&resp), "/login?error=oidc_missing_params");
}

// ── Test: missing `state` parameter ────────────────────────────────────────

/// When `code` is present but `state` is absent the handler redirects to
/// `/login?error=oidc_missing_params`.
#[cfg(feature = "oidc")]
#[tokio::test]
async fn oidc_callback_missing_state_redirects_to_oidc_missing_params() {
    let app = TestApp::new().await;
    let client = app.client();

    let resp = client
        .get("/api/v1/auth/oidc/callback?code=some_code")
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(location(&resp), "/login?error=oidc_missing_params");
}

// ── Test: expired / unknown CSRF state ─────────────────────────────────────

/// When both `code` and `state` are present but the `state` token does not
/// exist in the `pending_oidc_flows` table (expired or never issued), the
/// handler redirects to `/login?error=oidc_state_expired`.
#[cfg(feature = "oidc")]
#[tokio::test]
async fn oidc_callback_unknown_state_redirects_to_oidc_state_expired() {
    let app = TestApp::new().await;
    let client = app.client();

    // Use a random state that was never inserted into the flow store.
    let resp = client
        .get("/api/v1/auth/oidc/callback?code=any_code&state=totally_unknown_csrf_state")
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(location(&resp), "/login?error=oidc_state_expired");
}

// ── Test: provider deleted after flow was started ──────────────────────────

/// When a valid CSRF state entry exists in the DB but the referenced OIDC
/// provider has been deleted (or deactivated), the handler redirects to
/// `/login?error=oidc_provider_gone`.
#[cfg(feature = "oidc")]
#[tokio::test]
async fn oidc_callback_provider_gone_redirects_to_oidc_provider_gone() {
    use openidconnect::{Nonce, PkceCodeVerifier};

    let app = TestApp::new().await;
    let client = app.client();

    // Insert a pending flow that references a non-existent provider UUID.
    // Use a plaintext EncryptedString (plaintext mode is enabled in TestApp).
    let csrf_state = "test_csrf_state_gone_provider";
    let nonexistent_provider_id = uuid::Uuid::now_v7();

    // Use the OidcFlowStore to insert a real pending flow entry.
    // PkceCodeVerifier and Nonce are constructed from raw strings — the token
    // exchange never happens so their exact values do not matter.
    app.state
        .oidc_flow_store
        .insert(
            csrf_state.to_string(),
            nonexistent_provider_id,
            &PkceCodeVerifier::new("test_pkce_verifier".to_string()),
            &Nonce::new("test_nonce".to_string()),
        )
        .await
        .expect("insert pending OIDC flow");

    // The flow exists but the provider does not — expect oidc_provider_gone.
    // The Host header is required for `base_url_from_headers` to succeed.
    let resp = client
        .get(&format!(
            "/api/v1/auth/oidc/callback?code=any_code&state={csrf_state}"
        ))
        .header("Host", "localhost")
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    let loc = location(&resp);
    assert_eq!(
        loc, "/login?error=oidc_provider_gone",
        "expected oidc_provider_gone redirect, got: {loc}"
    );

    // Verify the flow was consumed (taken) — a second request with the same
    // state must hit oidc_state_expired since take() deletes it atomically.
    let resp2 = client
        .get(&format!(
            "/api/v1/auth/oidc/callback?code=any_code&state={csrf_state}"
        ))
        .header("Host", "localhost")
        .send()
        .await;

    assert_eq!(resp2.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(location(&resp2), "/login?error=oidc_state_expired");
}

// ── Test: no params at all ─────────────────────────────────────────────────

/// A completely empty callback (no query params) also redirects to
/// `oidc_missing_params` since both `code` and `state` are absent.
#[cfg(feature = "oidc")]
#[tokio::test]
async fn oidc_callback_no_params_redirects_to_oidc_missing_params() {
    let app = TestApp::new().await;
    let client = app.client();

    let resp = client.get("/api/v1/auth/oidc/callback").send().await;

    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(location(&resp), "/login?error=oidc_missing_params");
}
