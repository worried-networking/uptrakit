use crate::registration::RegistrationMode;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegistrationSettingsResponse {
    pub mode: RegistrationMode,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateRegistrationSettingsRequest {
    pub mode: RegistrationMode,
    /// Required when mode is `invite`. The plaintext token will be hashed before storage.
    pub token: Option<String>,
}
