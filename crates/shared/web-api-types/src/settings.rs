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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_settings_response_round_trip() {
        let resp = RegistrationSettingsResponse {
            mode: RegistrationMode::Invite,
            require_token_for_oidc: true,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: RegistrationSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.mode, RegistrationMode::Invite);
        assert!(de.require_token_for_oidc);
    }

    #[test]
    fn update_registration_settings_request_round_trip() {
        let req = UpdateRegistrationSettingsRequest {
            mode: RegistrationMode::Open,
            token: Some(SecretString::new("invite-token".to_string())),
            require_token_for_oidc: Some(false),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateRegistrationSettingsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.mode, RegistrationMode::Open);
        assert_eq!(
            de.token.as_ref().map(|s| s.expose_secret()),
            Some("invite-token")
        );
        assert_eq!(de.require_token_for_oidc, Some(false));
    }
}
