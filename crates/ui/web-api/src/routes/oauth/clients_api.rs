//! Operator OAuth Clients API (spec §11.4).
//!
//! GET    /api/oauth/clients                — list registered clients (paginated)
//! POST   /api/oauth/clients                — manually register a client (RFC 7591 shape)
//! DELETE /api/oauth/clients/{client_id}    — revoke a client (cascades to consents + tokens)
//! POST   /api/oauth/clients/{client_id}/trust — promote client to trusted
//!
//! All four endpoints require the `ManageAuthSettings` permission.
//! No rate limit — this is the Operator API (spec §11.4).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{EntityTrait, PaginatorTrait};
use uptrakit_shared_db::entity::oauth_client;
use uptrakit_web_api_types::oauth::responses::{DcrRegistrationRequest, DcrRegistrationResponse};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::api_error::ApiError;
use crate::middleware::permission::CanManageAuthSettings;
use crate::oauth::http_responses::{oauth_400, oauth_500};
use crate::oauth::services::client::OAuthClientService;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_client_service(state: &AppState) -> OAuthClientService {
    OAuthClientService::new(
        state.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    )
}

// ---------------------------------------------------------------------------
// GET /api/oauth/clients
// ---------------------------------------------------------------------------

/// List all registered OAuth clients (paginated).
///
/// Returns 404 when `oauth.enabled = false`.
/// Requires `ManageAuthSettings`.
#[utoipa::path(
    get,
    path = "/api/oauth/clients",
    params(PaginationParams),
    responses(
        (status = 200, description = "Paginated client list"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permission"),
        (status = 404, description = "OAuth disabled"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub(crate) async fn list_clients(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(_user): CanManageAuthSettings,
    Query(pagination): Query<PaginationParams>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let pag = pagination.resolve();

    let paginator = oauth_client::Entity::find().paginate(state.db(), pag.per_page);

    let total = match paginator.num_items().await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "failed to count oauth_clients");
            return oauth_500();
        }
    };

    let items = match paginator.fetch_page(pag.page - 1).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch oauth_clients page");
            return oauth_500();
        }
    };

    let client_jsons: Vec<serde_json::Value> = items
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id,
                "client_name": row.client_name,
                "client_uri": row.client_uri,
                "redirect_uris": serde_json::from_str::<serde_json::Value>(&row.redirect_uris)
                    .unwrap_or(serde_json::Value::Array(vec![])),
                "created_via": row.created_via,
                "created_at": row.created_at,
                "last_used_at": row.last_used_at,
                "revoked_at": row.revoked_at,
                "trusted_at": row.trusted_at,
            })
        })
        .collect();

    (
        StatusCode::OK,
        axum::Json(PaginatedResponse::new(client_jsons, total, pag)),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// POST /api/oauth/clients
// ---------------------------------------------------------------------------

/// Manually register a new OAuth client (operator-initiated, no cap check).
///
/// Returns 404 when `oauth.enabled = false`.
/// Requires `ManageAuthSettings`.
#[utoipa::path(
    post,
    path = "/api/oauth/clients",
    request_body = DcrRegistrationRequest,
    responses(
        (status = 201, description = "Client registered", body = DcrRegistrationResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permission"),
        (status = 404, description = "OAuth disabled"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub(crate) async fn manual_register_client(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(_user): CanManageAuthSettings,
    axum::Json(body): axum::Json<DcrRegistrationRequest>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    if let Err(e) = body.validate() {
        return oauth_400("invalid_request", &e.message);
    }

    let svc = build_client_service(&state);

    match svc.register_manual(body).await {
        Ok(resp) => (StatusCode::CREATED, axum::Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "manual register failed");
            oauth_500()
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/oauth/clients/{client_id}
// ---------------------------------------------------------------------------

/// Revoke an OAuth client and cascade-revoke all its consents and refresh tokens.
///
/// Returns 404 when the client does not exist or when `oauth.enabled = false`.
/// Returns 204 on success.
/// Requires `ManageAuthSettings`.
#[utoipa::path(
    delete,
    path = "/api/oauth/clients/{client_id}",
    params(("client_id" = String, Path, description = "OAuth client ID")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permission"),
        (status = 404, description = "Client not found or OAuth disabled"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub(crate) async fn revoke_client(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(_user): CanManageAuthSettings,
    Path(client_id): Path<String>,
) -> Result<Response, ApiError> {
    if !state.oauth.enabled {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let svc = build_client_service(&state);
    svc.revoke(&client_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// POST /api/oauth/clients/{client_id}/trust
// ---------------------------------------------------------------------------

/// Promote an OAuth client to trusted status by setting `trusted_at = now`.
///
/// Returns 404 when the client does not exist or when `oauth.enabled = false`.
/// Returns 204 on success.
/// Requires `ManageAuthSettings`.
#[utoipa::path(
    post,
    path = "/api/oauth/clients/{client_id}/trust",
    params(("client_id" = String, Path, description = "OAuth client ID")),
    responses(
        (status = 204, description = "Promoted to trusted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permission"),
        (status = 404, description = "Client not found or OAuth disabled"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub(crate) async fn trust_client(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(_user): CanManageAuthSettings,
    Path(client_id): Path<String>,
) -> Result<Response, ApiError> {
    if !state.oauth.enabled {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let svc = build_client_service(&state);
    svc.promote_trusted(&client_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
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
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::oauth_client;
    use uptrakit_web_api_types::pagination::PaginatedResponse;

    use crate::oauth::OAuthState;
    use crate::oauth::canonical_url::CanonicalUrlConfig;
    use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
    use crate::router::build_router;
    use crate::test_harness::fixtures::register_and_get_token;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    // -----------------------------------------------------------------------
    // Test helpers
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
            dcr_enabled: true,
            cimd_enabled: false,
        }
    }

    async fn app_with_oauth(oauth: OAuthState) -> (axum::Router, sea_orm::DatabaseConnection) {
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
            "client_name": "Test Operator Client",
            "redirect_uris": ["https://example.com/callback"],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        })
    }

    // -----------------------------------------------------------------------
    // Test 1 — list_clients requires ManageAuthSettings
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_clients_requires_manage_auth_settings() {
        let (router, _db) = app_with_oauth(enabled_oauth_state()).await;
        let client = crate::test_harness::http_client::TestClient::new(router);

        // No Authorization header → 401 from the permission extractor.
        let status = client.get("/api/oauth/clients").send_status().await;
        assert_eq!(status, http::StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // Test 2 — list_clients returns paginated empty on fresh DB
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_clients_returns_paginated_empty() {
        let (router, _db) = app_with_oauth(enabled_oauth_state()).await;
        let client = crate::test_harness::http_client::TestClient::new(router);
        let token = register_and_get_token(&client).await;

        let (status, body): (http::StatusCode, PaginatedResponse<serde_json::Value>) = client
            .get("/api/oauth/clients")
            .bearer(&token)
            .send_json()
            .await;

        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(body.total, 0);
        assert!(body.items.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test 3 — manual_register_client returns 201 with client_id
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn manual_register_returns_201_with_client_id() {
        let (router, _db) = app_with_oauth(enabled_oauth_state()).await;
        let client = crate::test_harness::http_client::TestClient::new(router);
        let token = register_and_get_token(&client).await;

        let (status, body): (http::StatusCode, serde_json::Value) = client
            .post_json("/api/oauth/clients", &minimal_dcr_body())
            .bearer(&token)
            .send_json()
            .await;

        assert_eq!(status, http::StatusCode::CREATED);
        assert!(
            !body["client_id"].as_str().unwrap_or("").is_empty(),
            "client_id must be present and non-empty"
        );
        // Operator sees the registration access token once.
        assert!(
            !body["registration_access_token"]
                .as_str()
                .unwrap_or("")
                .is_empty(),
            "registration_access_token must be present for operator-registered clients"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4 — revoke cascades and returns 204
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn revoke_cascades_returns_204() {
        let (router, db) = app_with_oauth(enabled_oauth_state()).await;
        let client = crate::test_harness::http_client::TestClient::new(router.clone());
        let token = register_and_get_token(&client).await;

        // Register a client.
        let (status, body): (http::StatusCode, serde_json::Value) = client
            .post_json("/api/oauth/clients", &minimal_dcr_body())
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);
        let client_id = body["client_id"].as_str().expect("client_id").to_string();

        // Revoke via DELETE.
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/oauth/clients/{client_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NO_CONTENT);

        // Verify revoked_at IS NOT NULL in DB.
        let row = oauth_client::Entity::find()
            .filter(oauth_client::Column::Id.eq(client_id))
            .one(&db)
            .await
            .expect("db query")
            .expect("row must exist");
        assert!(
            row.revoked_at.is_some(),
            "revoked_at must be set after DELETE"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5 — revoke unknown client returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn revoke_unknown_client_returns_404() {
        let (router, _db) = app_with_oauth(enabled_oauth_state()).await;
        let client = crate::test_harness::http_client::TestClient::new(router.clone());
        let token = register_and_get_token(&client).await;

        let req = Request::builder()
            .method("DELETE")
            .uri("/api/oauth/clients/nonexistent-client-id-xyz")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Test 6 — trust sets trusted_at and returns 204
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn trust_sets_trusted_at_returns_204() {
        let (router, db) = app_with_oauth(enabled_oauth_state()).await;
        let client = crate::test_harness::http_client::TestClient::new(router.clone());
        let token = register_and_get_token(&client).await;

        // Register a client.
        let (status, body): (http::StatusCode, serde_json::Value) = client
            .post_json("/api/oauth/clients", &minimal_dcr_body())
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);
        let client_id = body["client_id"].as_str().expect("client_id").to_string();

        // Trust via POST /{client_id}/trust.
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/oauth/clients/{client_id}/trust"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NO_CONTENT);

        // Verify trusted_at IS NOT NULL in DB.
        let row = oauth_client::Entity::find()
            .filter(oauth_client::Column::Id.eq(client_id))
            .one(&db)
            .await
            .expect("db query")
            .expect("row must exist");
        assert!(
            row.trusted_at.is_some(),
            "trusted_at must be set after trust"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6 — oauth disabled returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn oauth_disabled_returns_404() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        // Default state has oauth.enabled = false.
        assert!(
            !state.oauth.enabled,
            "oauth must be disabled in default test state"
        );

        // Mint a token with ManageAuthSettings so we can reach the handler.
        // Use a nil user_id — the auth middleware validates the JWT signature
        // but does not verify the user exists in DB for permission extractors.
        let user_id = uuid::Uuid::nil();
        let token = jwt
            .create_access_token(
                user_id,
                &[crate::auth::permissions::Permission::ManageAuthSettings],
                "password",
                None,
                None,
            )
            .expect("create access token");

        let router = build_router(Arc::clone(&state));
        let client = crate::test_harness::http_client::TestClient::new(router);

        let status = client
            .get("/api/oauth/clients")
            .bearer(&token)
            .send_status()
            .await;

        assert_eq!(status, http::StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Test 7 — no permission returns 403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_clients_returns_403_without_permission() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = enabled_oauth_state();
        let router = build_router(Arc::new(patched));
        let client = crate::test_harness::http_client::TestClient::new(router);

        // Mint a token with NO ManageAuthSettings permission.
        let user_id = uuid::Uuid::nil();
        let token = jwt
            .create_access_token(user_id, &[], "password", None, None)
            .expect("create access token");

        let status = client
            .get("/api/oauth/clients")
            .bearer(&token)
            .send_status()
            .await;

        assert_eq!(status, http::StatusCode::FORBIDDEN);
    }
}
