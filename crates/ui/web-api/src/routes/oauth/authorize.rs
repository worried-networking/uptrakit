//! OAuth 2.1 authorization endpoint — `GET /oauth/authorize`.
//!
//! Per spec §12.1: validate the request, redirect to login when
//! unauthenticated, skip consent when prior grant covers the scope,
//! otherwise redirect to the consent UI.

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uptrakit_web_api_types::oauth::AuthorizeRequest;
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::extract::ClientIp;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::oauth::rate_limit::{EndpointKind, OAuthRateLimiter, check_rate_limit};
use crate::oauth::services::authorization_code::{
    MintAuthorizationCode, OAuthAuthorizationCodeService,
};
use crate::oauth::services::authorization_request::{
    CreateAuthorizationRequest, OAuthAuthorizationRequestService,
};
use crate::oauth::services::client::OAuthClientService;
use crate::oauth::services::consent::OAuthConsentService;
use uptrakit_web_api_auth::auth::rate_limit::RateLimitStore;

/// OAuth 2.1 §12.1 authorization endpoint.
///
/// Validates the request, redirects to login if unauthenticated, skips
/// consent when prior consent covers the scope, otherwise creates an
/// authorization request row and redirects to `/oauth/consent/<id>`.
///
/// Returns 404 when `oauth.mcp_enabled = false`.
#[utoipa::path(
    get,
    path = "/oauth/authorize",
    params(
        ("response_type" = String, Query, description = "Must be \"code\""),
        ("client_id" = String, Query, description = "Client identifier"),
        ("redirect_uri" = String, Query, description = "Registered redirect URI"),
        ("scope" = String, Query, description = "Requested scope"),
        ("state" = String, Query, description = "CSRF state value"),
        ("code_challenge" = String, Query, description = "PKCE code challenge"),
        ("code_challenge_method" = String, Query, description = "Must be \"S256\""),
        ("resource" = String, Query, description = "RFC 8707 resource indicator"),
    ),
    responses(
        (status = 302, description = "Redirect to login, consent, or redirect_uri"),
        (status = 400, description = "Invalid request"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn authorize(
    State(state): State<Arc<AppState>>,
    client_ip: Option<Extension<ClientIp>>,
    auth_user: Option<Extension<AuthenticatedUser>>,
    Query(req): Query<AuthorizeRequest>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Step 1 — rate-limit check.
    let ip_str = match &client_ip {
        Some(Extension(ClientIp(ip))) => ip.to_string(),
        None => "unknown".to_string(),
    };
    let limiter = OAuthRateLimiter::new(RateLimitStore::new(state.db.db().clone()));
    if let Some(r) = check_rate_limit(EndpointKind::Authorize, &limiter, &ip_str).await {
        return r;
    }

    // Step 2 — validate the request parameters.
    if let Err(e) = req.validate() {
        return oauth_400("invalid_request", &e.to_string());
    }

    // Step 3 — look up the client.
    let client_svc = OAuthClientService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );
    let client = match client_svc.lookup(&req.client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return oauth_400("invalid_client", "unknown client_id"),
        Err(e) => {
            tracing::error!(error = %e, "oauth client lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Step 4 — validate redirect_uri against registered list.
    let registered_uris: Vec<String> =
        serde_json::from_str(&client.redirect_uris).unwrap_or_default();
    if !registered_uris.contains(&req.redirect_uri) {
        return oauth_400(
            "invalid_redirect_uri",
            "redirect_uri not registered for this client",
        );
    }

    // Step 5 — validate resource indicator.
    if !state.oauth.canonical.accepts_audience(&req.resource) {
        return oauth_400(
            "invalid_target",
            "resource indicator not accepted by this server",
        );
    }

    // Step 6 — check authentication.
    let user = match auth_user {
        Some(Extension(u)) => u,
        None => {
            // Not logged in — redirect to /login with return_to pointing here.
            let original_uri = build_authorize_uri(&req);
            let encoded = percent_encode(&original_uri);
            let location = format!("/login?return_to={encoded}&_auth_context=oauth");
            return redirect_302(&location);
        }
    };

    // Step 7 — check consent.
    let consent_svc =
        OAuthConsentService::new(state.db.db().clone(), Arc::clone(&state.oauth.clock));
    let skip_prompt = match consent_svc
        .should_skip_prompt(user.user_id, &req.client_id, &req.scope)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "consent should_skip_prompt failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if skip_prompt {
        // Mint an authorization code directly and redirect back to the client.
        let ar_svc = OAuthAuthorizationRequestService::new(
            state.db.db().clone(),
            Arc::clone(&state.oauth.clock),
        );
        let request_id = match ar_svc
            .create(CreateAuthorizationRequest {
                client_id: req.client_id.clone(),
                user_id: user.user_id,
                redirect_uri: req.redirect_uri.clone(),
                scope: req.scope.clone(),
                state: req.state.clone(),
                code_challenge: req.code_challenge.clone(),
                code_challenge_method: req.code_challenge_method.clone(),
                resource: req.resource.clone(),
            })
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = %e, "failed to create authorization request");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

        let code_svc = OAuthAuthorizationCodeService::new(
            state.db.db().clone(),
            Arc::clone(&state.oauth.clock),
        );
        let code = match code_svc
            .mint(MintAuthorizationCode {
                request_id,
                client_id: req.client_id.clone(),
                user_id: user.user_id,
                redirect_uri: req.redirect_uri.clone(),
                scope: req.scope.clone(),
                code_challenge: req.code_challenge.clone(),
                code_challenge_method: req.code_challenge_method.clone(),
                resource: req.resource.clone(),
            })
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "failed to mint authorization code");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

        let location = format!(
            "{}?code={}&state={}",
            req.redirect_uri,
            percent_encode(code.as_str()),
            percent_encode(&req.state),
        );
        return redirect_302(&location);
    }

    // Consent required — create the authorization request row and redirect to
    // the consent UI.
    let ar_svc = OAuthAuthorizationRequestService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
    );
    let request_id = match ar_svc
        .create(CreateAuthorizationRequest {
            client_id: req.client_id.clone(),
            user_id: user.user_id,
            redirect_uri: req.redirect_uri.clone(),
            scope: req.scope.clone(),
            state: req.state.clone(),
            code_challenge: req.code_challenge.clone(),
            code_challenge_method: req.code_challenge_method.clone(),
            resource: req.resource.clone(),
        })
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "failed to create authorization request");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let location = format!("/oauth/consent/{request_id}");
    redirect_302(&location)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the `GET /oauth/authorize?...` URI from the parsed request struct.
///
/// Used to reconstruct the original URL for the `return_to` redirect when the
/// user is not authenticated.
fn build_authorize_uri(req: &AuthorizeRequest) -> String {
    format!(
        "/oauth/authorize?response_type={}&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method={}&resource={}",
        percent_encode(&req.response_type),
        percent_encode(&req.client_id),
        percent_encode(&req.redirect_uri),
        percent_encode(&req.scope),
        percent_encode(&req.state),
        percent_encode(&req.code_challenge),
        percent_encode(&req.code_challenge_method),
        percent_encode(&req.resource),
    )
}

/// Percent-encode a string for safe use in query string values.
fn percent_encode(s: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Return a 302 redirect response.
fn redirect_302(location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(axum::http::header::LOCATION, location)],
    )
        .into_response()
}

/// Return a 400 JSON OAuth error response.
///
/// Body: `{"error":"<code>","error_description":"<desc>"}`.
fn oauth_400(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test assertions — panics on setup failure are acceptable in tests"
    )]

    use std::sync::Arc;

    use sea_orm::{ActiveModelTrait, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::oauth_client;

    use crate::oauth::OAuthState;
    use crate::oauth::canonical_url::CanonicalUrlConfig;
    use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
    use crate::router::build_router;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    // -----------------------------------------------------------------------
    // Shared helpers
    // -----------------------------------------------------------------------

    fn enabled_oauth_state() -> OAuthState {
        let canonical = CanonicalUrlConfig::new("controller.example.com".to_string(), vec![])
            .expect("test canonical url");
        OAuthState {
            enabled: true,
            canonical,
            signer: Arc::new(McpOAuthJwtSigner::new(b"test-secret-not-used")),
            verifier: Arc::new(McpOAuthJwtVerifier::new(
                b"test-secret-not-used",
                "https://controller.example.com".into(),
                vec![],
            )),
            clock: Arc::new(OffsetDateTime::now_utc),
            instance_id: uuid::Uuid::nil(),
            dcr_enabled: false,
            cimd_enabled: false,
        }
    }

    async fn app_with_oauth() -> (crate::test_harness::TestApp, axum::Router) {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = enabled_oauth_state();
        let state = Arc::new(patched);
        let router = build_router(Arc::clone(&state));
        let app = crate::test_harness::TestApp {
            state,
            router: router.clone(),
            db,
            jwt,
            tenant_id,
        };
        (app, router)
    }

    /// Insert a minimal `oauth_clients` row with a known redirect URI and
    /// return the client_id.
    async fn insert_oauth_client(db: &sea_orm::DatabaseConnection, redirect_uri: &str) -> String {
        let now = OffsetDateTime::now_utc();
        let client_id = format!("test-client-{}", uuid::Uuid::now_v7());
        // Store the redirect_uri as a JSON array (as the service does).
        let redirect_uris_json = serde_json::to_string(&vec![redirect_uri]).expect("serialize");

        oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set("Test Client".to_string()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set(redirect_uris_json),
            default_scope: Set("openid mcp:read".to_string()),
            grant_types: Set("authorization_code".to_string()),
            response_types: Set("code".to_string()),
            token_endpoint_auth_method: Set("none".to_string()),
            client_secret_hash: Set(None),
            registration_access_token_hash: Set(None),
            created_via: Set("test".to_string()),
            created_at: Set(now),
            last_used_at: Set(None),
            revoked_at: Set(None),
            metadata_cached_at: Set(None),
            metadata_etag: Set(None),
            metadata_content_hash: Set(None),
            metadata_raw: Set(None),
            metadata_parse_error: Set(None),
            metadata_parse_error_at: Set(None),
            trusted_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_client");

        client_id
    }

    /// Build a valid query string for the authorize endpoint.
    fn valid_query(client_id: &str, redirect_uri: &str) -> String {
        format!(
            "/oauth/authorize\
             ?response_type=code\
             &client_id={client_id}\
             &redirect_uri={redirect_uri}\
             &scope=mcp%3Aread\
             &state=test-state-1234\
             &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
             &code_challenge_method=S256\
             &resource=https%3A%2F%2Fcontroller.example.com%2Fmcp"
        )
    }

    // -----------------------------------------------------------------------
    // Test 1 — unauthenticated request redirects to /login
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn authorize_unauthenticated_redirects_to_login() {
        use axum::body::Body;
        use http::Request;
        use tower::ServiceExt;

        let (app, router) = app_with_oauth().await;
        let client_id = insert_oauth_client(&app.db, "https://example.com/callback").await;

        let uri = valid_query(&client_id, "https%3A%2F%2Fexample.com%2Fcallback");

        let req = Request::builder()
            .method("GET")
            .uri(&uri)
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::FOUND);
        let location = resp
            .headers()
            .get("location")
            .expect("location header")
            .to_str()
            .expect("location is utf8");
        assert!(
            location.starts_with("/login?return_to="),
            "expected /login redirect, got: {location}"
        );
        assert!(
            location.contains("_auth_context=oauth"),
            "expected _auth_context=oauth in location: {location}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2 — unknown client_id returns 400 invalid_client
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn authorize_invalid_client_returns_400() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let (app, router) = app_with_oauth().await;

        // Valid token for an existing user, but client_id does not exist in DB.
        let user_id = uuid::Uuid::now_v7();
        let jwt_token = app
            .jwt
            .create_access_token(user_id, &[], "password", None)
            .expect("create_access_token");

        let uri = "/oauth/authorize\
                   ?response_type=code\
                   &client_id=does-not-exist\
                   &redirect_uri=https%3A%2F%2Fexample.com%2Fcallback\
                   &scope=mcp%3Aread\
                   &state=s\
                   &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
                   &code_challenge_method=S256\
                   &resource=https%3A%2F%2Fcontroller.example.com%2Fmcp";

        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {jwt_token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
        assert_eq!(body["error"], "invalid_client");
    }

    // -----------------------------------------------------------------------
    // Test 3 — known client with wrong redirect_uri returns 400
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn authorize_invalid_redirect_uri_returns_400() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let (app, router) = app_with_oauth().await;

        // Insert a client registered with one URI, but request a different one.
        let client_id = insert_oauth_client(&app.db, "https://example.com/callback").await;

        let user_id = uuid::Uuid::now_v7();
        let jwt_token = app
            .jwt
            .create_access_token(user_id, &[], "password", None)
            .expect("create_access_token");

        let uri = format!(
            "/oauth/authorize\
             ?response_type=code\
             &client_id={client_id}\
             &redirect_uri=https%3A%2F%2Fevil.example.com%2Fcallback\
             &scope=mcp%3Aread\
             &state=s\
             &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
             &code_challenge_method=S256\
             &resource=https%3A%2F%2Fcontroller.example.com%2Fmcp"
        );

        let req = Request::builder()
            .method("GET")
            .uri(&uri)
            .header("authorization", format!("Bearer {jwt_token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
        assert_eq!(body["error"], "invalid_redirect_uri");
    }

    // -----------------------------------------------------------------------
    // Test 4 — non-S256 code_challenge_method returns 400 (caught by validate)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn authorize_non_s256_returns_400() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let (_app, router) = app_with_oauth().await;

        let uri = "/oauth/authorize\
                   ?response_type=code\
                   &client_id=any-client\
                   &redirect_uri=https%3A%2F%2Fexample.com%2Fcallback\
                   &scope=mcp%3Aread\
                   &state=s\
                   &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
                   &code_challenge_method=plain\
                   &resource=https%3A%2F%2Fcontroller.example.com%2Fmcp";

        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
        assert_eq!(body["error"], "invalid_request");
    }

    // -----------------------------------------------------------------------
    // Test 5 — missing / empty resource returns 400
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn authorize_missing_resource_returns_400() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let (_app, router) = app_with_oauth().await;

        // `resource` is intentionally an empty string — should fail validate().
        let uri = "/oauth/authorize\
                   ?response_type=code\
                   &client_id=any-client\
                   &redirect_uri=https%3A%2F%2Fexample.com%2Fcallback\
                   &scope=mcp%3Aread\
                   &state=s\
                   &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
                   &code_challenge_method=S256\
                   &resource=";

        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
        assert_eq!(body["error"], "invalid_request");
    }
}
