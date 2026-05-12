//! Shared helpers for OAuth route handlers.

use axum::Extension;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::extract::ExternalBaseUrl;

/// Resolve the external base URL for constructing verification and endpoint URIs.
///
/// Mirrors the resolution chain used in the legacy `device_auth_start` handler:
/// 1. `ExternalBaseUrl` extension — set by the reverse-proxy middleware.
/// 2. `Origin` header.
/// 3. `Host` header with an `https://` prefix.
/// 4. Empty string as a safe fallback.
pub(super) fn resolve_external_base_url(
    ext: Option<Extension<ExternalBaseUrl>>,
    headers: &HeaderMap,
) -> String {
    if let Some(Extension(base)) = ext {
        return base.0.clone();
    }
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        return origin.to_string();
    }
    if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
        return format!("https://{host}");
    }
    String::new()
}

/// Build a 400 RFC 6749 §5.2 OAuth error response.
pub(super) fn oauth_400(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response()
}

/// Build a generic 500 response for unexpected server-side errors.
pub(super) fn oauth_500() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({
            "error": "server_error",
            "error_description": "internal server error",
        })),
    )
        .into_response()
}
