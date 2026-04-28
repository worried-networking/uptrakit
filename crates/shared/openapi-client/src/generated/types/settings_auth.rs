use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthenticationSettingsResponse {
    pub password_auth_enabled: bool,
}
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAuthenticationSettingsRequest {
    pub password_auth_enabled: Option<bool>,
}
