use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uptrakit_web_api_types::error::ErrorResponse;

/// Build a JSON error response with the given HTTP status and message.
pub fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
            code: None,
        }),
    )
        .into_response()
}

/// Build a JSON error response with the given HTTP status, message, and machine-readable code.
pub fn error_response_with_code(
    status: StatusCode,
    message: impl Into<String>,
    code: impl Into<String>,
) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
            code: Some(code.into()),
        }),
    )
        .into_response()
}

/// Build an RFC 6749 §5.2 / RFC 8628 §3.5 OAuth error response.
///
/// The body is serialised as `application/json` with `error`, optional
/// `error_description`, and optional `interval` (slow_down extension).
pub fn oauth_error_response(
    status: StatusCode,
    error: uptrakit_web_api_types::oauth::OAuthErrorCode,
    description: Option<String>,
    interval: Option<i32>,
) -> Response {
    let body = uptrakit_web_api_types::oauth::OAuthErrorResponse::new(error, description, interval);
    (status, Json(body)).into_response()
}
