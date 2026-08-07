//! End-user Authorized Apps API (spec §12.5).
//!
//! GET    /api/oauth/consents        — list the current user's active consent rows
//! DELETE /api/oauth/consents/{id}   — revoke a specific consent (ownership enforced)
//!
//! No `ManageAuthSettings` required — these endpoints are for end users.
//! Both routes sit behind the standard `require_auth` middleware.

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome, Event};
use uptrakit_shared_db::entity::{oauth_client, oauth_consent};
use uptrakit_web_api_types::oauth::responses::OAuthConsentResponse;
use uuid::Uuid;

use crate::AppState;
use crate::api_error::ApiError;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::oauth::http_responses::oauth_500;
use crate::oauth::services::consent::OAuthConsentService;

// ---------------------------------------------------------------------------
// GET /api/oauth/consents
// ---------------------------------------------------------------------------

/// List the authenticated user's active (non-revoked) OAuth consent rows.
///
/// Returns 404 when `oauth.enabled = false`.
#[utoipa::path(
    get,
    path = "/api/oauth/consents",
    responses(
        (status = 200, description = "List of active consents", body = Vec<OAuthConsentResponse>),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "OAuth disabled"),
    ),
    tag = "OAuth",
    security(("oauth2" = []), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub(crate) async fn list_consents(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthenticatedUser>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let rows = match oauth_consent::Entity::find()
        .filter(oauth_consent::Column::UserId.eq(auth_user.user_id))
        .filter(oauth_consent::Column::RevokedAt.is_null())
        .order_by_desc(oauth_consent::Column::GrantedAt)
        .find_also_related(oauth_client::Entity)
        .all(state.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to list consents");
            return oauth_500();
        }
    };

    // Serialize safe fields only — internal hashes are never exposed.
    let mut items = Vec::with_capacity(rows.len());
    for (consent, client) in rows {
        // FK-unreachable: fk_oauth_consents_client is ON DELETE RESTRICT and
        // client revocation is a soft delete, so the client row always exists.
        let Some(client) = client else {
            tracing::error!(consent_id = %consent.id, "oauth consent references missing client row");
            return oauth_500();
        };
        items.push(OAuthConsentResponse::new(
            consent.id,
            consent.client_id,
            client.client_name,
            consent.scopes,
            consent.granted_at,
        ));
    }

    (StatusCode::OK, axum::Json(items)).into_response()
}

// ---------------------------------------------------------------------------
// DELETE /api/oauth/consents/{id}
// ---------------------------------------------------------------------------

/// Revoke a specific consent by UUID.
///
/// Enforces ownership — a cross-user attempt returns 403.
/// Returns 404 when the consent does not exist or `oauth.enabled = false`.
/// Returns 204 on success.
#[utoipa::path(
    delete,
    path = "/api/oauth/consents/{id}",
    params(("id" = Uuid, Path, description = "Consent UUID")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Consent belongs to a different user"),
        (status = 404, description = "Consent not found or OAuth disabled"),
    ),
    tag = "OAuth",
    security(("oauth2" = []), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub(crate) async fn revoke_consent(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthenticatedUser>,
    Path(id): Path<Uuid>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Result<Response, ApiError> {
    if !state.oauth.enabled {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&auth_user, api_token_id);

    // Pre-flight ownership check — gives a clean 403 before delegating to the service.
    let row = match oauth_consent::Entity::find_by_id(id).one(state.db()).await {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(StatusCode::NOT_FOUND.into_response()),
        Err(e) => {
            tracing::error!(error = %e, "consent lookup failed");
            return Ok(oauth_500());
        }
    };

    if row.user_id != auth_user.user_id {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    // Delegate to the service — it enforces ownership internally and cascades to refresh tokens.
    let svc = OAuthConsentService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    svc.revoke(id, auth_user.user_id).await?;

    match AuditEntry::<Event>::builder_event(AuditActionType::OAUTH_CONSENT_REVOKE)
        .tenant_scope(state.default_tenant_id)
        .actor(actor_type, actor_id)
        .target("oauth_consent", id.to_string(), None)
        .outcome(AuditOutcome::Success)
        .build()
    {
        Ok(entry) => state.audit_emitter.emit_event(entry),
        Err(err) => {
            tracing::warn!(error = %err, "dropping invalid oauth.consent_revoke audit entry")
        }
    }

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
    use http_body_util::BodyExt;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::{oauth_client, oauth_consent, user};
    use uptrakit_shared_types::MaskedEmail;

    use crate::oauth::OAuthState;
    use crate::oauth::canonical_url::CanonicalUrlConfig;
    use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
    use crate::router::build_router;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    // -----------------------------------------------------------------------
    // OAuth state builder
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

    // -----------------------------------------------------------------------
    // Test app setup
    // -----------------------------------------------------------------------

    struct ConsentsApiTestApp {
        router: axum::Router,
        db: sea_orm::DatabaseConnection,
        jwt: Arc<crate::auth::jwt::JwtManager>,
    }

    async fn setup_with_oauth(oauth: OAuthState) -> ConsentsApiTestApp {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = oauth;
        let router = build_router(Arc::new(patched));
        ConsentsApiTestApp { router, db, jwt }
    }

    // -----------------------------------------------------------------------
    // DB fixture helpers
    // -----------------------------------------------------------------------

    async fn insert_test_user(db: &sea_orm::DatabaseConnection) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(id),
            email: Set(MaskedEmail::new(format!(
                "consents-api-test-{id}@example.com"
            ))),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert test user");
        id
    }

    async fn insert_oauth_client_row(db: &sea_orm::DatabaseConnection) -> String {
        let now = OffsetDateTime::now_utc();
        let client_id = format!("test-client-{}", uuid::Uuid::now_v7());
        oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set("Test Client".to_string()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set("https://example.com/callback".to_string()),
            default_scope: Set("openid mcp:read".to_string()),
            grant_types: Set("authorization_code refresh_token".to_string()),
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

    async fn insert_consent_row(
        db: &sea_orm::DatabaseConnection,
        user_id: uuid::Uuid,
        client_id: &str,
        revoked: bool,
        granted_at: OffsetDateTime,
    ) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        oauth_consent::ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            client_id: Set(client_id.to_string()),
            scopes: Set("openid mcp:read".to_string()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(None),
            granted_at: Set(granted_at),
            revoked_at: Set(if revoked { Some(granted_at) } else { None }),
        }
        .insert(db)
        .await
        .expect("insert oauth_consent");
        id
    }

    // -----------------------------------------------------------------------
    // Test 1 — list_consents returns only the requesting user's consents
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_consents_returns_user_consents() {
        let app = setup_with_oauth(enabled_oauth_state()).await;
        let user_a = insert_test_user(&app.db).await;
        let user_b = insert_test_user(&app.db).await;
        let client_id = insert_oauth_client_row(&app.db).await;

        // Insert 2 consents for user A and 1 for user B.
        // Each needs a distinct client_id due to the partial unique index
        // on active (user_id, client_id). Use separate clients for the two
        // user-A rows.
        let client_id2 = insert_oauth_client_row(&app.db).await;
        insert_consent_row(
            &app.db,
            user_a,
            &client_id,
            false,
            OffsetDateTime::now_utc(),
        )
        .await;
        insert_consent_row(
            &app.db,
            user_a,
            &client_id2,
            false,
            OffsetDateTime::now_utc(),
        )
        .await;
        insert_consent_row(
            &app.db,
            user_b,
            &client_id,
            false,
            OffsetDateTime::now_utc(),
        )
        .await;

        let token = app
            .jwt
            .create_access_token(user_a, "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("GET")
            .uri("/api/oauth/consents")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::OK);

        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let items: serde_json::Value = serde_json::from_slice(&body_bytes).expect("parse json");
        let arr = items.as_array().expect("response must be array");

        assert_eq!(arr.len(), 2, "must return exactly user A's 2 consents");
        for item in arr {
            assert!(item["id"].is_string(), "id must be present");
            assert!(item["client_id"].is_string(), "client_id must be present");
            assert!(item["scopes"].is_string(), "scopes must be present");
            assert!(!item["granted_at"].is_null(), "granted_at must be present");
        }
    }

    // -----------------------------------------------------------------------
    // Test 2 — list_consents excludes revoked consents
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_consents_excludes_revoked() {
        let app = setup_with_oauth(enabled_oauth_state()).await;
        let user_id = insert_test_user(&app.db).await;
        // Use two distinct clients (partial-unique index on active consents).
        let client_id1 = insert_oauth_client_row(&app.db).await;
        let client_id2 = insert_oauth_client_row(&app.db).await;

        insert_consent_row(
            &app.db,
            user_id,
            &client_id1,
            false,
            OffsetDateTime::now_utc(),
        )
        .await;
        insert_consent_row(
            &app.db,
            user_id,
            &client_id2,
            true,
            OffsetDateTime::now_utc(),
        )
        .await;

        let token = app
            .jwt
            .create_access_token(user_id, "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("GET")
            .uri("/api/oauth/consents")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::OK);

        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let items: serde_json::Value = serde_json::from_slice(&body_bytes).expect("parse json");
        let arr = items.as_array().expect("response must be array");
        assert_eq!(arr.len(), 1, "revoked consent must be excluded");
    }

    // -----------------------------------------------------------------------
    // Test 3 — revoke_consent returns 204 and sets revoked_at
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn revoke_consent_returns_204() {
        let app = setup_with_oauth(enabled_oauth_state()).await;
        let user_id = insert_test_user(&app.db).await;
        let client_id = insert_oauth_client_row(&app.db).await;
        let consent_id = insert_consent_row(
            &app.db,
            user_id,
            &client_id,
            false,
            OffsetDateTime::now_utc(),
        )
        .await;

        let token = app
            .jwt
            .create_access_token(user_id, "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/oauth/consents/{consent_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NO_CONTENT);

        // Verify revoked_at is set in the DB.
        let row = oauth_consent::Entity::find_by_id(consent_id)
            .one(&app.db)
            .await
            .expect("db query")
            .expect("row must exist");
        assert!(
            row.revoked_at.is_some(),
            "revoked_at must be set after DELETE"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4 — revoke_consent cross-user returns 403
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn revoke_consent_cross_user_returns_403() {
        let app = setup_with_oauth(enabled_oauth_state()).await;
        let user_a = insert_test_user(&app.db).await;
        let user_b = insert_test_user(&app.db).await;
        let client_id = insert_oauth_client_row(&app.db).await;
        let consent_id = insert_consent_row(
            &app.db,
            user_a,
            &client_id,
            false,
            OffsetDateTime::now_utc(),
        )
        .await;

        // User B tries to revoke user A's consent.
        let token_b = app
            .jwt
            .create_access_token(user_b, "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/oauth/consents/{consent_id}"))
            .header("authorization", format!("Bearer {token_b}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);

        // Consent must remain active.
        let row = oauth_consent::Entity::find_by_id(consent_id)
            .one(&app.db)
            .await
            .expect("db query")
            .expect("row must exist");
        assert!(row.revoked_at.is_none(), "consent must not be revoked");
    }

    // -----------------------------------------------------------------------
    // Test 5 — revoke nonexistent consent returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn revoke_nonexistent_consent_returns_404() {
        let app = setup_with_oauth(enabled_oauth_state()).await;
        let user_id = insert_test_user(&app.db).await;

        let token = app
            .jwt
            .create_access_token(user_id, "password", None, None)
            .expect("create_access_token");

        let random_id = uuid::Uuid::new_v4();

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/oauth/consents/{random_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Test 6 — oauth disabled returns 404 for both endpoints
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn oauth_disabled_returns_404() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db, tenant_id).await;
        // Default test state has oauth.enabled = false.
        assert!(
            !state.oauth.enabled,
            "oauth must be disabled in default test state"
        );

        let user_id = uuid::Uuid::nil();
        let token = jwt
            .create_access_token(user_id, "password", None, None)
            .expect("create access token");

        let router = build_router(Arc::clone(&state));

        // GET /api/oauth/consents must return 404.
        let req = Request::builder()
            .method("GET")
            .uri("/api/oauth/consents")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let resp = router.clone().oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

        // DELETE /api/oauth/consents/{id} must return 404.
        let dummy_id = uuid::Uuid::nil();
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/oauth/consents/{dummy_id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build request");
        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Test 7 — list_consents is enriched with client_name, RFC 3339, newest first
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_consents_enriched_with_client_name_rfc3339_ordered() {
        let app = setup_with_oauth(enabled_oauth_state()).await;
        let user_id = insert_test_user(&app.db).await;
        let client_id1 = insert_oauth_client_row(&app.db).await;
        let client_id2 = insert_oauth_client_row(&app.db).await;

        let older = OffsetDateTime::now_utc() - time::Duration::hours(2);
        let newer = OffsetDateTime::now_utc();
        insert_consent_row(&app.db, user_id, &client_id1, false, older).await;
        let newest_id = insert_consent_row(&app.db, user_id, &client_id2, false, newer).await;

        let token = app
            .jwt
            .create_access_token(user_id, "password", None, None)
            .expect("create_access_token");

        let client = crate::test_harness::http_client::TestClient::new(app.router);
        let (status, items): (http::StatusCode, Vec<serde_json::Value>) = client
            .get("/api/oauth/consents")
            .bearer(&token)
            .send_json()
            .await;

        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(items.len(), 2);

        let first = items.first().expect("first item");
        // Newest-granted first.
        assert_eq!(
            first["id"].as_str().expect("id string"),
            newest_id.to_string(),
            "list must be ordered granted_at DESC"
        );
        // Enrichment: client_name comes from the joined oauth_client row.
        assert_eq!(
            first["client_name"].as_str().expect("client_name string"),
            "Test Client",
            "client_name must be populated from the join"
        );
        // RFC 3339 pin — the old json! path emitted a component array.
        let granted_at = first["granted_at"].as_str().expect("granted_at string");
        time::OffsetDateTime::parse(granted_at, &time::format_description::well_known::Rfc3339)
            .expect("granted_at parses as RFC 3339");
    }
}
