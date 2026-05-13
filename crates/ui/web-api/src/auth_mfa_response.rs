//! Response builders for MFA challenge load failures.
//!
//! Sanctioned exit for `routes/mfa.rs` handlers that need to return a single
//! generic `401 Unauthorized` for `MfaChallengeNotFound` / `MfaChallengeExpired`
//! / `MfaChallengeExhausted` — collapsing all three to one status is a deliberate
//! information-hiding measure (do not leak whether a challenge exists, has
//! expired, or has been exhausted). Standard `From<Report<AuthError>>` mapping
//! emits 404 / 401 / 429 respectively, which would leak that distinction; this
//! helper preserves the 401 collapse while keeping the `match e.current_context()`
//! pattern out of `crates/ui/web-api/src/routes/` per
//! `scripts/check_legacy_error_matches.sh`. See `docs/development/error-handling.md`
//! Pattern 18.

use axum::http::StatusCode;
use axum::response::Response;
use rootcause::prelude::*;
use uptrakit_web_api_auth::auth::AuthError;

use crate::error_response::error_response;

/// Build the `error_response` for an MFA challenge load failure.
pub(crate) fn mfa_challenge_load_error_response(report: &Report<AuthError>) -> Response {
    let (status, msg) = match report.current_context() {
        AuthError::MfaChallengeNotFound => {
            (StatusCode::UNAUTHORIZED, "Invalid or expired MFA token")
        }
        AuthError::MfaChallengeExpired => (StatusCode::UNAUTHORIZED, "MFA token has expired"),
        AuthError::MfaChallengeExhausted => (StatusCode::UNAUTHORIZED, "Too many failed attempts"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
    };
    error_response(status, msg)
}
