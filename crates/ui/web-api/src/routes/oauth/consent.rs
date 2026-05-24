//! OAuth 2.1 consent endpoint handlers — `GET /oauth/consent/{request_id}`,
//! `POST /oauth/consent/{request_id}/approve`, and
//! `POST /oauth/consent/{request_id}/deny`.
//!
//! Per spec §12.

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uptrakit_audit_log::{AuditActorType, AuditEntry, AuditOutcome, Event};
use uptrakit_shared_db::entity::{oauth_authorization_request, oauth_consent};
use uptrakit_web_api_types::oauth::ConsentDecision;
use uuid::Uuid;

use crate::AppState;
use crate::extract::ClientIp;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::oauth::http_responses::{oauth_400, oauth_500};
use crate::oauth::rate_limit::EndpointKind;
use crate::oauth::services::authorization_code::{
    MintAuthorizationCode, OAuthAuthorizationCodeService,
};
use crate::oauth::services::authorization_request::OAuthAuthorizationRequestService;
use crate::oauth::services::client::OAuthClientService;
use crate::oauth::services::consent::OAuthConsentService;
use crate::routes::oauth::helpers::{percent_encode, require_auth_and_rate_limit};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Returns `"localhost"` for loopback addresses, otherwise the lowercase host
/// parsed from `redirect_uri`. An unparseable URI returns an empty host, which
/// cannot match any typed-confirmation value and will always fail the
/// confirmation check.
fn loopback_or_host(redirect_uri: &str) -> String {
    let host = url::Url::parse(redirect_uri)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default();
    if host == "localhost" || host == "127.0.0.1" || host == "[::1]" {
        "localhost".to_string()
    } else {
        host
    }
}

// ---------------------------------------------------------------------------
// GET /oauth/consent/{request_id}
// ---------------------------------------------------------------------------

/// Return client and scope details for a pending authorization request.
///
/// The consent UI fetches this to render the approval prompt. Returns 404 when
/// OAuth is disabled, the request does not exist, is expired, or has already
/// been consumed.
#[utoipa::path(
    get,
    path = "/oauth/consent/{request_id}",
    params(
        ("request_id" = Uuid, Path, description = "Authorization request UUID"),
    ),
    responses(
        (status = 200, description = "Consent details"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Wrong user"),
        (status = 404, description = "Request not found or OAuth disabled"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn consent_details(
    State(state): State<Arc<AppState>>,
    client_ip: Option<Extension<ClientIp>>,
    auth_user: Option<Extension<AuthenticatedUser>>,
    Path(request_id): Path<Uuid>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (auth_user, _ip_str) =
        match require_auth_and_rate_limit(auth_user, &client_ip, &state, EndpointKind::Consent)
            .await
        {
            Ok(v) => v,
            Err(r) => return r,
        };

    let row = match oauth_authorization_request::Entity::find_by_id(request_id)
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch oauth_authorization_request");
            return oauth_500();
        }
    };

    let now = (state.oauth.clock)();
    if row.consumed_at.is_some() || row.expires_at < now {
        return StatusCode::NOT_FOUND.into_response();
    }

    if row.user_id != auth_user.user_id {
        return StatusCode::FORBIDDEN.into_response();
    }

    let client_svc = OAuthClientService::new(
        state.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );
    let client = match client_svc.lookup(&row.client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "oauth client lookup failed");
            return oauth_500();
        }
    };

    let existing_consent = match oauth_consent::Entity::find()
        .filter(oauth_consent::Column::UserId.eq(row.user_id))
        .filter(oauth_consent::Column::ClientId.eq(row.client_id.as_str()))
        .filter(oauth_consent::Column::RevokedAt.is_null())
        .one(state.db())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch oauth_consent for revalidation check");
            return oauth_500();
        }
    };
    let revalidation_required = existing_consent
        .as_ref()
        .map(|c| c.revalidation_required_at.is_some())
        .unwrap_or(false);

    let redirect_hostname = url::Url::parse(&row.redirect_uri)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_else(|| "unknown".to_string());

    let typed_confirmation_value = loopback_or_host(&row.redirect_uri);

    let scopes: Vec<&str> = row.scope.split_whitespace().collect();
    let requires_typed_confirmation = client.trusted_at.is_none();

    let mut body = serde_json::json!({
        "client_id": client.id,
        "client_name": client.client_name,
        "client_uri": client.client_uri,
        "scopes": scopes,
        "redirect_uri_host": redirect_hostname,
        "requires_typed_confirmation": requires_typed_confirmation,
        "typed_confirmation_value": typed_confirmation_value,
        "revalidation_required": revalidation_required,
    });
    if revalidation_required && let Some(map) = body.as_object_mut() {
        map.insert("metadata_change_diff".to_string(), serde_json::Value::Null);
    }

    (StatusCode::OK, axum::Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// POST /oauth/consent/{request_id}/approve
// ---------------------------------------------------------------------------

/// Approve a pending authorization request and redirect the user to the client
/// with an authorization code.
#[utoipa::path(
    post,
    path = "/oauth/consent/{request_id}/approve",
    params(
        ("request_id" = Uuid, Path, description = "Authorization request UUID"),
    ),
    responses(
        (status = 200, description = "JSON with redirect_to URL for the client app to navigate to"),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Wrong user"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn approve_consent(
    State(state): State<Arc<AppState>>,
    client_ip: Option<Extension<ClientIp>>,
    auth_user: Option<Extension<AuthenticatedUser>>,
    Path(request_id): Path<Uuid>,
    axum::Json(_body): axum::Json<ConsentDecision>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (auth_user, _ip_str) =
        match require_auth_and_rate_limit(auth_user, &client_ip, &state, EndpointKind::Consent)
            .await
        {
            Ok(v) => v,
            Err(r) => return r,
        };

    let preflight = match oauth_authorization_request::Entity::find_by_id(request_id)
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return oauth_400(
                "invalid_request",
                "authorization request expired or already used",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "preflight fetch failed");
            return oauth_500();
        }
    };
    if preflight.user_id != auth_user.user_id {
        return StatusCode::FORBIDDEN.into_response();
    }

    let ar_svc =
        OAuthAuthorizationRequestService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    let row = match ar_svc.consume(request_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return oauth_400(
                "invalid_request",
                "authorization request expired or already used",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to consume authorization request");
            return oauth_500();
        }
    };

    // Defensive ownership check: row.user_id must match since preflight verified it.
    if row.user_id != auth_user.user_id {
        return StatusCode::FORBIDDEN.into_response();
    }

    let consent_svc = OAuthConsentService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    match consent_svc
        .grant(row.user_id, &row.client_id, &row.scope, None)
        .await
    {
        Ok(_) => {}
        Err(e) => {
            tracing::error!(error = %e, "failed to grant consent");
            return oauth_500();
        }
    };

    let code_svc =
        OAuthAuthorizationCodeService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    let code = match code_svc
        .mint(MintAuthorizationCode {
            request_id: row.request_id,
            client_id: row.client_id.clone(),
            user_id: row.user_id,
            redirect_uri: row.redirect_uri.clone(),
            scope: row.scope.clone(),
            code_challenge: row.code_challenge.clone(),
            code_challenge_method: row.code_challenge_method.clone(),
            resource: row.resource.clone(),
        })
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to mint authorization code");
            return oauth_500();
        }
    };

    emit_consent_audit(
        &state,
        uptrakit_audit_log::AuditActionType::OAUTH_CONSENT_GRANT,
        AuditOutcome::Success,
        row.user_id,
        &row.client_id,
    );

    let sep = if row.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    let redirect_to = format!(
        "{}{}code={}&state={}",
        row.redirect_uri,
        sep,
        percent_encode(code.as_str()),
        percent_encode(&row.state),
    );
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "redirect_to": redirect_to })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// POST /oauth/consent/{request_id}/deny
// ---------------------------------------------------------------------------

/// Deny a pending authorization request and redirect the user to the client
/// with an `access_denied` error.
#[utoipa::path(
    post,
    path = "/oauth/consent/{request_id}/deny",
    params(
        ("request_id" = Uuid, Path, description = "Authorization request UUID"),
    ),
    responses(
        (status = 200, description = "JSON with redirect_to URL for the client app to navigate to"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Wrong user"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn deny_consent(
    State(state): State<Arc<AppState>>,
    client_ip: Option<Extension<ClientIp>>,
    auth_user: Option<Extension<AuthenticatedUser>>,
    Path(request_id): Path<Uuid>,
) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (auth_user, _ip_str) =
        match require_auth_and_rate_limit(auth_user, &client_ip, &state, EndpointKind::Consent)
            .await
        {
            Ok(v) => v,
            Err(r) => return r,
        };

    let ar_svc =
        OAuthAuthorizationRequestService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    let row = match ar_svc.consume(request_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            // Request expired or already consumed — still safe to return a deny
            // redirect if we can find the original redirect_uri. Since we cannot
            // reconstruct it, return a generic error. Per spec we still emit
            // the audit entry below via the None branch.
            return StatusCode::GONE.into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to consume authorization request on deny");
            return oauth_500();
        }
    };

    if row.user_id != auth_user.user_id {
        return StatusCode::FORBIDDEN.into_response();
    }

    emit_consent_audit(
        &state,
        uptrakit_audit_log::AuditActionType::OAUTH_CONSENT_DENY,
        AuditOutcome::Denied,
        row.user_id,
        &row.client_id,
    );

    let sep = if row.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    let redirect_to = format!(
        "{}{}error=access_denied&state={}",
        row.redirect_uri,
        sep,
        percent_encode(&row.state),
    );
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "redirect_to": redirect_to })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Internal audit helper
// ---------------------------------------------------------------------------

fn emit_consent_audit(
    state: &AppState,
    action: uptrakit_audit_log::RegisteredAuditAction,
    outcome: AuditOutcome,
    user_id: Uuid,
    client_id: &str,
) {
    let entry = match AuditEntry::<Event>::builder(action)
        .tenant_scope(state.default_tenant_id)
        .actor(AuditActorType::User, Some(user_id))
        .outcome(outcome)
        .details(serde_json::json!({ "client_id": client_id }))
        .build()
    {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(error = %err, "dropping invalid consent audit entry");
            return;
        }
    };
    state.audit_emitter.emit_event(entry);
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
    use sea_orm::{ActiveModelTrait, Set};
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::{oauth_authorization_request, oauth_client, user};
    use uptrakit_shared_types::MaskedEmail;

    use crate::oauth::OAuthState;
    use crate::oauth::canonical_url::CanonicalUrlConfig;
    use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
    use crate::router::build_router;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    const TEST_CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const TEST_REDIRECT_URI: &str = "https://example.com/callback";
    const TEST_RESOURCE: &str = "https://controller.example.com";

    // -----------------------------------------------------------------------
    // OAuth state builder
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

    // -----------------------------------------------------------------------
    // Optional auth middleware (mirrors authorize.rs tests)
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Shared test setup
    // -----------------------------------------------------------------------

    struct ConsentTestApp {
        #[expect(dead_code, reason = "retained for test fixture completeness")]
        state: Arc<crate::AppState>,
        router: axum::Router,
        db: sea_orm::DatabaseConnection,
        jwt: Arc<crate::auth::jwt::JwtManager>,
        #[expect(dead_code, reason = "retained for FK constraint satisfaction")]
        tenant_id: uuid::Uuid,
    }

    async fn setup() -> ConsentTestApp {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = enabled_oauth_state(false, false);
        let state = Arc::new(patched);
        let router = build_router(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            optional_auth_middleware,
        ));
        ConsentTestApp {
            state,
            router,
            db,
            jwt,
            tenant_id,
        }
    }

    // -----------------------------------------------------------------------
    // DB fixture helpers
    // -----------------------------------------------------------------------

    async fn insert_test_user(db: &sea_orm::DatabaseConnection) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(id),
            email: Set(MaskedEmail::new(format!("consent-test-{id}@example.com"))),
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

    /// Insert an `oauth_client` row and return its `client_id`.
    ///
    /// When `trusted` is `true`, `trusted_at` is set to the current timestamp.
    async fn insert_oauth_client(
        db: &sea_orm::DatabaseConnection,
        redirect_uri: &str,
        trusted: bool,
    ) -> String {
        let now = OffsetDateTime::now_utc();
        let client_id = format!("test-consent-client-{}", uuid::Uuid::now_v7());
        let redirect_uris_json =
            serde_json::to_string(&vec![redirect_uri]).expect("serialize redirect_uris");

        oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set("Consent Test Client".to_string()),
            client_uri: Set(Some("https://example.com".to_string())),
            logo_uri: Set(None),
            redirect_uris: Set(redirect_uris_json),
            default_scope: Set("mcp:read".to_string()),
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
            trusted_at: Set(if trusted { Some(now) } else { None }),
        }
        .insert(db)
        .await
        .expect("insert oauth_client");

        client_id
    }

    /// Insert an `oauth_authorization_request` row and return its `request_id`.
    async fn insert_auth_request(
        db: &sea_orm::DatabaseConnection,
        client_id: &str,
        user_id: uuid::Uuid,
        redirect_uri: &str,
    ) -> uuid::Uuid {
        let now = OffsetDateTime::now_utc();
        let request_id = uuid::Uuid::now_v7();
        oauth_authorization_request::ActiveModel {
            request_id: Set(request_id),
            client_id: Set(client_id.to_string()),
            user_id: Set(user_id),
            redirect_uri: Set(redirect_uri.to_string()),
            scope: Set("mcp:read".to_string()),
            state: Set("test-state-xyz".to_string()),
            code_challenge: Set(TEST_CODE_CHALLENGE.to_string()),
            code_challenge_method: Set("S256".to_string()),
            resource: Set(TEST_RESOURCE.to_string()),
            created_at: Set(now),
            expires_at: Set(now + time::Duration::seconds(600)),
            consumed_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_authorization_request");
        request_id
    }

    // -----------------------------------------------------------------------
    // Test 1 — GET returns client info
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn consent_details_returns_client_info() {
        let app = setup().await;
        let user_id = insert_test_user(&app.db).await;
        let client_id = insert_oauth_client(&app.db, TEST_REDIRECT_URI, false).await;
        let request_id = insert_auth_request(&app.db, &client_id, user_id, TEST_REDIRECT_URI).await;

        let jwt_token = app
            .jwt
            .create_access_token(user_id, &[], "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("GET")
            .uri(format!("/oauth/consent/{request_id}"))
            .header("authorization", format!("Bearer {jwt_token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");

        assert_eq!(body["client_name"], "Consent Test Client");
        assert_eq!(body["requires_typed_confirmation"], true);
        assert_eq!(body["typed_confirmation_value"], "example.com");
        assert!(body["scopes"].is_array());
    }

    // -----------------------------------------------------------------------
    // Test 2 — GET returns 403 when wrong user
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn consent_details_wrong_user_returns_403() {
        let app = setup().await;
        let owner_id = insert_test_user(&app.db).await;
        let other_id = insert_test_user(&app.db).await;
        let client_id = insert_oauth_client(&app.db, TEST_REDIRECT_URI, false).await;
        let request_id =
            insert_auth_request(&app.db, &client_id, owner_id, TEST_REDIRECT_URI).await;

        let other_jwt = app
            .jwt
            .create_access_token(other_id, &[], "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("GET")
            .uri(format!("/oauth/consent/{request_id}"))
            .header("authorization", format!("Bearer {other_jwt}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // Test 3 — GET without auth returns 401
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn consent_details_unauthenticated_returns_401() {
        let app = setup().await;
        let user_id = insert_test_user(&app.db).await;
        let client_id = insert_oauth_client(&app.db, TEST_REDIRECT_URI, false).await;
        let request_id = insert_auth_request(&app.db, &client_id, user_id, TEST_REDIRECT_URI).await;

        let req = Request::builder()
            .method("GET")
            .uri(format!("/oauth/consent/{request_id}"))
            // No Authorization header.
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // Test 4 — POST approve redirects with code
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn consent_approve_redirects_with_code() {
        let app = setup().await;
        let user_id = insert_test_user(&app.db).await;
        // Trusted client: no typed confirmation required.
        let client_id = insert_oauth_client(&app.db, TEST_REDIRECT_URI, true).await;
        let request_id = insert_auth_request(&app.db, &client_id, user_id, TEST_REDIRECT_URI).await;

        let jwt_token = app
            .jwt
            .create_access_token(user_id, &[], "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("POST")
            .uri(format!("/oauth/consent/{request_id}/approve"))
            .header("authorization", format!("Bearer {jwt_token}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
        let redirect_to = body["redirect_to"].as_str().expect("redirect_to string");
        assert!(
            redirect_to.starts_with(TEST_REDIRECT_URI),
            "redirect_to must point to registered redirect_uri, got: {redirect_to}"
        );
        assert!(
            redirect_to.contains("code="),
            "redirect_to must contain authorization code, got: {redirect_to}"
        );
        assert!(
            redirect_to.contains("state="),
            "redirect_to must include state param, got: {redirect_to}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5 — POST deny redirects with access_denied
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn consent_deny_redirects_with_error() {
        let app = setup().await;
        let user_id = insert_test_user(&app.db).await;
        let client_id = insert_oauth_client(&app.db, TEST_REDIRECT_URI, false).await;
        let request_id = insert_auth_request(&app.db, &client_id, user_id, TEST_REDIRECT_URI).await;

        let jwt_token = app
            .jwt
            .create_access_token(user_id, &[], "password", None, None)
            .expect("create_access_token");

        let req = Request::builder()
            .method("POST")
            .uri(format!("/oauth/consent/{request_id}/deny"))
            .header("authorization", format!("Bearer {jwt_token}"))
            .body(Body::empty())
            .expect("build request");

        let resp = app.router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
        let redirect_to = body["redirect_to"].as_str().expect("redirect_to string");
        assert!(
            redirect_to.contains("error=access_denied"),
            "redirect_to must contain access_denied, got: {redirect_to}"
        );
        assert!(
            redirect_to.starts_with(TEST_REDIRECT_URI),
            "redirect_to must point to registered redirect_uri, got: {redirect_to}"
        );
    }

    // -----------------------------------------------------------------------
    // Test 7 — OAuth disabled returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn consent_oauth_disabled_returns_404() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db, tenant_id).await;
        // oauth.enabled is false by default.
        assert!(!state.oauth.enabled);
        let router = build_router(Arc::clone(&state));

        let req = Request::builder()
            .method("GET")
            .uri("/oauth/consent/00000000-0000-0000-0000-000000000000")
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }
}
