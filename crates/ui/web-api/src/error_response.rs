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
