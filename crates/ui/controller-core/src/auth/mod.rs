pub mod api_token;
pub mod denylist;
pub mod device_flow;
pub mod jwt;
pub mod rate_limit;

pub use uptrakit_web_api_auth::auth::device_flow::DeviceFlowStore;
pub use uptrakit_web_api_auth::auth::jwt::JwtManager;
pub use uptrakit_web_api_auth::auth::permissions::Permission;
pub use uptrakit_web_api_auth::auth::rate_limit::RateLimitStore;
pub use uptrakit_web_api_auth::auth::token_denylist::TokenDenylist;
pub use uptrakit_web_api_auth::auth::{AuthError, AuthMethod};

/// Struct holding the result of a successful authentication attempt.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (e.g. scope, sub claim).
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
    pub permissions: Vec<Permission>,
    /// JTI of the JWT access token, if authenticated via JWT (None for API token auth).
    pub jti: Option<String>,
}

impl AuthenticatedUser {
    /// Creates a new [`AuthenticatedUser`].
    pub fn new(
        user_id: uuid::Uuid,
        auth_method: AuthMethod,
        permissions: Vec<Permission>,
        jti: Option<String>,
    ) -> Self {
        Self {
            user_id,
            auth_method,
            permissions,
            jti,
        }
    }

    /// Returns `true` if the authenticated user holds the given permission.
    pub fn has_permission(&self, perm: Permission) -> bool {
        self.permissions.contains(&perm)
    }

    /// Returns the `(actor_type, actor_id)` pair for audit log entries.
    pub fn audit_actor(
        &self,
        api_token_id: Option<AuthenticatedApiTokenId>,
    ) -> (uptrakit_audit_log::AuditActorType, Option<uuid::Uuid>) {
        use uptrakit_audit_log::AuditActorType;
        match self.auth_method {
            AuthMethod::ApiToken => (AuditActorType::ApiToken, api_token_id.map(|t| t.0)),
            _ => {
                tracing::warn!(
                    auth_method = ?self.auth_method,
                    "unknown AuthMethod variant in audit_actor; defaulting to User actor type"
                );
                (AuditActorType::User, Some(self.user_id))
            }
        }
    }
}

/// Newtype for an authenticated API token ID — preserves type safety at audit boundaries.
#[derive(Clone, Copy, Debug)]
pub struct AuthenticatedApiTokenId(pub uuid::Uuid);

/// Failure variants returned by authentication helpers.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add new rejection cases.
#[non_exhaustive]
#[derive(Debug)]
pub enum AuthFailure {
    InvalidApiToken,
    UserNotFound,
    UserDeactivated,
    InvalidOrExpiredToken,
    InvalidTokenSubject,
    TokenRevoked,
    InvalidOidcSessionMissingProvider,
    InternalError,
}

impl AuthFailure {
    /// Returns a short reason code for audit log `details`, if applicable.
    pub fn api_token_reason_code(&self) -> Option<&'static str> {
        match self {
            Self::InvalidApiToken => Some("invalid_or_revoked_api_token"),
            Self::UserNotFound => Some("user_not_found"),
            Self::UserDeactivated => Some("user_deactivated"),
            Self::InternalError => Some("internal_error"),
            _ => None,
        }
    }

    /// Returns audit attributes for JWT authentication failures, if applicable.
    ///
    /// Returns `None` for failures that are not JWT-related (e.g. invalid API token).
    pub fn jwt_audit_attributes(
        &self,
    ) -> Option<(
        uptrakit_audit_log::AuditActorType,
        uptrakit_audit_log::AuditOutcome,
        &'static str,
    )> {
        match self {
            Self::InvalidOrExpiredToken => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_or_expired_token",
            )),
            Self::InvalidTokenSubject => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_token_subject",
            )),
            Self::TokenRevoked => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "token_revoked",
            )),
            Self::InvalidOidcSessionMissingProvider => Some((
                uptrakit_audit_log::AuditActorType::Oidc,
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_oidc_session_missing_provider",
            )),
            Self::UserNotFound => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_not_found",
            )),
            Self::UserDeactivated => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_deactivated",
            )),
            Self::InternalError => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Failed,
                "jwt_authenticate_failed",
            )),
            Self::InvalidApiToken => None,
        }
    }
}

#[cfg(feature = "axum-integration")]
impl axum::response::IntoResponse for AuthFailure {
    fn into_response(self) -> axum::response::Response {
        use axum::Json;
        use axum::http::StatusCode;
        use uptrakit_web_api_types::error::ErrorResponse;

        let (status, message) = match self {
            Self::InvalidApiToken => (StatusCode::UNAUTHORIZED, "Invalid or revoked API token"),
            Self::UserNotFound => (StatusCode::UNAUTHORIZED, "User not found"),
            Self::UserDeactivated => (StatusCode::FORBIDDEN, "User is deactivated"),
            Self::InvalidOrExpiredToken => (StatusCode::UNAUTHORIZED, "Invalid or expired token"),
            Self::InvalidTokenSubject => (StatusCode::UNAUTHORIZED, "Invalid token subject"),
            Self::TokenRevoked => (StatusCode::UNAUTHORIZED, "Token has been revoked"),
            Self::InvalidOidcSessionMissingProvider => (
                StatusCode::UNAUTHORIZED,
                "Invalid OIDC session: missing provider",
            ),
            Self::InternalError => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
        };
        (
            status,
            Json(ErrorResponse {
                error: message.to_string(),
                code: None,
            }),
        )
            .into_response()
    }
}
