use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};

use uptrakit_web_api::AppState;
use uptrakit_web_api::{McpAuthError, validate_api_token_for_mcp};

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
        self.inner.poll_ready(cx).map_err(|e| e.into())
    }

    fn call(&mut self, mut req: axum::extract::Request<B>) -> Self::Future {
        let state = Arc::clone(&self.state);
        // Standard Tower clone-and-swap so the ready clone is used.
        let mut inner = self.inner.clone();
        std::mem::swap(&mut inner, &mut self.inner);

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
