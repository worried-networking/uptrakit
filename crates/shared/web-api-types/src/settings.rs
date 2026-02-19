use crate::registration::RegistrationMode;
use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegistrationSettingsResponse {
    pub mode: RegistrationMode,
    /// Whether OIDC users also need a registration token (only relevant in `invite` mode).
    pub require_token_for_oidc: bool,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateRegistrationSettingsRequest {
    pub mode: RegistrationMode,
    /// Required when mode is `invite`. The plaintext token will be hashed before storage.
    pub token: Option<SecretString>,
    /// Whether OIDC users also need a registration token (only relevant in `invite` mode).
    pub require_token_for_oidc: Option<bool>,
}
