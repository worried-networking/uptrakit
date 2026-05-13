use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthenticationSettingsResponse {
    pub password_auth_enabled: bool,
    pub two_factor_required: bool,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAuthenticationSettingsRequest {
    pub password_auth_enabled: Option<bool>,
    pub two_factor_required: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_settings_response_round_trip() {
        let resp = AuthenticationSettingsResponse {
            password_auth_enabled: true,
            two_factor_required: false,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: AuthenticationSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(de.password_auth_enabled);
        assert!(!de.two_factor_required);
    }

    #[test]
    fn update_auth_settings_request_round_trip() {
        let req = UpdateAuthenticationSettingsRequest {
            password_auth_enabled: Some(false),
            two_factor_required: Some(true),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateAuthenticationSettingsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.password_auth_enabled, Some(false));
        assert_eq!(de.two_factor_required, Some(true));
    }
}
