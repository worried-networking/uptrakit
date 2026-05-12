//! Typed OAuth error enum per RFC 6749 §5.2 + RFC 8707 §2.
//!
//! The authorization server returns standard OAuth error codes at the token
//! and authorization endpoints. [`OAuthError`] is the AS-internal
//! representation; the wire form (the `error` string and HTTP status) is
//! produced via [`OAuthError::error_code`] and [`OAuthError::http_status`].
//!
//! Cross-boundary conversion from `sea_orm::DbErr` is wired through
//! [`impl_report_conversion!`] per
//! [`docs/development/error-handling.md`](../../../../../docs/development/error-handling.md);
//! callers use `.context_to()?` instead of `?`.

use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

/// OAuth 2.1 + RFC 8707 error envelope returned by the authorization server.
///
/// Variants map 1:1 onto the RFC 6749 §5.2 `error` strings (plus
/// `invalid_target` from RFC 8707 §2 and `insufficient_scope` from RFC 6750
/// §3.1). Use [`error_code`](Self::error_code) for the wire string and
/// [`http_status`](Self::http_status) for the matching HTTP status code.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum OAuthError {
    /// The request is missing a parameter, includes an invalid value, or is
    /// otherwise malformed (RFC 6749 §5.2).
    #[error("invalid_request: {0}")]
    InvalidRequest(String),
    /// Client authentication failed (RFC 6749 §5.2).
    #[error("invalid_client")]
    InvalidClient,
    /// The grant or refresh token is invalid, expired, revoked, or does not
    /// match the redirection URI / client (RFC 6749 §5.2).
    #[error("invalid_grant: {0}")]
    InvalidGrant(&'static str),
    /// The authenticated client is not authorized to use this grant type
    /// (RFC 6749 §5.2).
    #[error("unauthorized_client")]
    UnauthorizedClient,
    /// The grant type is not supported by the authorization server
    /// (RFC 6749 §5.2).
    #[error("unsupported_grant_type")]
    UnsupportedGrantType,
    /// The requested scope is invalid, unknown, malformed, or exceeds the
    /// scope granted by the resource owner (RFC 6749 §5.2).
    #[error("invalid_scope")]
    InvalidScope,
    /// The requested resource indicator is not a valid audience for this
    /// authorization server (RFC 8707 §2).
    #[error("invalid_target")]
    InvalidTarget,
    /// The resource owner or authorization server denied the request
    /// (RFC 6749 §4.1.2.1).
    #[error("access_denied")]
    AccessDenied,
    /// The authorization server encountered an unexpected condition that
    /// prevented it from fulfilling the request (RFC 6749 §5.2).
    #[error("server_error")]
    ServerError,
    /// The authorization server is currently unable to handle the request
    /// due to a temporary overload or maintenance (RFC 6749 §5.2).
    #[error("temporarily_unavailable")]
    TemporarilyUnavailable,
    /// The access token does not carry sufficient scope for the requested
    /// resource (RFC 6750 §3.1).
    #[error("insufficient_scope")]
    InsufficientScope,
    /// Database error encountered while servicing the request. Bridged via
    /// `impl_report_conversion!` — no `#[from]` here.
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),
}

impl OAuthError {
    /// Returns the RFC 6749 §5.2 / RFC 8707 §2 error code string.
    ///
    /// Database errors are reported to clients as the generic `server_error`
    /// to avoid leaking internal failure detail across the wire.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            OAuthError::InvalidRequest(_) => "invalid_request",
            OAuthError::InvalidClient => "invalid_client",
            OAuthError::InvalidGrant(_) => "invalid_grant",
            OAuthError::UnauthorizedClient => "unauthorized_client",
            OAuthError::UnsupportedGrantType => "unsupported_grant_type",
            OAuthError::InvalidScope => "invalid_scope",
            OAuthError::InvalidTarget => "invalid_target",
            OAuthError::AccessDenied => "access_denied",
            OAuthError::ServerError => "server_error",
            OAuthError::TemporarilyUnavailable => "temporarily_unavailable",
            OAuthError::InsufficientScope => "insufficient_scope",
            OAuthError::Database(_) => "server_error",
        }
    }

    /// Returns the HTTP status code paired with this OAuth error per
    /// RFC 6749 §5.2.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            OAuthError::InvalidRequest(_)
            | OAuthError::InvalidGrant(_)
            | OAuthError::UnsupportedGrantType
            | OAuthError::InvalidScope
            | OAuthError::InvalidTarget => 400,
            OAuthError::InvalidClient | OAuthError::UnauthorizedClient => 401,
            OAuthError::AccessDenied | OAuthError::InsufficientScope => 403,
            OAuthError::ServerError | OAuthError::Database(_) => 500,
            OAuthError::TemporarilyUnavailable => 503,
        }
    }
}

impl_report_conversion!(sea_orm::DbErr => OAuthError::Database);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6749_codes_match_spec() {
        assert_eq!(
            OAuthError::InvalidGrant("test").error_code(),
            "invalid_grant"
        );
        assert_eq!(OAuthError::InvalidTarget.error_code(), "invalid_target");
        assert_eq!(OAuthError::InvalidClient.error_code(), "invalid_client");
        assert_eq!(
            OAuthError::InsufficientScope.error_code(),
            "insufficient_scope"
        );
    }

    #[test]
    fn http_status_mapping_correct() {
        assert_eq!(OAuthError::InvalidGrant("x").http_status(), 400);
        assert_eq!(OAuthError::InvalidClient.http_status(), 401);
        assert_eq!(OAuthError::AccessDenied.http_status(), 403);
        assert_eq!(OAuthError::InsufficientScope.http_status(), 403);
        assert_eq!(OAuthError::ServerError.http_status(), 500);
        assert_eq!(OAuthError::TemporarilyUnavailable.http_status(), 503);
    }
}
