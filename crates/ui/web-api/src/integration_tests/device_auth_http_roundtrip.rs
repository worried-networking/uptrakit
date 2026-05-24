use http::StatusCode;
use uptrakit_web_api_types::auth::UserResponse;
use uptrakit_web_api_types::oauth::{
    DeviceAuthorizationResponse, OAuthErrorCode, OAuthErrorResponse, OAuthTokenResponse,
};

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_user;

const CLIENT_ID: &str = "uptrakit-cli";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Percent-encode a string for inclusion in an `application/x-www-form-urlencoded` body.
/// The `:` and `/` in the device_code grant URN must be encoded as %3A and %2F.
fn urlencoded(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            ':' => vec!['%', '3', 'A'],
            '/' => vec!['%', '2', 'F'],
            _ => {
                let encoded = format!("%{:02X}", c as u32);
                encoded.chars().collect()
            }
        })
        .collect()
}

// ── Test 1: full approve → token → API ─────────────────────────────────────

#[tokio::test]
async fn device_flow_full_http_chain_token_works_at_api() {
    let app = TestApp::new().await;
    let client = app.client();

    // Start device flow.
    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);
    let user_code = flow.user_code.clone();
    let device_code = flow.device_code.clone();

    // Register user via HTTP, capture JWT and expected email.
    let (reg_status, auth) =
        register_user(&client, "approve-device@example.com", "TestPassword1!").await;
    assert_eq!(reg_status, StatusCode::CREATED);
    let user_jwt = auth.access_token.expose_secret().to_string();
    let expected_email = auth.user.email.clone();

    // Approve via POST /api/v1/auth/device/approve (no internal state injection).
    let approve_status = client
        .post_json(
            "/api/v1/auth/device/approve",
            &serde_json::json!({ "user_code": user_code }),
        )
        .bearer(&user_jwt)
        .send_status()
        .await;
    assert_eq!(approve_status, StatusCode::OK);

    // Poll for token.
    let form = format!(
        "grant_type={}&device_code={}&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
        urlencoded(&device_code),
    );
    let (token_status, token): (_, OAuthTokenResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;
    assert_eq!(token_status, StatusCode::OK);
    assert_eq!(token.token_type, "Bearer");
    assert!(
        token.access_token.starts_with("upk_"),
        "access_token must start with 'upk_', got: {}",
        token.access_token,
    );

    // Use the device token at a standard authenticated endpoint.
    let (me_status, me): (_, UserResponse) = client
        .get("/api/v1/auth/me")
        .bearer(&token.access_token)
        .send_json()
        .await;
    assert_eq!(me_status, StatusCode::OK);
    assert_eq!(me.email, expected_email);
}

// ── Test 2: deny → poll → access_denied ────────────────────────────────────

#[tokio::test]
async fn device_flow_deny_via_http_returns_access_denied_on_poll() {
    let app = TestApp::new().await;
    let client = app.client();

    // Start device flow.
    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    // Register user via HTTP.
    let (reg_status, auth) =
        register_user(&client, "deny-device@example.com", "TestPassword1!").await;
    assert_eq!(reg_status, StatusCode::CREATED);
    let user_jwt = auth.access_token.expose_secret().to_string();

    // Deny via POST /api/v1/auth/device/deny.
    let deny_status = client
        .post_json(
            "/api/v1/auth/device/deny",
            &serde_json::json!({ "user_code": flow.user_code }),
        )
        .bearer(&user_jwt)
        .send_status()
        .await;
    assert_eq!(deny_status, StatusCode::OK);

    // Poll — must return access_denied per RFC 8628.
    let form = format!(
        "grant_type={}&device_code={}&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
        urlencoded(&flow.device_code),
    );
    let (token_status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;
    assert_eq!(token_status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::AccessDenied);
}
