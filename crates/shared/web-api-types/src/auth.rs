use crate::permissions::Permission;
use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = json!({
    "email": "admin@example.com",
    "first_name": "Admin",
    "last_name": "User",
    "password": "SecurePass123"
})))]
pub struct RegisterRequest {
    #[cfg_attr(feature = "openapi", schema(example = "admin@example.com"))]
    pub email: String,
    #[cfg_attr(feature = "openapi", schema(example = "Admin"))]
    pub first_name: String,
    #[cfg_attr(feature = "openapi", schema(example = "User"))]
    pub last_name: String,
    #[cfg_attr(feature = "openapi", schema(example = "SecurePass123", min_length = 8))]
    pub password: SecretString,
    /// Required when registration mode is `invite`.
    pub registration_token: Option<SecretString>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = json!({
    "email": "admin@example.com",
    "password": "SecurePass123"
})))]
pub struct LoginRequest {
    #[cfg_attr(feature = "openapi", schema(example = "admin@example.com"))]
    pub email: String,
    #[cfg_attr(feature = "openapi", schema(example = "SecurePass123"))]
    pub password: SecretString,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogoutRequest {
    /// The refresh token to revoke. Optional when the token is provided
    /// via the `refresh_token` `HttpOnly` cookie.
    #[serde(default)]
    pub refresh_token: Option<SecretString>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RefreshRequest {
    /// The refresh token. Optional when the token is provided via the
    /// `refresh_token` `HttpOnly` cookie.
    #[serde(default)]
    pub refresh_token: Option<SecretString>,
}

#[derive(Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthResponse {
    pub access_token: SecretString,
    pub refresh_token: SecretString,
    pub expires_in: i64,
    pub token_type: String,
    pub user: UserResponse,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RefreshResponse {
    pub access_token: SecretString,
    /// Rotated refresh token. The previous refresh token is now invalid.
    pub refresh_token: SecretString,
    pub expires_in: i64,
    pub token_type: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub permissions: Vec<Permission>,
}
