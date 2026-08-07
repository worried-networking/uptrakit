#![expect(
    clippy::expect_used,
    reason = "test helper functions are not covered by allow-expect-in-tests"
)]

use http::StatusCode;
use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
use uptrakit_shared_types::access::{ActionPattern, Selector};
use uptrakit_web_api_types::oauth::{
    AuthorizationServerMetadata, DeviceAuthDenyResponse, DeviceAuthLookupResponse,
    DeviceAuthorizationResponse, OAuthErrorCode, OAuthErrorResponse, OAuthTokenResponse,
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
async fn discovery_doc_lists_mcp_grant_endpoints() {
    let client = oauth_enabled_client().await;

    let (status, metadata): (_, AuthorizationServerMetadata) = client
        .get("/.well-known/oauth-authorization-server")
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        metadata
            .grant_types_supported
            .iter()
            .any(|g| g == "authorization_code"),
        "grant_types_supported must contain authorization_code"
    );
    assert!(!metadata.token_endpoint.is_empty());
    assert!(!metadata.authorization_endpoint.is_empty());
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
            &serde_json::json!({ "user_code": "BCDF-GHJK" }),
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
        .get("/api/v1/auth/device/lookup?user_code=BCDF-GHJK")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── additional coverage (plan Task 13 requirements) ─────────────────────────

/// Token endpoint must return `invalid_client` when `client_id` does not match
/// the hardcoded constant, even for a real pending flow.
#[tokio::test]
async fn invalid_client_when_client_id_mismatches() {
    let app = TestApp::new().await;
    let client = app.client();

    let (flow_status, flow): (_, DeviceAuthorizationResponse) = client
        .post_form(
            "/api/v1/oauth/device_authorization",
            &format!("client_id={CLIENT_ID}"),
        )
        .send_json()
        .await;
    assert_eq!(flow_status, StatusCode::OK);

    let token_body = format!(
        "grant_type={}&device_code={}&client_id=wrong-client",
        urlencoded(DEVICE_CODE_GRANT),
        flow.device_code
    );
    let (status, err): (_, OAuthErrorResponse) = client
        .post_form("/api/v1/oauth/token", &token_body)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err.error, OAuthErrorCode::InvalidClient);
}

/// `POST /auth/device/deny` authorizes on the single mapped `services:read`
/// grant alone (positive proof a lone action-grant suffices — no other
/// action is consulted). Tokens carry no authorization claim at all
/// (`CanReadServices` only consults `AccessEngine` grants), so the ephemeral
/// user is granted `services:read` directly via `insert_grant` and the
/// request proceeds past authorization to the handler's business logic,
/// which then 404s on the unknown `user_code` (mirrors
/// `deny_unknown_user_code_returns_not_found`).
#[tokio::test]
async fn deny_requires_permission() {
    let app = TestApp::new().await;
    let client = app.client();

    let user_id = uuid::Uuid::now_v7();
    let viewer_token = app
        .jwt
        .create_access_token(user_id, "password", None, None)
        .expect("mint viewer token");

    let patterns = vec!["services:read".parse::<ActionPattern>().expect("pattern")];
    insert_grant(
        &app.db,
        NewGrant {
            subject: GrantSubject::User(user_id),
            tenant_id: Some(app.tenant_id),
            patterns: &patterns,
            selector: Selector::All,
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("insert grant");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);

    let status = client
        .post_json(
            "/api/v1/auth/device/deny",
            &serde_json::json!({ "user_code": "BCDF-GHJK" }),
        )
        .bearer(&viewer_token)
        .send_status()
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// `GET /.well-known/oauth-authorization-server` must return 200 without any
/// authentication headers when OAuth is enabled.
#[tokio::test]
async fn discovery_doc_no_auth_required() {
    let client = oauth_enabled_client().await;

    // Deliberately omit the bearer token.
    let status = client
        .get("/.well-known/oauth-authorization-server")
        .send_status()
        .await;

    assert_eq!(status, StatusCode::OK);
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a [`TestClient`] backed by a router with MCP OAuth enabled.
///
/// Used for tests that exercise the `/.well-known/oauth-authorization-server`
/// endpoint, which returns 404 when OAuth is disabled (the default in
/// `TestApp::new()`).
async fn oauth_enabled_client() -> crate::test_harness::http_client::TestClient {
    use std::sync::Arc;

    use crate::oauth::OAuthState;
    use crate::oauth::canonical_url::CanonicalUrlConfig;
    use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
    use crate::router::build_router;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db, tenant_id).await;

    let canonical = CanonicalUrlConfig::new("controller.example.com".to_string(), vec![])
        .expect("test canonical url");
    let oauth = OAuthState {
        enabled: true,
        canonical,
        signer: Arc::new(McpOAuthJwtSigner::new(b"test-secret-not-used")),
        verifier: Arc::new(McpOAuthJwtVerifier::new(
            b"test-secret-not-used",
            "https://controller.example.com".into(),
            vec![],
        )),
        clock: Arc::new(time::OffsetDateTime::now_utc),
        instance_id: uuid::Uuid::nil(),
        dcr_enabled: false,
        cimd_enabled: false,
    };

    let mut patched = (*state).clone();
    patched.oauth = oauth;
    let router = build_router(Arc::new(patched));
    crate::test_harness::http_client::TestClient::new(router)
}

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
