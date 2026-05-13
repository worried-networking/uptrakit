//! RFC 6749 §5.2 OAuth error response builders.
//!
//! Used by both `routes/oauth/*` handlers (direct construction of bad-request /
//! server-error responses) and the `oauth/services/*_error_to_response` helpers
//! that produce sanctioned RFC 6749 exits for spec endpoints (token, register,
//! authorize).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Build a 400 RFC 6749 §5.2 OAuth error response.
pub(crate) fn oauth_400(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response()
}

/// Build a 403 RFC 6749-style error response.
pub(crate) fn oauth_403(error: &str, description: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response()
}

/// Build a generic 500 response for unexpected server-side errors.
pub(crate) fn oauth_500() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({
            "error": "server_error",
            "error_description": "internal server error",
        })),
    )
        .into_response()
}
