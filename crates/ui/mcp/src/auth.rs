use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tower::{Layer, Service};
use uuid::Uuid;

use uptrakit_audit_log::{AuditActionType, AuditActorType, AuditEntry, AuditOutcome};
use uptrakit_controller_core::auth::api_token::{
    authenticate_api_token, emit_api_token_auth_audit,
};
use uptrakit_controller_core::auth::{AuthFailure, Permission};
// DB entity prelude exports `Permission` as the ORM entity — alias it to
// avoid the name collision with `uptrakit_controller_core::auth::Permission`
// (the permission enum used throughout this file).
use uptrakit_shared_db::entity::prelude::Permission as DbPermissionEntity;
use uptrakit_shared_db::entity::prelude::{RolePermission, User, UserRole};
use uptrakit_shared_db::entity::{permission, role_permission, user_role};
use uptrakit_web_api_types::oauth::McpScope;

use crate::context::{McpAuthError, McpAuthMethod, McpRequestContext};
use crate::state::McpState;

// ---------------------------------------------------------------------------
// Tower layer
// ---------------------------------------------------------------------------

/// Tower [`Layer`] that validates API-token credentials before forwarding to
/// the underlying [`StreamableHttpService`].
///
/// Dispatches on the token prefix: `upk_`-prefixed strings are validated as
/// API tokens; JWT-shaped strings (`eyJ…` with two dots) are validated as
/// OAuth access tokens when OAuth is enabled. Emits a spec-compliant
/// `WWW-Authenticate` header on 401 responses when OAuth is enabled.
///
/// On success inserts [`McpRequestContext`] into request extensions for tool
/// handlers.
#[derive(Clone)]
pub struct McpAuthLayer {
    state: McpState,
}

impl McpAuthLayer {
    /// Create a new [`McpAuthLayer`] from the given [`McpState`].
    #[must_use]
    pub fn new(state: McpState) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for McpAuthLayer {
    type Service = McpAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        McpAuthService {
            inner,
            state: self.state.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tower service
// ---------------------------------------------------------------------------

/// Tower [`Service`] produced by [`McpAuthLayer`].
#[derive(Clone)]
pub struct McpAuthService<S> {
    inner: S,
    state: McpState,
}

impl<S, B> Service<axum::extract::Request<B>> for McpAuthService<S>
where
    S: Service<axum::extract::Request<B>> + Clone + Send + 'static,
    S::Response: IntoResponse,
    S::Error: Into<std::convert::Infallible>,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(|e| e.into())
    }

    fn call(&mut self, mut req: axum::extract::Request<B>) -> Self::Future {
        let state = self.state.clone();
        // Standard Tower clone-and-replace so the ready service is used for this call.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            // Build the PRM URL once; used in WWW-Authenticate headers below.
            let prm_url: Option<String> = state.oauth_canonical.as_ref().map(|c| {
                format!(
                    "{}/.well-known/oauth-protected-resource",
                    c.issuer().as_str()
                )
            });

            let token = extract_bearer_token(&req);
            let mcp_ctx = match token.as_deref() {
                Some(t) if t.starts_with("upk_") => {
                    validate_api_token_for_mcp(&state, Some(t)).await
                }
                Some(t) if looks_like_jwt(t) => {
                    if state.oauth_enabled {
                        validate_oauth_access_token_for_mcp(&state, t).await
                    } else {
                        Err(McpAuthError::JwtNotAccepted)
                    }
                }
                Some(_) => Err(McpAuthError::Unauthorized),
                None => Err(McpAuthError::MissingCredentials),
            };

            match mcp_ctx {
                Ok(ctx) => {
                    req.extensions_mut().insert(ctx);
                    inner
                        .call(req)
                        .await
                        .map(IntoResponse::into_response)
                        .map_err(Into::into)
                }
                Err(McpAuthError::MissingCredentials) => Ok(unauthorized_with_www_auth(
                    "Authentication required: provide a valid Bearer token",
                    prm_url.as_deref(),
                    state.oauth_enabled,
                )),
                // JWT presented but OAuth is disabled — don't advertise PRM.
                Err(McpAuthError::JwtNotAccepted) => Ok(unauthorized(
                    "JWT tokens are not accepted for MCP access. \
                     Use an API token (upk_...)",
                )),
                Err(McpAuthError::Forbidden) => Ok(plain(
                    StatusCode::FORBIDDEN,
                    "User is deactivated or lacks the AccessMcp permission",
                )),
                Err(McpAuthError::Internal) => Ok(plain(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error",
                )),
                // McpAuthError::Unauthorized and any future #[non_exhaustive] variants.
                Err(_) => Ok(unauthorized_with_www_auth(
                    "Invalid or revoked token",
                    prm_url.as_deref(),
                    state.oauth_enabled,
                )),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_bearer_token<B>(req: &axum::extract::Request<B>) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_owned)
}

/// Returns `true` if `token` has the three-segment shape of a compact JWT.
///
/// Only used for prefix-dispatch — the verifier performs full cryptographic
/// validation; this heuristic merely avoids running JWT parsing on API tokens.
fn looks_like_jwt(token: &str) -> bool {
    token.starts_with("eyJ") && token.matches('.').count() == 2
}

fn plain(status: StatusCode, body: &'static str) -> Response {
    #[expect(
        clippy::expect_used,
        reason = "infallible: `Response::builder()` with a static MIME type and a `&'static str` body cannot fail"
    )]
    axum::http::Response::builder()
        .status(status)
        .header(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )
        .body(axum::body::Body::from(body))
        .expect("valid response builder arguments")
}

fn unauthorized(body: &'static str) -> Response {
    plain(StatusCode::UNAUTHORIZED, body)
}

/// Build a 401 response with a spec-compliant `WWW-Authenticate` header when
/// OAuth is enabled and we know the PRM discovery URL.
fn unauthorized_with_www_auth(
    body: &'static str,
    prm_url: Option<&str>,
    oauth_enabled: bool,
) -> Response {
    if !oauth_enabled {
        return unauthorized(body);
    }

    let www_auth = match prm_url {
        Some(url) => format!(r#"Bearer realm="mcp", resource_metadata="{url}", scope="mcp:read""#),
        None => r#"Bearer realm="mcp", scope="mcp:read""#.to_owned(),
    };

    let header_value = axum::http::HeaderValue::from_str(&www_auth)
        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("Bearer realm=\"mcp\""));

    let mut resp = unauthorized(body);
    resp.headers_mut()
        .insert(axum::http::header::WWW_AUTHENTICATE, header_value);
    resp
}

// ---------------------------------------------------------------------------
// McpState-based auth: API token path
// ---------------------------------------------------------------------------

/// Validate a `upk_`-prefixed API token for an MCP request.
///
/// Accepts `None` (missing header) or `Some(token_str)`. Handles the full
/// auth path: missing token, JWT rejection, DB lookup, `AccessMcp` permission
/// check, and audit emission. Does not require `AppState`.
///
/// # Errors
///
/// Returns [`McpAuthError::MissingCredentials`] if no token is present,
/// [`McpAuthError::JwtNotAccepted`] for non-API-token bearer values,
/// [`McpAuthError::Unauthorized`] for invalid/revoked tokens,
/// [`McpAuthError::Forbidden`] for deactivated users or missing `AccessMcp`,
/// and [`McpAuthError::Internal`] on database failures.
pub async fn validate_api_token_for_mcp(
    state: &McpState,
    token: Option<&str>,
) -> Result<McpRequestContext, McpAuthError> {
    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => {
            emit_api_token_auth_audit(
                &state.audit_emitter,
                state.default_tenant_id,
                None,
                AuditOutcome::Denied,
                "missing_authorization_header",
            );
            return Err(McpAuthError::MissingCredentials);
        }
    };

    if !token.starts_with("upk_") {
        emit_api_token_auth_audit(
            &state.audit_emitter,
            state.default_tenant_id,
            None,
            AuditOutcome::Denied,
            "jwt_not_accepted_for_mcp",
        );
        return Err(McpAuthError::JwtNotAccepted);
    }

    let (auth_user, token_id) =
        match authenticate_api_token(state.db.db(), state.default_tenant_id, token).await {
            Ok(pair) => pair,
            Err(failure) => {
                if let Some(reason) = failure.api_token_reason_code() {
                    emit_api_token_auth_audit(
                        &state.audit_emitter,
                        state.default_tenant_id,
                        None,
                        AuditOutcome::Denied,
                        reason,
                    );
                }
                return Err(match failure {
                    AuthFailure::UserDeactivated => McpAuthError::Forbidden,
                    AuthFailure::InternalError => McpAuthError::Internal,
                    _ => McpAuthError::Unauthorized,
                });
            }
        };

    if !auth_user.has_permission(Permission::AccessMcp) {
        emit_api_token_auth_audit(
            &state.audit_emitter,
            state.default_tenant_id,
            None,
            AuditOutcome::Denied,
            "missing_access_mcp_permission",
        );
        return Err(McpAuthError::Forbidden);
    }

    emit_api_token_auth_audit(
        &state.audit_emitter,
        state.default_tenant_id,
        None,
        AuditOutcome::Success,
        "authenticated",
    );

    Ok(McpRequestContext::new(
        auth_user.user_id,
        token_id,
        state.default_tenant_id,
        auth_user.permissions,
        McpAuthMethod::ApiToken,
    ))
}

// ---------------------------------------------------------------------------
// McpState-based auth: OAuth JWT path
// ---------------------------------------------------------------------------

/// Validate an OAuth 2.1 JWT access token for an MCP request.
///
/// Verifies the JWT (signature, `iss`, `aud`, `exp`, etc.), loads the subject
/// user from the DB, checks `is_active`, loads their permissions, and verifies
/// the `AccessMcp` permission is present.
///
/// # Errors
///
/// - [`McpAuthError::Unauthorized`] — JWT invalid, expired, wrong `iss`/`aud`,
///   or malformed `sub`/`tenant_id`/`jti` claims.
/// - [`McpAuthError::Forbidden`] — user not found, deactivated, or lacks
///   the `AccessMcp` permission.
/// - [`McpAuthError::Internal`] — database failure.
pub async fn validate_oauth_access_token_for_mcp(
    state: &McpState,
    token: &str,
) -> Result<McpRequestContext, McpAuthError> {
    let verifier = match state.oauth_verifier.as_ref() {
        Some(v) => v,
        None => {
            emit_mcp_oauth_audit(state, None, AuditOutcome::Denied, "oauth_not_configured");
            return Err(McpAuthError::Unauthorized);
        }
    };

    let claims = match verifier.verify(token) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "OAuth JWT verification failed");
            emit_mcp_oauth_audit(state, None, AuditOutcome::Denied, "jwt_verification_failed");
            return Err(McpAuthError::Unauthorized);
        }
    };

    let user_id = match claims.sub.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            emit_mcp_oauth_audit(state, None, AuditOutcome::Denied, "invalid_sub_claim");
            return Err(McpAuthError::Unauthorized);
        }
    };

    let tenant_id = match claims.tenant_id.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            emit_mcp_oauth_audit(state, None, AuditOutcome::Denied, "invalid_tenant_id_claim");
            return Err(McpAuthError::Unauthorized);
        }
    };

    let jti = match claims.jti.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            emit_mcp_oauth_audit(state, None, AuditOutcome::Denied, "invalid_jti_claim");
            return Err(McpAuthError::Unauthorized);
        }
    };

    // Load user to verify it exists and is active.
    let user = match User::find_by_id(user_id).one(state.db.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            emit_mcp_oauth_audit(state, Some(user_id), AuditOutcome::Denied, "user_not_found");
            return Err(McpAuthError::Forbidden);
        }
        Err(e) => {
            tracing::error!(error = %e, %user_id, "DB error looking up OAuth user for MCP");
            emit_mcp_oauth_audit(state, None, AuditOutcome::Denied, "internal_error");
            return Err(McpAuthError::Internal);
        }
    };

    if !user.is_active {
        emit_mcp_oauth_audit(
            state,
            Some(user_id),
            AuditOutcome::Denied,
            "user_deactivated",
        );
        return Err(McpAuthError::Forbidden);
    }

    let permissions = match load_user_permissions_for_mcp(state.db.db(), tenant_id, user_id).await {
        Ok(perms) => perms,
        Err(e) => {
            tracing::error!(error = %e, %user_id, "failed to load user permissions for OAuth MCP");
            emit_mcp_oauth_audit(state, Some(user_id), AuditOutcome::Denied, "internal_error");
            return Err(McpAuthError::Internal);
        }
    };

    if !permissions.contains(&Permission::AccessMcp) {
        emit_mcp_oauth_audit(
            state,
            Some(user_id),
            AuditOutcome::Denied,
            "missing_access_mcp_permission",
        );
        return Err(McpAuthError::Forbidden);
    }

    emit_mcp_oauth_audit(state, Some(user_id), AuditOutcome::Success, "authenticated");

    let scopes: Vec<McpScope> = claims
        .scope
        .split_whitespace()
        .map(|s| McpScope::from(s.to_owned()))
        .collect();

    Ok(McpRequestContext::new(
        user_id,
        jti,
        tenant_id,
        permissions,
        McpAuthMethod::OAuth {
            client_id: claims.client_id,
            jti,
            scopes,
        },
    ))
}

/// Emit an `MCP_OAUTH_AUTHENTICATE` audit entry.
///
/// `actor_id` is `None` before the subject UUID is parsed (early failures).
fn emit_mcp_oauth_audit(
    state: &McpState,
    actor_id: Option<Uuid>,
    outcome: AuditOutcome,
    reason: &'static str,
) {
    let entry = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        AuditActionType::MCP_OAUTH_AUTHENTICATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(AuditActorType::User, actor_id)
    .outcome(outcome)
    .details(serde_json::json!({ "reason_code": reason }))
    .build();

    match entry {
        Ok(e) => state.audit_emitter.emit_event(e),
        Err(e) => tracing::warn!(error = %e, "failed to build MCP_OAUTH_AUTHENTICATE audit entry"),
    }
}

/// Load the permission set for `user_id` in `tenant_id`.
///
/// Mirrors the same query in `uptrakit_controller_core::auth::api_token`
/// (which is private); duplicated here because `uptrakit-mcp` must not depend
/// on `uptrakit-web-api`.
async fn load_user_permissions_for_mcp(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Permission>, sea_orm::DbErr> {
    let user_roles = UserRole::find()
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await?;

    let role_ids: Vec<Uuid> = user_roles.iter().map(|ur| ur.role_id).collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }

    let role_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .all(db)
        .await?;

    let perm_ids: Vec<Uuid> = role_perms.iter().map(|rp| rp.permission_id).collect();
    if perm_ids.is_empty() {
        return Ok(Vec::new());
    }

    let perm_models = DbPermissionEntity::find()
        .filter(permission::Column::Id.is_in(perm_ids))
        .all(db)
        .await?;

    let mut seen = std::collections::HashSet::new();
    let permissions = perm_models
        .into_iter()
        .filter_map(|p| p.name.parse::<Permission>().ok())
        .filter(|p| seen.insert(p.clone()))
        .collect();

    Ok(permissions)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + Clone>() {}

    #[test]
    fn mcp_auth_layer_types_are_send_sync() {
        assert_send_sync::<McpAuthLayer>();
    }

    #[test]
    fn looks_like_jwt_detects_jwt_shape() {
        assert!(looks_like_jwt("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sig"));
        assert!(!looks_like_jwt("upk_abc123"));
        assert!(!looks_like_jwt("eyJhbGc.only_two"));
        assert!(!looks_like_jwt(""));
    }
}
