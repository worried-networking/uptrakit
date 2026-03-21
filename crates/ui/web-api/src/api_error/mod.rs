//! Centralised HTTP error type for all API route handlers.
//!
//! Route handlers propagate domain errors via `?` — each domain error type has a
//! [`From`] impl in [`mappings`].  [`IntoResponse`] serialises the error to a
//! JSON `{ error, code }` body and emits a structured `tracing::error!` event
//! when `internal_detail` is `Some` (5xx errors only).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rootcause::Report;
use std::fmt::Display;

use crate::error_response::error_response_with_code;

pub mod mappings;

#[cfg(test)]
mod tests;

/// Centralised HTTP error returned from route handlers.
///
/// All fields are private; construct instances via the `From` impls in
/// [`mappings`] or [`ApiError::new`].
pub struct ApiError {
    status: StatusCode,
    user_message: String,
    code: String,
    /// Present for 5xx errors; drives the `tracing::error!` event.
    /// Absent for 4xx errors so client mistakes are not noise-logged.
    internal_detail: Option<String>,
}

impl ApiError {
    /// Construct an `ApiError` directly.  Prefer the `From` impls for
    /// domain-error conversion.
    pub(crate) fn new(
        status: StatusCode,
        user_message: impl Into<String>,
        code: impl Into<String>,
        internal_detail: Option<String>,
    ) -> Self {
        Self {
            status,
            user_message: user_message.into(),
            code: code.into(),
            internal_detail,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Some(ref detail) = self.internal_detail {
            tracing::error!(
                error.code = %self.code,
                error.status = %self.status.as_u16(),
                error.detail = %detail,
                "request failed"
            );
        }
        error_response_with_code(self.status, self.user_message, self.code)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Format a [`rootcause::Report`] as a single-line summary string, truncated
/// to [`REPORT_SUMMARY_MAX_BYTES`].
pub(crate) fn format_report_summary<E: Display>(report: &Report<E>) -> String {
    let s = format!("{report}");
    truncate_utf8_safe(&s, REPORT_SUMMARY_MAX_BYTES)
}

const REPORT_SUMMARY_MAX_BYTES: usize = 1024;

/// Truncate `s` to at most `max_bytes` bytes, respecting UTF-8 char boundaries.
///
/// Appends `"…[truncated]"` if the string was shortened.
pub(crate) fn truncate_utf8_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    // Walk back until we land on a valid UTF-8 char boundary.
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…[truncated]", &s[..boundary])
}
