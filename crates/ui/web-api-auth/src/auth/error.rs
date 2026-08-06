use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

/// Source-carrying variants deliberately omit `#[from]`: every conversion into
/// `AuthError` goes through `.context_to()` via the `impl_report_conversion!`
/// blocks below, so a derived `From` impl would be dead code (error-handling.md
/// bans carrying both).
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
    PasswordHash(argon2::password_hash::Error),

    #[error("token generation failed: {0}")]
    TokenGeneration(String),

    #[error("database error: {0}")]
    Database(sea_orm::DbErr),

    #[error("UUID parsing error: {0}")]
    UuidParse(uuid::Error),

    #[error("time error: {0}")]
    TimeError(time::error::ComponentRange),

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

    #[error("IO error: {0}")]
    Io(std::io::Error),

    #[error("invalid session: auth method data is corrupted or inconsistent")]
    InvalidSession,

    #[error("internal error: {0}")]
    Internal(String),

    #[error("MFA challenge not found or already consumed")]
    MfaChallengeNotFound,

    #[error("MFA challenge has expired")]
    MfaChallengeExpired,

    #[error("MFA challenge exhausted (too many failed attempts)")]
    MfaChallengeExhausted,

    #[error("MFA code is invalid")]
    MfaCodeInvalid,

    #[error("Email delivery unavailable")]
    EmailDeliveryUnavailable,

    /// Lockout-guard failure during OIDC role sync.
    #[error("access guard error: {0}")]
    AccessGuard(uptrakit_shared_db::access_grants::AccessGrantError),
}

pub type Result<T> = std::result::Result<T, Report<AuthError>>;

impl_report_conversion! {
    sea_orm::DbErr              => AuthError::Database,
    argon2::password_hash::Error => AuthError::PasswordHash,
    uuid::Error                 => AuthError::UuidParse,
    time::error::ComponentRange => AuthError::TimeError,
    std::io::Error              => AuthError::Io,
}

impl_report_conversion!(uptrakit_shared_db::access_grants::AccessGrantError => AuthError::AccessGuard);

impl_report_conversion!(argon2::Error => AuthError, |_| AuthError::Internal("argon2 error".to_string()));
