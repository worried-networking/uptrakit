use http::StatusCode;
use uptrakit_web_api_types::oauth::{
    DeviceAuthDenyResponse, DeviceAuthLookupResponse, DeviceAuthorizationResponse,
    OAuthAuthorizationServerMetadata, OAuthErrorCode, OAuthErrorResponse, OAuthTokenResponse,
};

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{register_and_get_token, register_user};

const CLIENT_ID: &str = "uptrakit-cli";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

// ── oauth/device_authorization ───────────────────────────────────────────────

#[tokio::test]
async fn success_response_shape_matches_rfc() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(!flow.device_code.is_empty());
    assert_eq!(flow.user_code.len(), 9);
    assert!(!flow.verification_uri.is_empty());
    assert!(!flow.verification_uri_complete.is_empty());
    assert_eq!(flow.expires_in, 600);
    assert_eq!(flow.interval, 5);
}

#[tokio::test]
async fn client_id_mismatch_returns_invalid_client() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, err): (_, OAuthErrorResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            "client_id=wrong-client",
        )
        .send_json()
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::InvalidClient);
}

#[tokio::test]
async fn client_name_extension_field_accepted() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, _flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}&client_name=my-laptop-2026"),
        )
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn verification_uri_complete_contains_user_code() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        flow.verification_uri_complete.contains(&flow.user_code),
        "verification_uri_complete must contain user_code"
    );
}

// ── oauth/token ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn unsupported_grant_type_response() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, err): (_, OAuthErrorResponse) = client
        .post_form(
            "/api/v1/oauth/token",
            "grant_type=authorization_code&code=abc",
        )
        .send_json()
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::UnsupportedGrantType);
}

#[tokio::test]
async fn invalid_grant_when_device_code_unknown() {
    let app = TestApp::new().await;
    let client = app.client();

    let form = format!(
        "grant_type={}&device_code=notarealcode&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
    );

    let (status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::ExpiredToken);
}

#[tokio::test]
async fn invalid_request_when_missing_fields() {
    let app = TestApp::new().await;
    let client = app.client();

    let form = format!(
        "grant_type={}&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
    );

    let (status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::InvalidRequest);
}

#[tokio::test]
async fn success_returns_bearer_token() {
    let app = TestApp::new().await;
    let client = app.client();

    let (reg_status, auth) =
        register_user(&client, "device-owner@test.local", "TestPassword1!").await;
    assert_eq!(reg_status, StatusCode::CREATED);
    let user_id = auth.user.id;

    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    let normalized = flow.user_code.replace('-', "").to_uppercase();
    app.state
        .auth
        .device_flow_store
        .approve(&normalized, user_id)
        .await
        .expect("approve");

    let form = format!(
        "grant_type={}&device_code={}&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
        urlencoded(&flow.device_code),
    );

    let (token_status, token): (_, OAuthTokenResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;

    assert_eq!(token_status, StatusCode::OK);
    assert!(!token.access_token.is_empty());
    assert_eq!(token.token_type, "Bearer");
}

#[tokio::test]
async fn slow_down_400_with_interval() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    let form = format!(
        "grant_type={}&device_code={}&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
        urlencoded(&flow.device_code),
    );

    let (first_status, _): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;
    assert_eq!(first_status, StatusCode::BAD_REQUEST);

    let (second_status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;

    assert_eq!(second_status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::SlowDown);
    assert!(err.interval.is_some(), "slow_down must include interval");
}

#[tokio::test]
async fn authorization_pending_400() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    let form = format!(
        "grant_type={}&device_code={}&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
        urlencoded(&flow.device_code),
    );

    let (poll_status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;

    assert_eq!(poll_status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::AuthorizationPending);
}

#[tokio::test]
async fn access_denied_400() {
    let app = TestApp::new().await;
    let client = app.client();

    let (reg_status, auth) = register_user(&client, "denier@test.local", "TestPassword1!").await;
    assert_eq!(reg_status, StatusCode::CREATED);
    let user_id = auth.user.id;

    let (status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    let normalized = flow.user_code.replace('-', "").to_uppercase();
    app.state
        .auth
        .device_flow_store
        .deny(&normalized, user_id)
        .await
        .expect("deny");

    let form = format!(
        "grant_type={}&device_code={}&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
        urlencoded(&flow.device_code),
    );

    let (poll_status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;

    assert_eq!(poll_status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::AccessDenied);
}

#[tokio::test]
async fn malformed_device_code_returns_invalid_grant() {
    let app = TestApp::new().await;
    let client = app.client();

    let form = format!(
        "grant_type={}&device_code=zzznotahexsha256zzz&client_id={CLIENT_ID}",
        urlencoded(DEVICE_CODE_GRANT),
    );

    let (status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &form)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::ExpiredToken);
}

// ── /.well-known/oauth-authorization-server ──────────────────────────────────

#[tokio::test]
async fn discovery_doc_lists_device_grant_endpoints() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, metadata): (_, OAuthAuthorizationServerMetadata) = client
        .get("/.well-known/oauth-authorization-server")
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        metadata
            .grant_types_supported
            .iter()
            .any(|g| g == DEVICE_CODE_GRANT),
        "grant_types_supported must contain device_code URN"
    );
    assert!(!metadata.device_authorization_endpoint.is_empty());
    assert!(!metadata.token_endpoint.is_empty());
}

// ── device_auth deny ─────────────────────────────────────────────────────────

#[tokio::test]
async fn deny_requires_auth() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .post_json(
            "/api/v1/auth/device/deny",
            &serde_json::json!({ "user_code": "ABCD-EFGH" }),
        )
        .send_status()
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn deny_returns_ok() {
    let app = TestApp::new().await;
    let client = app.client();

    let token = register_and_get_token(&client).await;

    let (flow_status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(flow_status, StatusCode::OK);

    let (status, resp): (_, DeviceAuthDenyResponse) = client
        .post_json(
            "/api/v1/auth/device/deny",
            &serde_json::json!({ "user_code": flow.user_code }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(!resp.message.is_empty());
}

#[tokio::test]
async fn deny_unknown_user_code_returns_not_found() {
    let app = TestApp::new().await;
    let client = app.client();

    let token = register_and_get_token(&client).await;

    let status = client
        .post_json(
            "/api/v1/auth/device/deny",
            &serde_json::json!({ "user_code": "UNKN-OWNX" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── device_auth lookup ───────────────────────────────────────────────────────

#[tokio::test]
async fn lookup_requires_auth() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .get("/api/v1/auth/device/lookup?user_code=ABCD-EFGH")
        .send_status()
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn lookup_returns_flow_info() {
    let app = TestApp::new().await;
    let client = app.client();

    let token = register_and_get_token(&client).await;

    let (flow_status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}&client_name=my-cli-test"),
        )
        .send_json()
        .await;
    assert_eq!(flow_status, StatusCode::OK);

    let (status, resp): (_, DeviceAuthLookupResponse) = client
        .get(&format!(
            "/api/v1/auth/device/lookup?user_code={}",
            flow.user_code
        ))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp.client_name.as_deref(), Some("my-cli-test"));
}

#[tokio::test]
async fn lookup_unknown_user_code_returns_not_found() {
    let app = TestApp::new().await;
    let client = app.client();

    let token = register_and_get_token(&client).await;

    let status = client
        .get("/api/v1/auth/device/lookup?user_code=UNKN-OWNX")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Percent-encode a string for inclusion in an `application/x-www-form-urlencoded` body.
fn urlencoded(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                vec![c]
            }
            ':' => vec!['%', '3', 'A'],
            '/' => vec!['%', '2', 'F'],
            _ => {
                let encoded = format!("%{:02X}", c as u32);
                encoded.chars().collect()
            }
        })
        .collect()
}
