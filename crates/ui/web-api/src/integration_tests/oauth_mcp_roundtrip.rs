//! End-to-end tests for the MCP OAuth 2.1 authorization-code + PKCE flow.
//!
//! These tests exercise the full HTTP path from `GET /oauth/authorize` through
//! `POST /oauth/consent/{id}/approve` (or `/deny`) to `POST /oauth/token`,
//! verifying that the resulting JWT access token carries correct claims.

#![expect(
    clippy::expect_used,
    reason = "test helper functions are not covered by allow-expect-in-tests"
)]

use std::sync::Arc;

use http::StatusCode;
use time::OffsetDateTime;
use uptrakit_web_api_types::oauth::TokenResponse;

use crate::oauth::OAuthState;
use crate::oauth::canonical_url::CanonicalUrlConfig;
use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
use crate::router::build_router;
use crate::test_harness::fixtures::{insert_oauth_client, register_user};
use crate::test_harness::http_client::TestClient;
use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

// RFC 7636 §4.6 test vectors.
const CODE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT_URI: &str = "https://localhost/callback";
const RESOURCE: &str = "https://controller.example.com/mcp";
// 32 bytes: safe minimum for HS256.
const TEST_SECRET: &[u8] = b"mcp-roundtrip-test-secret-32by!!";

async fn optional_auth_middleware(
    axum::extract::State(state): axum::extract::State<Arc<crate::AppState>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use crate::middleware::require_auth::authenticate_jwt;

    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);

    if let Some(token) = token
        && let Ok((user, _setup_required)) = authenticate_jwt(&state, &token).await
    {
        req.extensions_mut().insert(user);
    }

    next.run(req).await
}

async fn setup_client() -> (TestClient, sea_orm::DatabaseConnection) {
    let db = setup_migrated_db().await;
    let tenant_id = insert_default_tenant(&db).await;
    let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

    let canonical = CanonicalUrlConfig::new(
        "controller.example.com".to_string(),
        vec![RESOURCE.to_string()],
    )
    .expect("test canonical url");

    let patched = Arc::new(crate::AppState {
        oauth: OAuthState {
            enabled: true,
            canonical,
            signer: Arc::new(McpOAuthJwtSigner::new(TEST_SECRET)),
            verifier: Arc::new(McpOAuthJwtVerifier::new(
                TEST_SECRET,
                "https://controller.example.com".to_string(),
                vec![RESOURCE.to_string()],
            )),
            clock: Arc::new(OffsetDateTime::now_utc),
            instance_id: uuid::Uuid::nil(),
            dcr_enabled: false,
            cimd_enabled: false,
        },
        ..(*state).clone()
    });

    let router = build_router(Arc::clone(&patched)).layer(axum::middleware::from_fn_with_state(
        Arc::clone(&patched),
        optional_auth_middleware,
    ));

    (TestClient::new(router), db)
}

/// Percent-encode a string for use in a query parameter value.
///
/// Encodes everything except unreserved URI characters (RFC 3986 §2.3).
fn urlencoded(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            ':' => vec!['%', '3', 'A'],
            '/' => vec!['%', '2', 'F'],
            _ => format!("%{:02X}", c as u32).chars().collect::<Vec<_>>(),
        })
        .collect()
}

#[tokio::test]
async fn mcp_oauth_auth_code_pkce_roundtrip_token_claims_valid() {
    let (client, db) = setup_client().await;
    let oauth_client_id = insert_oauth_client(&db, REDIRECT_URI, true).await;

    let (reg_status, auth) =
        register_user(&client, "mcp-roundtrip@example.com", "TestPassword1!").await;
    assert_eq!(reg_status, StatusCode::CREATED);
    let user_jwt = auth.access_token.expose_secret().to_string();
    let user_id = auth.user.id;

    // --- Step 1: GET /oauth/authorize → 302 to consent page ---
    let authorize_url = format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope=mcp%3Aread&state=test-state-123&code_challenge={}&code_challenge_method=S256&resource={}",
        urlencoded(&oauth_client_id),
        urlencoded(REDIRECT_URI),
        CODE_CHALLENGE,
        urlencoded(RESOURCE),
    );
    let authorize_resp = client.get(&authorize_url).bearer(&user_jwt).send().await;
    assert_eq!(authorize_resp.status(), StatusCode::FOUND);
    let location = authorize_resp
        .headers()
        .get("location")
        .expect("Location header missing")
        .to_str()
        .expect("Location is not ASCII");
    let request_id = location
        .strip_prefix("/oauth/consent/")
        .expect("Location must start with /oauth/consent/")
        .split('?')
        .next()
        .expect("split always yields at least one element");

    // --- Step 2: POST /oauth/consent/{id}/approve → redirect_to with code ---
    let (approve_status, approve_body): (_, serde_json::Value) = client
        .post_json(
            &format!("/oauth/consent/{request_id}/approve"),
            &serde_json::json!({}),
        )
        .bearer(&user_jwt)
        .send_json()
        .await;
    assert_eq!(approve_status, StatusCode::OK);
    let redirect_to = approve_body["redirect_to"]
        .as_str()
        .expect("redirect_to must be a string");
    assert!(
        redirect_to.contains("code="),
        "redirect_to must contain code=, got: {redirect_to}"
    );

    let auth_code = redirect_to
        .split('?')
        .nth(1)
        .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("code=")))
        .expect("code query param in redirect_to");

    // --- Step 3: POST /oauth/token → access token ---
    // `auth_code` is already percent-encoded (extracted verbatim from the redirect_to
    // query string which the server built with NON_ALPHANUMERIC percent-encoding).
    // Pass it as-is — Axum's Form extractor will decode it on the server side.
    let token_form = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}&resource={}",
        auth_code,
        urlencoded(REDIRECT_URI),
        urlencoded(&oauth_client_id),
        CODE_VERIFIER,
        urlencoded(RESOURCE),
    );
    let (token_status, token): (_, TokenResponse) = client
        .post_form("/oauth/token", &token_form)
        .send_json()
        .await;
    assert_eq!(token_status, StatusCode::OK);
    assert_eq!(token.token_type, "Bearer");

    // --- Step 4: Verify JWT claims ---
    let verifier = McpOAuthJwtVerifier::new(
        TEST_SECRET,
        "https://controller.example.com".to_string(),
        vec![RESOURCE.to_string()],
    );
    let claims = verifier
        .verify(&token.access_token)
        .expect("JWT must verify with matching secret and issuer");
    assert_eq!(claims.iss, "https://controller.example.com");
    assert_eq!(claims.aud, RESOURCE);
    assert_eq!(claims.sub, user_id.to_string());
    assert_eq!(claims.scope, "mcp:read");
    assert!(
        claims.exp > OffsetDateTime::now_utc().unix_timestamp(),
        "token must not be expired",
    );
}

#[tokio::test]
async fn mcp_oauth_deny_consent_yields_access_denied_redirect() {
    let (client, db) = setup_client().await;
    let oauth_client_id = insert_oauth_client(&db, REDIRECT_URI, true).await;

    let (reg_status, auth) = register_user(&client, "mcp-deny@example.com", "TestPassword1!").await;
    assert_eq!(reg_status, StatusCode::CREATED);
    let user_jwt = auth.access_token.expose_secret().to_string();

    // --- Step 1: GET /oauth/authorize → 302 to consent page ---
    let authorize_url = format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope=mcp%3Aread&state=test-state-123&code_challenge={}&code_challenge_method=S256&resource={}",
        urlencoded(&oauth_client_id),
        urlencoded(REDIRECT_URI),
        CODE_CHALLENGE,
        urlencoded(RESOURCE),
    );
    let authorize_resp = client.get(&authorize_url).bearer(&user_jwt).send().await;
    assert_eq!(authorize_resp.status(), StatusCode::FOUND);
    let location = authorize_resp
        .headers()
        .get("location")
        .expect("Location header missing")
        .to_str()
        .expect("Location is not ASCII");
    let request_id = location
        .strip_prefix("/oauth/consent/")
        .expect("Location must start with /oauth/consent/")
        .split('?')
        .next()
        .expect("split always yields at least one element");

    // --- Step 2: POST /oauth/consent/{id}/deny → redirect_to with error ---
    let (deny_status, deny_body): (_, serde_json::Value) = client
        .post_empty(&format!("/oauth/consent/{request_id}/deny"))
        .bearer(&user_jwt)
        .send_json()
        .await;
    assert_eq!(deny_status, StatusCode::OK);
    let redirect_to = deny_body["redirect_to"]
        .as_str()
        .expect("redirect_to must be a string");
    assert!(
        redirect_to.contains("error=access_denied"),
        "redirect_to must contain error=access_denied, got: {redirect_to}",
    );
    // The server percent-encodes the state with NON_ALPHANUMERIC, so hyphens
    // become %2D.  Assert the encoded form that actually appears in the URL.
    assert!(
        redirect_to.contains("state=test%2Dstate%2D123"),
        "redirect_to must preserve state param (percent-encoded), got: {redirect_to}",
    );
}
