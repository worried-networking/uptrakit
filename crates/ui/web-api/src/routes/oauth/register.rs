//! RFC 7591 Dynamic Client Registration + RFC 7592 management endpoints.
//!
//! POST /oauth/register            — DCR (RFC 7591 §3.1, returns 201)
//! GET  /oauth/register/{client_id} — read registration (RFC 7592)
//! PUT  /oauth/register/{client_id} — update registration (RFC 7592)
//! DELETE /oauth/register/{client_id} — revoke registration (RFC 7592)

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use sea_orm::{ActiveModelTrait, Set};
use uptrakit_shared_db::entity::oauth_client;
use uptrakit_web_api_auth::auth::rate_limit::RateLimitStore;
use uptrakit_web_api_types::oauth::responses::{DcrRegistrationRequest, DcrRegistrationResponse};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::extract::ClientIp;
use crate::oauth::http_responses::{oauth_400, oauth_403, oauth_500};
use crate::oauth::rate_limit::{EndpointKind, OAuthRateLimiter, check_rate_limit};
use crate::oauth::services::client::{OAuthClientService, registration_error_to_response};

// ---------------------------------------------------------------------------
// Local error helpers
// ---------------------------------------------------------------------------

fn oauth_401(description: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": "unauthorized_client",
            "error_description": description,
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Helper: extract Bearer token from Authorization header
// ---------------------------------------------------------------------------

fn extract_registration_bearer(headers: &HeaderMap) -> Option<String> {
    let auth_value = headers.get(axum::http::header::AUTHORIZATION)?;
    let auth_str = auth_value.to_str().ok()?;
    let token = auth_str.strip_prefix("Bearer ")?;
    if token.is_empty() {
        return None;
    }
    Some(token.to_owned())
}

// ---------------------------------------------------------------------------
// Helper: constant-time verification of the registration access token
// ---------------------------------------------------------------------------

fn verify_registration_token(client: &oauth_client::Model, bearer: &str) -> bool {
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;

    let stored_hash = match &client.registration_access_token_hash {
        Some(h) => h,
        None => return false,
    };

    // Hash the bearer token exactly as `hash_token` does (SHA-256, hex-encoded).
    let mut hasher = Sha256::new();
    hasher.update(bearer.as_bytes());
    let computed = format!("{:x}", hasher.finalize());

    // Compare as equal-length byte slices to prevent timing attacks.
    computed.as_bytes().ct_eq(stored_hash.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// POST /oauth/register
// ---------------------------------------------------------------------------

/// RFC 7591 §3.1 Dynamic Client Registration.
///
/// Returns 201 Created with a `DcrRegistrationResponse` body.
/// Returns 404 when `oauth.enabled = false`.
/// Returns 403 when `oauth.dcr_enabled = false`.
#[utoipa::path(
    post,
    path = "/oauth/register",
    responses(
        (status = 201, description = "Client registered (RFC 7591 §3.2.1)", content_type = "application/json"),
        (status = 400, description = "Invalid client metadata"),
        (status = 403, description = "DCR disabled or cap exceeded"),
        (status = 404, description = "OAuth disabled"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn register(
    State(state): State<Arc<AppState>>,
    client_ip: Option<Extension<ClientIp>>,
    body: axum::extract::Json<DcrRegistrationRequest>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    if !state.oauth.dcr_enabled {
        return oauth_403("dcr_disabled", "dynamic client registration is not enabled");
    }

    // Rate-limit check per IP.
    let ip_str = match &client_ip {
        Some(Extension(ClientIp(ip))) => ip.to_string(),
        None => "unknown".to_string(),
    };
    let limiter = OAuthRateLimiter::new(RateLimitStore::new(state.db.db().clone()));
    if let Some(r) = check_rate_limit(EndpointKind::Dcr, &limiter, &ip_str).await {
        return r;
    }

    // Validate the request body.
    let req = body.0;
    if let Err(e) = req.validate() {
        return oauth_400("invalid_client_metadata", &e.to_string());
    }

    // Resolve the source IP (IpAddr) for the service call.
    let source_ip: std::net::IpAddr = match &client_ip {
        Some(Extension(ClientIp(ip))) => *ip,
        None => std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
    };

    let client_svc = OAuthClientService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );

    match client_svc.register_dcr(req, source_ip, &[]).await {
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(e) => registration_error_to_response(&e),
    }
}

// ---------------------------------------------------------------------------
// GET /oauth/register/{client_id}
// ---------------------------------------------------------------------------

/// RFC 7592 client configuration endpoint — read.
///
/// Returns 200 with the current registration metadata.
/// Returns 401 when the registration access token is missing or invalid.
/// Returns 404 when the client is unknown or revoked.
#[utoipa::path(
    get,
    path = "/oauth/register/{client_id}",
    params(
        ("client_id" = String, Path, description = "OAuth client identifier"),
    ),
    responses(
        (status = 200, description = "Client registration metadata (RFC 7592)", content_type = "application/json"),
        (status = 401, description = "Invalid or missing registration access token"),
        (status = 404, description = "Client not found or revoked, or OAuth disabled"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all, fields(client_id = %client_id))]
pub async fn get_client_registration(
    State(state): State<Arc<AppState>>,
    Path(client_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let bearer = match extract_registration_bearer(&headers) {
        Some(t) => t,
        None => return oauth_401("missing or malformed Authorization header"),
    };

    let client_svc = OAuthClientService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );

    let client = match client_svc.lookup(&client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "client lookup failed");
            return oauth_500();
        }
    };

    // Treat revoked clients as not found (per RFC 7592 §2.2).
    if client.revoked_at.is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }

    if !verify_registration_token(&client, &bearer) {
        return oauth_401("invalid registration access token");
    }

    let Some(resp) = client_to_response(&client) else {
        return oauth_500();
    };
    (StatusCode::OK, Json(resp)).into_response()
}

// ---------------------------------------------------------------------------
// PUT /oauth/register/{client_id}
// ---------------------------------------------------------------------------

/// RFC 7592 client configuration endpoint — update.
///
/// Returns 200 with the updated registration metadata.
#[utoipa::path(
    put,
    path = "/oauth/register/{client_id}",
    params(
        ("client_id" = String, Path, description = "OAuth client identifier"),
    ),
    responses(
        (status = 200, description = "Updated client registration metadata (RFC 7592)", content_type = "application/json"),
        (status = 400, description = "Invalid request body"),
        (status = 401, description = "Invalid or missing registration access token"),
        (status = 404, description = "Client not found or revoked, or OAuth disabled"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all, fields(client_id = %client_id))]
pub async fn update_client_registration(
    State(state): State<Arc<AppState>>,
    Path(client_id): Path<String>,
    headers: HeaderMap,
    body: axum::extract::Json<DcrRegistrationRequest>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let bearer = match extract_registration_bearer(&headers) {
        Some(t) => t,
        None => return oauth_401("missing or malformed Authorization header"),
    };

    let client_svc = OAuthClientService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );

    let client = match client_svc.lookup(&client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "client lookup failed");
            return oauth_500();
        }
    };

    if client.revoked_at.is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }

    if !verify_registration_token(&client, &bearer) {
        return oauth_401("invalid registration access token");
    }

    let req = body.0;
    if let Err(e) = req.validate() {
        return oauth_400("invalid_client_metadata", &e.to_string());
    }

    let redirect_uris_json = match serde_json::to_string(&req.redirect_uris) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize redirect_uris");
            return oauth_500();
        }
    };
    let grant_types_json = match serde_json::to_string(&req.grant_types) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize grant_types");
            return oauth_500();
        }
    };
    let response_types_json = match serde_json::to_string(&req.response_types) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize response_types");
            return oauth_500();
        }
    };

    let mut active: oauth_client::ActiveModel = client.into();
    active.client_name = Set(req.client_name.clone());
    active.client_uri = Set(req.client_uri.clone());
    active.logo_uri = Set(req.logo_uri.clone());
    active.redirect_uris = Set(redirect_uris_json);
    active.grant_types = Set(grant_types_json);
    active.response_types = Set(response_types_json);
    active.token_endpoint_auth_method = Set(req.token_endpoint_auth_method.clone());
    active.default_scope = Set(req.scope.clone().unwrap_or_default());

    let updated = match active.update(state.db.db()).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "client update failed");
            return oauth_500();
        }
    };

    let Some(resp) = client_to_response(&updated) else {
        return oauth_500();
    };
    (StatusCode::OK, Json(resp)).into_response()
}

// ---------------------------------------------------------------------------
// DELETE /oauth/register/{client_id}
// ---------------------------------------------------------------------------

/// RFC 7592 client configuration endpoint — revoke.
///
/// Returns 204 No Content on success.
#[utoipa::path(
    delete,
    path = "/oauth/register/{client_id}",
    params(
        ("client_id" = String, Path, description = "OAuth client identifier"),
    ),
    responses(
        (status = 204, description = "Client registration revoked"),
        (status = 401, description = "Invalid or missing registration access token"),
        (status = 404, description = "Client not found or revoked, or OAuth disabled"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all, fields(client_id = %client_id))]
pub async fn delete_client_registration(
    State(state): State<Arc<AppState>>,
    Path(client_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let bearer = match extract_registration_bearer(&headers) {
        Some(t) => t,
        None => return oauth_401("missing or malformed Authorization header"),
    };

    let client_svc = OAuthClientService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );

    let client = match client_svc.lookup(&client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "client lookup failed");
            return oauth_500();
        }
    };

    if client.revoked_at.is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }

    if !verify_registration_token(&client, &bearer) {
        return oauth_401("invalid registration access token");
    }

    match client_svc.revoke(&client_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "client revocation failed");
            oauth_500()
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a [`DcrRegistrationResponse`] from a stored `oauth_client::Model`.
///
/// The `registration_access_token` is omitted (`None`) — it is a one-time
/// secret returned only in the initial POST response and must never be
/// re-exposed on GET or PUT (RFC 7592 §2).
fn client_to_response(client: &oauth_client::Model) -> Option<DcrRegistrationResponse> {
    let redirect_uris: Vec<String> = match serde_json::from_str(&client.redirect_uris) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to deserialize redirect_uris from db");
            return None;
        }
    };
    let grant_types: Vec<String> = match serde_json::from_str(&client.grant_types) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to deserialize grant_types from db");
            return None;
        }
    };
    let response_types: Vec<String> = match serde_json::from_str(&client.response_types) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to deserialize response_types from db");
            return None;
        }
    };
    Some(DcrRegistrationResponse::new(
        client.id.clone(),
        client.created_at.unix_timestamp(),
        None,
        format!("/oauth/register/{}", client.id),
        client.client_name.clone(),
        client.client_uri.clone(),
        client.logo_uri.clone(),
        redirect_uris,
        grant_types,
        response_types,
        client.token_endpoint_auth_method.clone(),
        client.default_scope.clone(),
    ))
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

    use axum::body::Body;
    use http::Request;
    use http_body_util::BodyExt;
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::oauth_client;
    use uptrakit_web_api_types::oauth::responses::DcrRegistrationResponse;

    use crate::oauth::OAuthState;
    use crate::oauth::canonical_url::CanonicalUrlConfig;
    use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
    use crate::router::build_router;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    // -----------------------------------------------------------------------
    // Shared constants
    // -----------------------------------------------------------------------

    const TEST_CLIENT_NAME: &str = "Test MCP Client";

    // -----------------------------------------------------------------------
    // Shared helpers
    // -----------------------------------------------------------------------

    fn enabled_oauth_state(dcr: bool, cimd: bool) -> OAuthState {
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
            dcr_enabled: dcr,
            cimd_enabled: cimd,
        }
    }

    async fn app_with_state(oauth: OAuthState) -> (axum::Router, sea_orm::DatabaseConnection) {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = oauth;
        let router = build_router(Arc::new(patched));
        (router, db)
    }

    fn minimal_dcr_body() -> serde_json::Value {
        serde_json::json!({
            "client_name": TEST_CLIENT_NAME,
            "redirect_uris": ["https://example.com/callback"],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        })
    }

    async fn do_register(router: &axum::Router) -> http::Response<Body> {
        let req = Request::builder()
            .method("POST")
            .uri("/oauth/register")
            .header("content-type", "application/json")
            .body(Body::from(minimal_dcr_body().to_string()))
            .expect("build request");
        router.clone().oneshot(req).await.expect("oneshot")
    }

    async fn register_and_get_token(router: &axum::Router) -> (String, String) {
        let resp = do_register(router).await;
        assert_eq!(resp.status(), http::StatusCode::CREATED);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let resp: DcrRegistrationResponse =
            serde_json::from_slice(&body_bytes).expect("parse DCR response");
        let token = resp
            .registration_access_token
            .expect("POST /oauth/register must return registration_access_token");
        (resp.client_id, token)
    }

    // -----------------------------------------------------------------------
    // Test 1 — POST valid DCR returns 201 with client_id
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_dcr_returns_201_with_client_id() {
        let (router, _db) = app_with_state(enabled_oauth_state(true, false)).await;

        let resp = do_register(&router).await;

        assert_eq!(resp.status(), http::StatusCode::CREATED);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
        assert!(
            !body["client_id"].as_str().unwrap_or("").is_empty(),
            "client_id must be present and non-empty"
        );
        assert!(
            !body["registration_access_token"]
                .as_str()
                .unwrap_or("")
                .is_empty(),
            "registration_access_token must be present"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2 — after DCR, DB row has created_via == "dcr"
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_dcr_persists_row_with_created_via_dcr() {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let (router, db) = app_with_state(enabled_oauth_state(true, false)).await;

        let resp = do_register(&router).await;
        assert_eq!(resp.status(), http::StatusCode::CREATED);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        let client_id = body["client_id"].as_str().expect("client_id").to_string();

        let row = oauth_client::Entity::find()
            .filter(oauth_client::Column::Id.eq(client_id))
            .one(&db)
            .await
            .expect("db query")
            .expect("row should exist");

        assert_eq!(row.created_via, "dcr", "created_via must be 'dcr'");
    }

    // -----------------------------------------------------------------------
    // Test 3 — DCR disabled returns 403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_dcr_disabled_returns_403() {
        let (router, _db) = app_with_state(enabled_oauth_state(false, false)).await;

        let resp = do_register(&router).await;

        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(body["error"], "dcr_disabled");
    }

    // -----------------------------------------------------------------------
    // Test 4 — GET with valid bearer returns 200 and client_name
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_get_returns_200_for_valid_token() {
        let (router, _db) = app_with_state(enabled_oauth_state(true, false)).await;

        let (client_id, token) = register_and_get_token(&router).await;

        let req = Request::builder()
            .method("GET")
            .uri(format!("/oauth/register/{client_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let resp = router.clone().oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: DcrRegistrationResponse =
            serde_json::from_slice(&body_bytes).expect("parse response");
        assert_eq!(body.client_name, TEST_CLIENT_NAME);
        assert!(
            body.registration_access_token.is_none(),
            "GET must not return the registration access token"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5 — GET with wrong bearer returns 401
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_get_returns_401_for_invalid_token() {
        let (router, _db) = app_with_state(enabled_oauth_state(true, false)).await;

        let (client_id, _real_token) = register_and_get_token(&router).await;

        let req = Request::builder()
            .method("GET")
            .uri(format!("/oauth/register/{client_id}"))
            .header("authorization", "Bearer wrong-token-value")
            .body(Body::empty())
            .expect("build request");
        let resp = router.clone().oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // Test 6 — DELETE revokes the client (revoked_at IS NOT NULL in DB)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_delete_revokes_client() {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let (router, db) = app_with_state(enabled_oauth_state(true, false)).await;

        let (client_id, token) = register_and_get_token(&router).await;

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/oauth/register/{client_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let resp = router.clone().oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::NO_CONTENT);

        let row = oauth_client::Entity::find()
            .filter(oauth_client::Column::Id.eq(client_id))
            .one(&db)
            .await
            .expect("db query")
            .expect("row should exist");

        assert!(
            row.revoked_at.is_some(),
            "revoked_at must be set after DELETE"
        );
    }

    // -----------------------------------------------------------------------
    // Test 7 — OAuth disabled → 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_oauth_disabled_returns_404() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        assert!(
            !state.oauth.enabled,
            "oauth must be disabled in default test state"
        );
        let router = build_router(Arc::clone(&state));

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/register")
            .header("content-type", "application/json")
            .body(Body::from(minimal_dcr_body().to_string()))
            .expect("build request");
        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Test 8 — GET after DELETE returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_get_after_delete_returns_404() {
        let (router, _db) = app_with_state(enabled_oauth_state(true, false)).await;

        let (client_id, token) = register_and_get_token(&router).await;

        // DELETE the client.
        let delete_req = Request::builder()
            .method("DELETE")
            .uri(format!("/oauth/register/{client_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let delete_resp = router.clone().oneshot(delete_req).await.expect("oneshot");
        assert_eq!(delete_resp.status(), http::StatusCode::NO_CONTENT);

        // GET must now return 404.
        let get_req = Request::builder()
            .method("GET")
            .uri(format!("/oauth/register/{client_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let get_resp = router.clone().oneshot(get_req).await.expect("oneshot");

        assert_eq!(
            get_resp.status(),
            http::StatusCode::NOT_FOUND,
            "GET after DELETE must return 404"
        );
    }

    // -----------------------------------------------------------------------
    // Test 9 — PUT updates fields and returns 200
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_put_updates_client_name() {
        let (router, _db) = app_with_state(enabled_oauth_state(true, false)).await;

        let (client_id, token) = register_and_get_token(&router).await;

        let updated_name = "Updated MCP Client";
        let put_body = serde_json::json!({
            "client_name": updated_name,
            "redirect_uris": ["https://example.com/callback"],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });

        let put_req = Request::builder()
            .method("PUT")
            .uri(format!("/oauth/register/{client_id}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(put_body.to_string()))
            .expect("build request");
        let put_resp = router.clone().oneshot(put_req).await.expect("oneshot");

        assert_eq!(put_resp.status(), http::StatusCode::OK);
        let body_bytes = put_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let body: DcrRegistrationResponse =
            serde_json::from_slice(&body_bytes).expect("parse PUT response");
        assert_eq!(
            body.client_name, updated_name,
            "PUT must update client_name"
        );
        assert!(
            body.registration_access_token.is_none(),
            "PUT must not return the registration access token"
        );
    }

    // -----------------------------------------------------------------------
    // Test 10 — PUT with wrong token returns 401
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_put_wrong_token_returns_401() {
        let (router, _db) = app_with_state(enabled_oauth_state(true, false)).await;

        let (client_id, _real_token) = register_and_get_token(&router).await;

        let put_body = serde_json::json!({
            "client_name": "Attacker Client",
            "redirect_uris": ["https://attacker.example.com/cb"],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });

        let put_req = Request::builder()
            .method("PUT")
            .uri(format!("/oauth/register/{client_id}"))
            .header("authorization", "Bearer wrong-token-value")
            .header("content-type", "application/json")
            .body(Body::from(put_body.to_string()))
            .expect("build request");
        let put_resp = router.clone().oneshot(put_req).await.expect("oneshot");

        assert_eq!(
            put_resp.status(),
            http::StatusCode::UNAUTHORIZED,
            "PUT with wrong token must return 401"
        );
    }
}
