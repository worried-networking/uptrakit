use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};
use uuid::Uuid;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::{
    AuthFailure, authenticate_api_token, emit_api_token_auth_audit,
};

/// Per-request context injected by the MCP auth layer into request extensions.
///
/// Tool handlers can read this by extracting `http::request::Parts` via rmcp's
/// `Extension<Parts>` and calling `parts.extensions.get::<McpRequestContext>()`.
///
/// # Example
///
/// ```rust,ignore
/// use rmcp::handler::server::tool::Extension;
/// use http::request::Parts;
/// use uptrakit_web_api::mcp::auth::McpRequestContext;
///
/// async fn my_tool(Extension(parts): Extension<Parts>) {
///     let ctx = parts.extensions.get::<McpRequestContext>().unwrap();
///     tracing::info!(user_id = %ctx.user_id, "tool called");
/// }
/// ```
#[derive(Clone, Debug)]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub permissions: Vec<Permission>,
}

impl McpRequestContext {
    /// Returns `true` if the user holds `perm`.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm)
    }
}

// ---------------------------------------------------------------------------
// Tower layer
// ---------------------------------------------------------------------------

/// Tower [`Layer`] that validates API-token credentials before forwarding to
/// the underlying [`StreamableHttpService`].
///
/// Rejects JWT tokens with a descriptive 401 message (MCP clients must use
/// `upk_`-prefixed API tokens). On success, inserts [`McpRequestContext`] into
/// request extensions so tool handlers can read auth state.
#[derive(Clone)]
pub struct McpAuthLayer {
    state: Arc<AppState>,
}

impl McpAuthLayer {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for McpAuthLayer {
    type Service = McpAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        McpAuthService {
            inner,
            state: Arc::clone(&self.state),
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
    state: Arc<AppState>,
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
        // `poll_ready` on the inner may return `Err` typed as `S::Error`; since
        // `S::Error: Into<Infallible>` the into() call is unreachable but needed
        // for type-correctness.
        self.inner.poll_ready(cx).map_err(|e| e.into())
    }

    fn call(&mut self, mut req: axum::extract::Request<B>) -> Self::Future {
        let state = Arc::clone(&self.state);
        // Standard Tower clone-and-swap pattern so the ready clone is used.
        let mut inner = self.inner.clone();
        std::mem::swap(&mut inner, &mut self.inner);

        Box::pin(async move {
            // ------------------------------------------------------------------
            // 1. Extract Authorization: Bearer <token>
            // ------------------------------------------------------------------
            let token = match extract_bearer_token(&req) {
                Some(t) => t,
                None => {
                    emit_api_token_auth_audit(
                        &state,
                        None,
                        uptrakit_audit_log::AuditOutcome::Denied,
                        "missing_authorization_header",
                    );
                    return Ok(unauthorized(
                        "Authentication required: provide an API token via \
                         Authorization: Bearer <upk_...>",
                    ));
                }
            };

            // ------------------------------------------------------------------
            // 2. Reject JWTs — MCP only accepts upk_ API tokens
            // ------------------------------------------------------------------
            if !token.starts_with("upk_") {
                emit_api_token_auth_audit(
                    &state,
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "jwt_not_accepted_for_mcp",
                );
                return Ok(unauthorized(
                    "JWT tokens are not accepted for MCP access. \
                     Use an API token (upk_...)",
                ));
            }

            // ------------------------------------------------------------------
            // 3. Validate API token via DB lookup
            // ------------------------------------------------------------------
            let (auth_user, token_id) = match authenticate_api_token(&state, &token).await {
                Ok(pair) => pair,
                Err(failure) => {
                    if let Some(reason) = failure.api_token_reason_code() {
                        emit_api_token_auth_audit(
                            &state,
                            None,
                            uptrakit_audit_log::AuditOutcome::Denied,
                            reason,
                        );
                    }
                    let status = match failure {
                        AuthFailure::UserDeactivated => StatusCode::FORBIDDEN,
                        _ => StatusCode::UNAUTHORIZED,
                    };
                    return Ok(plain(status, failure_message(&failure)));
                }
            };

            // ------------------------------------------------------------------
            // 4. Check Permission::AccessMcp
            // ------------------------------------------------------------------
            if !auth_user.has_permission(Permission::AccessMcp) {
                emit_api_token_auth_audit(
                    &state,
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "missing_access_mcp_permission",
                );
                return Ok(plain(
                    StatusCode::FORBIDDEN,
                    "API token does not have the AccessMcp permission",
                ));
            }

            // ------------------------------------------------------------------
            // 5. Emit success audit
            // ------------------------------------------------------------------
            emit_api_token_auth_audit(
                &state,
                None,
                uptrakit_audit_log::AuditOutcome::Success,
                "authenticated",
            );

            // ------------------------------------------------------------------
            // 6. Build and insert McpRequestContext
            // ------------------------------------------------------------------
            let mcp_ctx = McpRequestContext {
                user_id: auth_user.user_id,
                token_id,
                tenant_id: state.default_tenant_id,
                permissions: auth_user.permissions,
            };
            req.extensions_mut().insert(mcp_ctx);

            // ------------------------------------------------------------------
            // 7. Forward to inner service
            // ------------------------------------------------------------------
            // `S::Error: Into<Infallible>` so the map_err is a type-level
            // assertion; the branch is unreachable at runtime.
            inner
                .call(req)
                .await
                .map(IntoResponse::into_response)
                .map_err(Into::into)
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

fn failure_message(failure: &AuthFailure) -> &'static str {
    match failure {
        AuthFailure::InvalidApiToken => "Invalid or revoked API token",
        AuthFailure::UserNotFound => "User not found",
        AuthFailure::UserDeactivated => "User is deactivated",
        AuthFailure::InternalError => "Internal server error",
        _ => "Authentication denied",
    }
}

fn plain(status: StatusCode, body: &'static str) -> Response {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + Clone>() {}

    #[test]
    fn mcp_request_context_is_clone_send_sync() {
        assert_send_sync::<McpRequestContext>();
    }
}
