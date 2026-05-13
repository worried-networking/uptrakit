//! RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint.
//!
//! The MCP OAuth 2.1 `POST /oauth/token` handler lives in [`mcp_token`].
//! It is wired to the router in Task 19 (TODO: route wiring — Task 19).

use std::sync::Arc;

use axum::Extension;
use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{oauth_consent, user_role};
use uptrakit_web_api_auth::auth::device_flow::{PollOutcome, validate_client_id};
use uptrakit_web_api_auth::auth::rate_limit::RateLimitStore;
use uptrakit_web_api_types::oauth::{
    McpAccessTokenClaims, OAuthErrorCode, OAuthTokenRequest, OAuthTokenResponse, TokenRequest,
    TokenResponse,
};
use uptrakit_web_api_types::validation::Validate;

use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome};

use crate::AppState;
use crate::error_response::oauth_error_response;
use crate::extract::ClientIp;
use crate::oauth::http_responses::{oauth_400, oauth_500};
use crate::oauth::rate_limit::{EndpointKind, OAuthRateLimiter, check_rate_limit};
use crate::oauth::services::authorization_code::{
    OAuthAuthorizationCodeService, code_error_to_response,
};
use crate::oauth::services::client::OAuthClientService;
use crate::oauth::services::refresh_token::{OAuthRefreshTokenService, refresh_error_to_response};

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const ACCESS_TOKEN_TTL_SECS: i64 = 900;

/// RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint.
#[utoipa::path(
    post,
    path = "/api/v1/oauth/token",
    request_body(
        content = OAuthTokenRequest,
        content_type = "application/x-www-form-urlencoded"
    ),
    responses(
        (status = 200, description = "Token granted", body = OAuthTokenResponse),
        (status = 400, description = "OAuth error per RFC 6749 §5.2 / RFC 8628 §3.5")
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn token(
    State(state): State<Arc<AppState>>,
    Form(req): Form<OAuthTokenRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::InvalidRequest,
            Some(e.to_string()),
            None,
        );
    }

    match req.grant_type.as_str() {
        DEVICE_CODE_GRANT => device_code_grant(state, req).await,
        _ => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::UnsupportedGrantType,
            None,
            None,
        ),
    }
}

async fn device_code_grant(state: Arc<AppState>, req: OAuthTokenRequest) -> Response {
    let device_code = match req.device_code.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                OAuthErrorCode::InvalidRequest,
                Some("device_code is required".into()),
                None,
            );
        }
    };

    if let Some(client_id) = req.client_id.as_deref()
        && let Err(code) = validate_client_id(client_id)
    {
        return oauth_error_response(StatusCode::BAD_REQUEST, code, None, None);
    }

    let outcome = match state
        .auth
        .device_flow_store
        .poll(&device_code, OffsetDateTime::now_utc())
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("device flow poll failed: {e}");
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                OAuthErrorCode::ServerError,
                Some("internal error".into()),
                None,
            );
        }
    };

    emit_poll_audit(&state, &device_code, &outcome);

    match outcome {
        PollOutcome::Authorized { token, .. } => {
            let body = OAuthTokenResponse::new(token.expose_secret().to_string(), "Bearer".into());
            (StatusCode::OK, Json(body)).into_response()
        }
        PollOutcome::Pending => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::AuthorizationPending,
            None,
            None,
        ),
        PollOutcome::SlowDown { bumped_interval } => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::SlowDown,
            None,
            Some(bumped_interval),
        ),
        PollOutcome::Denied => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::AccessDenied,
            None,
            None,
        ),
        PollOutcome::Expired | PollOutcome::Unknown => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::ExpiredToken,
            None,
            None,
        ),
        PollOutcome::MalformedDeviceCode => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::InvalidGrant,
            None,
            None,
        ),
        _ => {
            tracing::warn!(
                "unhandled PollOutcome variant returned by device_flow_store.poll(); \
                 treating as server_error"
            );
            oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                OAuthErrorCode::ServerError,
                None,
                None,
            )
        }
    }
}

fn emit_poll_audit(state: &AppState, device_code: &str, outcome: &PollOutcome) {
    use uptrakit_audit_log::AuditOutcome as Outcome;

    let device_flow_id = crate::auth::token::hash_token(device_code);

    let (audit_outcome, details) = match outcome {
        PollOutcome::Authorized { .. } => (Outcome::Success, serde_json::json!({})),
        PollOutcome::SlowDown { bumped_interval } => (
            Outcome::Failed,
            serde_json::json!({
                "slow_down": true,
                "bumped_interval": bumped_interval,
            }),
        ),
        PollOutcome::Denied => (
            Outcome::Failed,
            serde_json::json!({ "reason_code": "access_denied" }),
        ),
        PollOutcome::Expired | PollOutcome::Unknown => (
            Outcome::Failed,
            serde_json::json!({ "reason_code": "expired_token" }),
        ),
        PollOutcome::MalformedDeviceCode => (
            Outcome::Failed,
            serde_json::json!({ "reason_code": "invalid_grant" }),
        ),
        _ => (Outcome::Failed, serde_json::json!({})),
    };

    let builder = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL,
    )
    .tenant_scope(state.default_tenant_id)
    .actor_system()
    .target("device_flow", device_flow_id, None)
    .outcome(audit_outcome)
    .details(details);

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

// ---------------------------------------------------------------------------
// MCP OAuth 2.1 token endpoint — `POST /oauth/token`
// Routing is wired in Task 19.
// ---------------------------------------------------------------------------

/// MCP OAuth 2.1 token endpoint per spec §10.3 (authorization-code exchange).
///
/// Returns 404 when `oauth.mcp_enabled = false`.
///
/// Current grant support:
/// - `authorization_code` — fully implemented (Task 13).
/// - `refresh_token` — fully implemented (Task 14).
#[utoipa::path(
    post,
    path = "/oauth/token",
    request_body(content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Token response"),
        (status = 400, description = "OAuth error"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn mcp_token(
    State(state): State<Arc<AppState>>,
    client_ip: Option<Extension<ClientIp>>,
    Form(req): Form<TokenRequest>,
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
    if let Some(r) = check_rate_limit(EndpointKind::Token, &limiter, &ip_str).await {
        return r;
    }

    // Step 2 — validate the request.
    if let Err(e) = req.validate() {
        return oauth_400("invalid_request", &e.to_string());
    }

    // Step 3 — dispatch on grant_type.
    match req {
        TokenRequest::AuthorizationCode {
            code,
            redirect_uri,
            client_id,
            code_verifier,
            resource,
        } => {
            authorization_code_grant(
                state,
                code,
                redirect_uri,
                client_id,
                code_verifier,
                resource,
            )
            .await
        }
        TokenRequest::RefreshToken {
            refresh_token,
            client_id,
            scope,
            resource,
        } => refresh_token_grant(state, resource, refresh_token, client_id, scope).await,
        _ => {
            tracing::warn!("unhandled TokenRequest variant; returning unsupported_grant_type");
            oauth_400("unsupported_grant_type", "grant type not supported")
        }
    }
}

/// Handle `grant_type=refresh_token` — spec §10.3 full rotation algorithm.
///
/// Delegates the full rotation algorithm (replay detection, scope subset check,
/// consent check, access JWT mint, cascade revoke on replay) to
/// [`OAuthRefreshTokenService::rotate`]. Audit emission is performed inside the
/// service; no additional audit calls are needed here.
async fn refresh_token_grant(
    state: Arc<AppState>,
    resource: String,
    refresh_token_str: String,
    client_id: String,
    scope_opt: Option<String>,
) -> Response {
    // Step 1 — validate resource indicator.
    if !state.oauth.canonical.accepts_audience(&resource) {
        return oauth_400("invalid_target", "resource indicator not accepted");
    }

    // Step 2 — client lookup + revocation check.
    let client_svc = OAuthClientService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );
    match client_svc.lookup(&client_id).await {
        Ok(Some(client)) => {
            if client.revoked_at.is_some() {
                return oauth_400("invalid_client", "client revoked");
            }
        }
        Ok(None) => return oauth_400("invalid_client", "unknown client_id"),
        Err(e) => {
            tracing::error!(error = %e, "oauth client lookup failed during refresh_token grant");
            return oauth_500();
        }
    }

    // Step 3 — rotate via service.
    let issuer = state.oauth.canonical.issuer().as_str().to_string();
    let rt_svc = OAuthRefreshTokenService::with_defaults(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::clone(&state.oauth.signer),
        Arc::new(state.audit_emitter.clone()),
        issuer,
        resource.clone(),
    );

    let outcome = match rt_svc
        .rotate(
            &refresh_token_str,
            &client_id,
            scope_opt.as_deref(),
            &resource,
        )
        .await
    {
        Ok(o) => o,
        Err(e) => return refresh_error_to_response(&e),
    };

    // Step 4 — return token response.
    let body = TokenResponse::new(
        outcome.access_token,
        "Bearer".into(),
        outcome.expires_in,
        Some(outcome.refresh_token.as_str().to_string()),
        Some(outcome.refresh_expires_in),
        outcome.scope,
    );
    (StatusCode::OK, axum::Json(body)).into_response()
}

/// Handle `grant_type=authorization_code` — spec §10.3 steps 17-21.
async fn authorization_code_grant(
    state: Arc<AppState>,
    code: String,
    redirect_uri: String,
    client_id: String,
    code_verifier: String,
    resource: String,
) -> Response {
    // Step 4 — client lookup.
    let client_svc = OAuthClientService::new(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::new(state.audit_emitter.clone()),
    );
    match client_svc.lookup(&client_id).await {
        Ok(Some(client)) => {
            if client.revoked_at.is_some() {
                let entry = AuditEntry::builder(AuditActionType::OAUTH_TOKEN_REJECTED)
                    .tenant_scope(state.default_tenant_id)
                    .actor_system()
                    .outcome(AuditOutcome::Denied)
                    .details(serde_json::json!({
                        "reason": "client_revoked",
                        "client_id": &client_id,
                    }))
                    .build();
                if let Ok(entry) = entry {
                    state.audit_emitter.emit_best_effort(entry);
                }
                return oauth_400("invalid_client", "client has been revoked");
            }
        }
        Ok(None) => {
            let entry = AuditEntry::builder(AuditActionType::OAUTH_TOKEN_REJECTED)
                .tenant_scope(state.default_tenant_id)
                .actor_system()
                .outcome(AuditOutcome::Denied)
                .details(serde_json::json!({
                    "reason": "unknown_client",
                    "client_id": &client_id,
                }))
                .build();
            if let Ok(entry) = entry {
                state.audit_emitter.emit_best_effort(entry);
            }
            return oauth_400("invalid_client", "unknown client_id");
        }
        Err(e) => {
            tracing::error!(error = %e, "oauth client lookup failed");
            return oauth_500();
        }
    }

    // Step 5 — validate resource indicator against canonical server.
    if !state.oauth.canonical.accepts_audience(&resource) {
        let entry = AuditEntry::builder(AuditActionType::OAUTH_TOKEN_REJECTED)
            .tenant_scope(state.default_tenant_id)
            .actor_system()
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason": "invalid_target",
                "client_id": &client_id,
                "resource": &resource,
            }))
            .build();
        if let Ok(entry) = entry {
            state.audit_emitter.emit_best_effort(entry);
        }
        return oauth_400("invalid_target", "resource indicator not accepted");
    }

    // Step 6 — verify and consume authorization code.
    let code_svc =
        OAuthAuthorizationCodeService::new(state.db.db().clone(), Arc::clone(&state.oauth.clock));
    let code_row = match code_svc
        .verify_and_consume(&code, &client_id, &redirect_uri, &code_verifier, &resource)
        .await
    {
        Ok(row) => row,
        Err(e) => {
            return code_error_to_response(
                &e,
                &state.audit_emitter,
                state.default_tenant_id,
                &client_id,
            );
        }
    };

    // Step 7 — find active consent for (user_id, client_id).
    let consent_row = match oauth_consent::Entity::find()
        .filter(oauth_consent::Column::UserId.eq(code_row.user_id))
        .filter(oauth_consent::Column::ClientId.eq(&client_id))
        .filter(oauth_consent::Column::RevokedAt.is_null())
        .one(state.db.db())
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return oauth_400("invalid_grant", "no active consent");
        }
        Err(e) => {
            tracing::error!(error = %e, "DB error looking up oauth_consent");
            return oauth_500();
        }
    };
    let consent_id = consent_row.id;

    // Step 8 — resolve tenant_id for the user via user_role table.
    let tenant_id = match user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(code_row.user_id))
        .one(state.db.db())
        .await
    {
        Ok(Some(row)) => row.tenant_id,
        Ok(None) => {
            tracing::error!(user_id = %code_row.user_id, "no user_role row found for user during token exchange");
            return oauth_500();
        }
        Err(e) => {
            tracing::error!(error = %e, "DB error looking up user_role");
            return oauth_500();
        }
    };

    // Step 9 — mint refresh token.
    let issuer = state.oauth.canonical.issuer().as_str().to_string();
    let rt_svc = OAuthRefreshTokenService::with_defaults(
        state.db.db().clone(),
        Arc::clone(&state.oauth.clock),
        Arc::clone(&state.oauth.signer),
        Arc::new(state.audit_emitter.clone()),
        issuer.clone(),
        resource.clone(),
    );
    let mint = match rt_svc
        .mint(
            &client_id,
            code_row.user_id,
            consent_id,
            &code_row.scope,
            &resource,
        )
        .await
    {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "refresh token mint failed");
            return oauth_500();
        }
    };

    // Step 10 — mint access JWT.
    let now = (state.oauth.clock)();
    let jti = uuid::Uuid::now_v7().to_string();
    let iat = now.unix_timestamp();
    let exp = iat + ACCESS_TOKEN_TTL_SECS;
    let claims = McpAccessTokenClaims::new(
        issuer,
        code_row.user_id.to_string(),
        resource.clone(),
        client_id.clone(),
        code_row.scope.clone(),
        jti.clone(),
        iat,
        iat,
        exp,
        tenant_id.to_string(),
    );
    let access_token = match state.oauth.signer.mint(&claims) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "JWT signing failed");
            return oauth_500();
        }
    };

    // Step 11 — emit success audit and return token response.
    let entry = AuditEntry::builder(AuditActionType::OAUTH_TOKEN_ISSUED)
        .tenant_scope(tenant_id)
        .actor_system()
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": &client_id,
            "user_id": code_row.user_id.to_string(),
            "scope": &code_row.scope,
            "jti": &jti,
            "aud": &resource,
        }))
        .build();
    if let Ok(entry) = entry {
        state.audit_emitter.emit_best_effort(entry);
    }

    let body = TokenResponse::new(
        access_token,
        "Bearer".into(),
        ACCESS_TOKEN_TTL_SECS,
        Some(mint.refresh_token.as_str().to_string()),
        Some(mint.expires_in),
        code_row.scope.clone(),
    );
    (StatusCode::OK, Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// Tests for mcp_token
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod mcp_token_tests {
    #![expect(
        clippy::expect_used,
        reason = "test assertions — panics on setup failure are acceptable in tests"
    )]

    use std::sync::Arc;

    use axum::body::Body;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use http::Request;
    use http_body_util::BodyExt;
    use sea_orm::{ActiveModelTrait, Set};
    use sha2::{Digest, Sha256};
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::{
        oauth_authorization_code, oauth_authorization_request, oauth_client, oauth_consent, role,
        user, user_role,
    };
    use uptrakit_shared_types::MaskedEmail;
    use uptrakit_web_api_auth::auth::token::hash_token;
    use uuid::Uuid;

    use crate::oauth::OAuthState;
    use crate::oauth::canonical_url::CanonicalUrlConfig;
    use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    const TEST_CLIENT_ID: &str = "test-mcp-client";
    const TEST_REDIRECT_URI: &str = "https://example.com/callback";
    const TEST_RESOURCE: &str = "https://controller.example.com/mcp";
    const TEST_SCOPE: &str = "mcp:read";
    const TEST_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

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
                vec![TEST_RESOURCE.into()],
            )),
            clock: Arc::new(OffsetDateTime::now_utc),
            instance_id: uuid::Uuid::nil(),
            dcr_enabled: false,
            cimd_enabled: false,
        }
    }

    fn pkce_challenge(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        URL_SAFE_NO_PAD.encode(digest)
    }

    async fn app_with_oauth() -> (crate::test_harness::TestApp, axum::Router) {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, jwt) = build_test_state(db.clone(), tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = enabled_oauth_state();
        let state = Arc::new(patched);
        // Build a minimal router that mounts mcp_token at POST /oauth/token.
        // Task 19 will wire this into the main router; for now tests use their own.
        let router = axum::Router::new()
            .route("/oauth/token", axum::routing::post(super::mcp_token))
            .with_state(Arc::clone(&state));
        let app = crate::test_harness::TestApp {
            state,
            router: router.clone(),
            db,
            jwt,
            tenant_id,
        };
        (app, router)
    }

    /// Insert a minimal user + tenant + role + user_role chain and return user_id.
    async fn insert_user_with_tenant(db: &sea_orm::DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let user_id = Uuid::now_v7();

        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new(format!(
                "test-token-{user_id}@example.com"
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
        .expect("insert user");

        let role_id = Uuid::now_v7();
        role::ActiveModel {
            id: Set(role_id),
            name: Set(format!("test-role-{role_id}")),
            description: Set(None),
            is_built_in: Set(false),
            created_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert role");

        user_role::ActiveModel {
            tenant_id: Set(tenant_id),
            user_id: Set(user_id),
            role_id: Set(role_id),
            assigned_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert user_role");

        user_id
    }

    /// Insert an oauth_client row with `trusted_at` set and the given redirect_uri.
    async fn insert_trusted_client(
        db: &sea_orm::DatabaseConnection,
        client_id: &str,
        redirect_uri: &str,
    ) {
        let now = OffsetDateTime::now_utc();
        let redirect_uris_json = serde_json::to_string(&vec![redirect_uri]).expect("serialize");

        oauth_client::ActiveModel {
            id: Set(client_id.to_string()),
            client_name: Set("MCP Test Client".to_string()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set(redirect_uris_json),
            default_scope: Set(TEST_SCOPE.to_string()),
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
            trusted_at: Set(Some(now)),
        }
        .insert(db)
        .await
        .expect("insert oauth_client");
    }

    /// Insert an oauth_consent row for (user_id, client_id) and return the consent id.
    async fn insert_consent(
        db: &sea_orm::DatabaseConnection,
        user_id: Uuid,
        client_id: &str,
    ) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        oauth_consent::ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            client_id: Set(client_id.to_string()),
            scopes: Set(TEST_SCOPE.to_string()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(None),
            granted_at: Set(now),
            revoked_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_consent");
        id
    }

    /// Insert an authorization_request row (required by FK on authorization_code).
    async fn insert_auth_request(
        db: &sea_orm::DatabaseConnection,
        client_id: &str,
        user_id: Uuid,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let request_id = Uuid::now_v7();

        oauth_authorization_request::ActiveModel {
            request_id: Set(request_id),
            client_id: Set(client_id.to_string()),
            user_id: Set(user_id),
            redirect_uri: Set(TEST_REDIRECT_URI.to_string()),
            scope: Set(TEST_SCOPE.to_string()),
            state: Set("test-state".to_string()),
            code_challenge: Set(pkce_challenge(TEST_VERIFIER)),
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

    /// Insert a valid, unconsumed authorization code and return the raw code string.
    async fn insert_valid_code(
        db: &sea_orm::DatabaseConnection,
        client_id: &str,
        user_id: Uuid,
        verifier: &str,
        expires_in_secs: i64,
    ) -> String {
        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(expires_in_secs);
        let request_id = insert_auth_request(db, client_id, user_id).await;

        // Generate a upc_-prefixed raw code.
        let mut bytes = [0u8; 32];
        use rand::Rng;
        rand::rng().fill(&mut bytes);
        let raw = format!("upc_{}", URL_SAFE_NO_PAD.encode(bytes));
        let code_hash = hash_token(&raw);

        oauth_authorization_code::ActiveModel {
            id: Set(Uuid::now_v7()),
            code_hash: Set(code_hash),
            request_id: Set(request_id),
            client_id: Set(client_id.to_string()),
            user_id: Set(user_id),
            redirect_uri: Set(TEST_REDIRECT_URI.to_string()),
            scope: Set(TEST_SCOPE.to_string()),
            code_challenge: Set(pkce_challenge(verifier)),
            code_challenge_method: Set("S256".to_string()),
            resource: Set(TEST_RESOURCE.to_string()),
            issued_at: Set(now),
            expires_at: Set(expires_at),
            consumed_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_authorization_code");

        raw
    }

    fn token_form(
        code: &str,
        verifier: &str,
        client_id: &str,
        redirect_uri: &str,
        resource: &str,
    ) -> String {
        format!(
            "grant_type=authorization_code&code={code}&redirect_uri={redirect_uri}&client_id={client_id}&code_verifier={verifier}&resource={resource}",
            redirect_uri = percent_encode(redirect_uri),
            resource = percent_encode(resource),
        )
    }

    fn percent_encode(s: &str) -> String {
        use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
        utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
    }

    // -----------------------------------------------------------------------
    // Test 1 — happy path: valid code exchange returns 200 with tokens
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_authorization_code_returns_access_and_refresh() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        let code = insert_valid_code(&app.db, TEST_CLIENT_ID, user_id, TEST_VERIFIER, 30).await;

        let body = token_form(
            &code,
            TEST_VERIFIER,
            TEST_CLIENT_ID,
            TEST_REDIRECT_URI,
            TEST_RESOURCE,
        );

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(
            resp.status(),
            http::StatusCode::OK,
            "expected 200 OK on valid code exchange"
        );
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: uptrakit_web_api_types::oauth::TokenResponse =
            serde_json::from_slice(&body_bytes).expect("valid TokenResponse JSON");
        assert!(
            !parsed.access_token.is_empty(),
            "access_token must not be empty"
        );
        assert!(
            parsed.refresh_token.is_some(),
            "refresh_token must be present"
        );
        let rt = parsed.refresh_token.as_deref().unwrap_or("");
        assert!(rt.starts_with("upr_"), "refresh_token must start with upr_");
        assert_eq!(parsed.token_type, "Bearer");
        assert_eq!(parsed.scope, TEST_SCOPE);
    }

    // -----------------------------------------------------------------------
    // Test 2 — non-existent code returns 400 invalid_grant
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_invalid_code_returns_400_invalid_grant() {
        let (app, router) = app_with_oauth().await;

        // Insert a valid client so the handler reaches the code-verification step.
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;

        let body = token_form(
            "upc_doesnotexist",
            TEST_VERIFIER,
            TEST_CLIENT_ID,
            TEST_REDIRECT_URI,
            TEST_RESOURCE,
        );

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(
            parsed["error"].as_str().unwrap_or(""),
            "invalid_grant",
            "expected exactly invalid_grant for unknown code"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3 — valid code but wrong code_verifier → 400 invalid_grant (pkce_mismatch)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_pkce_mismatch_returns_400() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        let code = insert_valid_code(&app.db, TEST_CLIENT_ID, user_id, TEST_VERIFIER, 30).await;

        let body = token_form(
            &code,
            "wrong-verifier-value",
            TEST_CLIENT_ID,
            TEST_REDIRECT_URI,
            TEST_RESOURCE,
        );

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(parsed["error"], "invalid_grant");
    }

    // -----------------------------------------------------------------------
    // Test 4 — expired code → 400 invalid_grant (code_expired)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_expired_code_returns_400() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        // Insert a code that expired 1 second ago.
        let code = insert_valid_code(&app.db, TEST_CLIENT_ID, user_id, TEST_VERIFIER, -1).await;

        let body = token_form(
            &code,
            TEST_VERIFIER,
            TEST_CLIENT_ID,
            TEST_REDIRECT_URI,
            TEST_RESOURCE,
        );

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(parsed["error"], "invalid_grant");
    }

    // -----------------------------------------------------------------------
    // Test 5 — wrong redirect_uri → 400 invalid_grant (redirect_uri_mismatch)
    // -----------------------------------------------------------------------
    //
    // (Tests 6, 7, 8 follow after.)

    #[tokio::test]
    async fn token_redirect_uri_mismatch_returns_400() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        let code = insert_valid_code(&app.db, TEST_CLIENT_ID, user_id, TEST_VERIFIER, 30).await;

        // Use a different redirect_uri.
        let body = token_form(
            &code,
            TEST_VERIFIER,
            TEST_CLIENT_ID,
            "https://evil.example.com/callback",
            TEST_RESOURCE,
        );

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(parsed["error"], "invalid_grant");
    }

    // -----------------------------------------------------------------------
    // Test 6 — OAuth disabled returns 404
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_mcp_oauth_disabled_returns_404() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        // Default test state has oauth.enabled = false.
        assert!(
            !state.oauth.enabled,
            "expected oauth disabled in default test state"
        );
        let router = axum::Router::new()
            .route("/oauth/token", axum::routing::post(super::mcp_token))
            .with_state(Arc::clone(&state));

        let body = token_form(
            "upc_anycode",
            TEST_VERIFIER,
            TEST_CLIENT_ID,
            TEST_REDIRECT_URI,
            TEST_RESOURCE,
        );

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(
            resp.status(),
            http::StatusCode::NOT_FOUND,
            "expected 404 when OAuth is disabled"
        );
    }

    // -----------------------------------------------------------------------
    // Test 7 — replay of a consumed code returns 400 invalid_grant
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_authorization_code_replay_returns_invalid_grant() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        let code = insert_valid_code(&app.db, TEST_CLIENT_ID, user_id, TEST_VERIFIER, 30).await;

        let body = token_form(
            &code,
            TEST_VERIFIER,
            TEST_CLIENT_ID,
            TEST_REDIRECT_URI,
            TEST_RESOURCE,
        );

        // First exchange — must succeed.
        let req1 = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.clone()))
            .expect("build request 1");

        let resp1 = router.clone().oneshot(req1).await.expect("oneshot 1");
        assert_eq!(
            resp1.status(),
            http::StatusCode::OK,
            "first exchange must return 200"
        );

        // Second exchange with the same code — must be rejected.
        let req2 = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request 2");

        let resp2 = router.oneshot(req2).await.expect("oneshot 2");
        assert_eq!(
            resp2.status(),
            http::StatusCode::BAD_REQUEST,
            "replay must return 400"
        );
        let body_bytes = resp2.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(
            parsed["error"].as_str().unwrap_or(""),
            "invalid_grant",
            "replay must return invalid_grant"
        );
    }

    // -----------------------------------------------------------------------
    // Refresh-token grant helpers
    // -----------------------------------------------------------------------

    /// Build a raw `upr_testtoken` and insert the corresponding DB row.
    async fn insert_refresh_token(
        db: &sea_orm::DatabaseConnection,
        client_id: &str,
        user_id: Uuid,
        consent_id: Uuid,
        scope: &str,
        resource: &str,
        revoked: bool,
    ) -> String {
        use uptrakit_shared_db::entity::oauth_refresh_token;
        let raw = "upr_testtoken".to_string();
        let token_hash = hash_token(&raw);
        let now = OffsetDateTime::now_utc();
        oauth_refresh_token::ActiveModel {
            id: Set(Uuid::now_v7()),
            family_id: Set(Uuid::now_v7()),
            parent_id: Set(None),
            token_hash: Set(token_hash),
            client_id: Set(client_id.to_string()),
            user_id: Set(user_id),
            consent_id: Set(consent_id),
            scope: Set(scope.to_string()),
            resource: Set(resource.to_string()),
            issued_at: Set(now),
            expires_at: Set(now + time::Duration::days(30)),
            family_expires_at: Set(now + time::Duration::days(90)),
            rotated_at: Set(None),
            revoked_at: Set(if revoked { Some(now) } else { None }),
        }
        .insert(db)
        .await
        .expect("insert oauth_refresh_token");
        raw
    }

    fn refresh_token_form(
        refresh_token: &str,
        client_id: &str,
        resource: &str,
        scope: Option<&str>,
    ) -> String {
        let base = format!(
            "grant_type=refresh_token&refresh_token={rt}&client_id={cid}&resource={res}",
            rt = percent_encode(refresh_token),
            cid = client_id,
            res = percent_encode(resource),
        );
        match scope {
            Some(s) => format!("{base}&scope={}", percent_encode(s)),
            None => base,
        }
    }

    // -----------------------------------------------------------------------
    // Test 8 — refresh_token grant rotates and returns a new pair
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_refresh_token_rotates_and_returns_new_pair() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        let consent_id = insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        let raw_rt = insert_refresh_token(
            &app.db,
            TEST_CLIENT_ID,
            user_id,
            consent_id,
            TEST_SCOPE,
            TEST_RESOURCE,
            false,
        )
        .await;

        let body = refresh_token_form(&raw_rt, TEST_CLIENT_ID, TEST_RESOURCE, None);

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(
            resp.status(),
            http::StatusCode::OK,
            "expected 200 on valid refresh_token grant"
        );
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: uptrakit_web_api_types::oauth::TokenResponse =
            serde_json::from_slice(&body_bytes).expect("valid TokenResponse JSON");
        assert!(
            !parsed.access_token.is_empty(),
            "access_token must not be empty"
        );
        let new_rt = parsed
            .refresh_token
            .as_deref()
            .expect("refresh_token must be present");
        assert!(
            new_rt.starts_with("upr_"),
            "new refresh_token must start with upr_"
        );
        assert_ne!(
            new_rt,
            raw_rt.as_str(),
            "new refresh_token must differ from the original"
        );
        assert_eq!(parsed.scope, TEST_SCOPE);
    }

    // -----------------------------------------------------------------------
    // Test 9 — replay of the same refresh_token returns 400 invalid_grant
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_refresh_replay_returns_invalid_grant() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        let consent_id = insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        let raw_rt = insert_refresh_token(
            &app.db,
            TEST_CLIENT_ID,
            user_id,
            consent_id,
            TEST_SCOPE,
            TEST_RESOURCE,
            false,
        )
        .await;

        let body = refresh_token_form(&raw_rt, TEST_CLIENT_ID, TEST_RESOURCE, None);

        // First use — must succeed.
        let req1 = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.clone()))
            .expect("build request 1");
        let resp1 = router.clone().oneshot(req1).await.expect("oneshot 1");
        assert_eq!(
            resp1.status(),
            http::StatusCode::OK,
            "first rotation must succeed"
        );

        // Second use (replay) — must be rejected.
        let req2 = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request 2");
        let resp2 = router.oneshot(req2).await.expect("oneshot 2");

        assert_eq!(
            resp2.status(),
            http::StatusCode::BAD_REQUEST,
            "replay must return 400"
        );
        let body_bytes = resp2.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(
            parsed["error"].as_str().unwrap_or(""),
            "invalid_grant",
            "replay must return invalid_grant"
        );
        assert_eq!(
            parsed["error_description"].as_str().unwrap_or(""),
            "replay_detected",
            "replay must describe reason as replay_detected"
        );
    }

    // -----------------------------------------------------------------------
    // Test 10 — refresh_token with revoked_at set returns 400 invalid_grant
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_refresh_revoked_returns_invalid_grant() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        let consent_id = insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        // Insert a refresh token that is already revoked.
        let raw_rt = insert_refresh_token(
            &app.db,
            TEST_CLIENT_ID,
            user_id,
            consent_id,
            TEST_SCOPE,
            TEST_RESOURCE,
            true,
        )
        .await;

        let body = refresh_token_form(&raw_rt, TEST_CLIENT_ID, TEST_RESOURCE, None);

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(
            resp.status(),
            http::StatusCode::BAD_REQUEST,
            "revoked token must return 400"
        );
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(
            parsed["error"].as_str().unwrap_or(""),
            "invalid_grant",
            "revoked token must return invalid_grant"
        );
    }

    // -----------------------------------------------------------------------
    // Test 11 — requesting scope superset returns 400 invalid_scope
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn token_refresh_scope_superset_returns_invalid_scope() {
        let (app, router) = app_with_oauth().await;

        let user_id = insert_user_with_tenant(&app.db, app.tenant_id).await;
        insert_trusted_client(&app.db, TEST_CLIENT_ID, TEST_REDIRECT_URI).await;
        let consent_id = insert_consent(&app.db, user_id, TEST_CLIENT_ID).await;
        // Token is bound to "mcp:read" only.
        let raw_rt = insert_refresh_token(
            &app.db,
            TEST_CLIENT_ID,
            user_id,
            consent_id,
            "mcp:read",
            TEST_RESOURCE,
            false,
        )
        .await;

        // Request a superset "mcp:read mcp:write".
        let body = refresh_token_form(
            &raw_rt,
            TEST_CLIENT_ID,
            TEST_RESOURCE,
            Some("mcp:read mcp:write"),
        );

        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");

        assert_eq!(
            resp.status(),
            http::StatusCode::BAD_REQUEST,
            "scope superset must return 400"
        );
        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(
            parsed["error"].as_str().unwrap_or(""),
            "invalid_scope",
            "scope superset must return invalid_scope"
        );
    }
}
