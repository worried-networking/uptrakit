use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};

use uptrakit_audit_log::AuditOutcome;
use uptrakit_controller_core::auth::api_token::{
    authenticate_api_token, emit_api_token_auth_audit,
};
use uptrakit_controller_core::auth::{AuthFailure, Permission};

use crate::context::{McpAuthError, McpAuthMethod, McpRequestContext};
use crate::state::McpState;

// ---------------------------------------------------------------------------
// Tower layer
// ---------------------------------------------------------------------------

/// Tower [`Layer`] that validates API-token credentials before forwarding to
/// the underlying [`StreamableHttpService`].
///
/// Rejects missing tokens, JWT tokens, and invalid API tokens. On success,
/// inserts [`McpRequestContext`] into request extensions for tool handlers.
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
            let token = extract_bearer_token(&req);
            let mcp_ctx = match validate_api_token_for_mcp(&state, token.as_deref()).await {
                Ok(ctx) => ctx,
                Err(McpAuthError::MissingCredentials) => {
                    return Ok(unauthorized(
                        "Authentication required: provide an API token via \
                         Authorization: Bearer <upk_...>",
                    ));
                }
                Err(McpAuthError::JwtNotAccepted) => {
                    return Ok(unauthorized(
                        "JWT tokens are not accepted for MCP access. \
                         Use an API token (upk_...)",
                    ));
                }
                Err(McpAuthError::Forbidden) => {
                    return Ok(plain(
                        StatusCode::FORBIDDEN,
                        "User is deactivated or lacks the AccessMcp permission",
                    ));
                }
                Err(McpAuthError::Internal) => {
                    return Ok(plain(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    ));
                }
                Err(_) => {
                    return Ok(unauthorized("Invalid or revoked API token"));
                }
            };

            req.extensions_mut().insert(mcp_ctx);
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

// ---------------------------------------------------------------------------
// McpState-based auth
// ---------------------------------------------------------------------------

/// Validate a bearer token for an MCP request using only [`McpState`].
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
}
