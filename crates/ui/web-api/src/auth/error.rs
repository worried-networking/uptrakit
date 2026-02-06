use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("session not found or expired")]
    SessionExpired,

    #[error("user not found: {0}")]
    UserNotFound(String),

    #[error("user is deactivated")]
    UserDeactivated,

    #[error("password hashing error: {0}")]
    PasswordHash(#[from] argon2::password_hash::Error),

    #[error("token generation failed: {0}")]
    TokenGeneration(String),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("UUID parsing error: {0}")]
    UuidParse(#[from] uuid::Error),

    #[error("time error: {0}")]
    TimeError(#[from] time::error::ComponentRange),

    #[error("OIDC provider not found or inactive")]
    OidcProviderNotFound,

    #[error("OIDC discovery failed: {0}")]
    OidcDiscovery(String),

    #[error("OIDC token exchange failed: {0}")]
    OidcTokenExchange(String),

    #[error("OIDC token validation failed: {0}")]
    OidcTokenValidation(String),

    #[error("OIDC state not found or expired")]
    OidcStateNotFound,

    #[error("OIDC account not found and auto-creation disabled")]
    OidcNoAccount,

    #[error("OIDC account linking required")]
    OidcLinkRequired,

    #[error("OIDC link verification failed")]
    OidcLinkVerificationFailed,

    #[error("password authentication is disabled")]
    PasswordAuthDisabled,

    #[error("cannot disable the auth method used by the current session")]
    CannotDisableOwnAuthMethod,

    #[error("at least one auth method must remain enabled")]
    NoAuthMethodsRemaining,

    #[error("JWT encoding failed: {0}")]
    JwtEncode(String),

    #[error("JWT validation failed: {0}")]
    JwtDecode(String),

    #[error("invalid refresh token")]
    InvalidRefreshToken,

    #[error("refresh token expired")]
    RefreshTokenExpired,

    #[error("refresh token revoked")]
    RefreshTokenRevoked,

    #[error("API token not found")]
    ApiTokenNotFound,

    #[error("API token has been revoked")]
    ApiTokenRevoked,

    #[error("device flow not found or expired")]
    DeviceFlowNotFound,

    #[error("device flow already authorized")]
    DeviceFlowAlreadyAuthorized,

    #[error("device flow polling too fast")]
    DeviceFlowRateLimited,

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Report<AuthError>>;

impl_report_conversion! {
    sea_orm::DbErr              => AuthError::Database,
    argon2::password_hash::Error => AuthError::PasswordHash,
    uuid::Error                 => AuthError::UuidParse,
    time::error::ComponentRange => AuthError::TimeError,
}

impl_report_conversion!(argon2::Error => AuthError, |_| AuthError::Internal("argon2 error".to_string()));
