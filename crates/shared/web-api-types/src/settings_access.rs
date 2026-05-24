use serde::{Deserialize, Serialize};

use crate::registration::RegistrationMode;
use crate::validation::{Validate, ValidationError};
use uptrakit_shared_types::SecretString;

// No #[non_exhaustive] — UpdateAccessSettingsRequest is constructed in external crates
// (openapi-client, CLI, tests) via struct literals; #[non_exhaustive] would break that.
// AccessSettingsResponse carries #[non_exhaustive] because it is never constructed externally.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAccessSettingsRequest {
    pub mode: RegistrationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<SecretString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_token_for_oidc: Option<bool>,
    pub password_auth_enabled: Option<bool>,
    pub two_factor_required: Option<bool>,
}

impl Validate for UpdateAccessSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        match self.mode {
            RegistrationMode::Invite => {
                if self.token.is_none() {
                    return Err(ValidationError {
                        field: "token",
                        message: "required when mode is invite".to_string(),
                    });
                }
            }
            _ => {
                if self.token.is_some() {
                    return Err(ValidationError {
                        field: "token",
                        message: "only valid when mode is invite".to_string(),
                    });
                }
                if self.require_token_for_oidc.is_some() {
                    return Err(ValidationError {
                        field: "require_token_for_oidc",
                        message: "only valid when mode is invite".to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccessSettingsResponse {
    pub mode: RegistrationMode,
    pub require_token_for_oidc: bool,
    pub password_auth_enabled: bool,
    pub two_factor_required: bool,
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn validate_invite_requires_token() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Invite,
            token: None,
            require_token_for_oidc: None,
            password_auth_enabled: None,
            two_factor_required: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_invite_with_token_ok() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Invite,
            token: Some(SecretString::new("abc".to_string())),
            require_token_for_oidc: None,
            password_auth_enabled: None,
            two_factor_required: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_open_with_token_rejected() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Open,
            token: Some(SecretString::new("abc".to_string())),
            require_token_for_oidc: None,
            password_auth_enabled: None,
            two_factor_required: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_closed_no_token_ok() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Closed,
            token: None,
            require_token_for_oidc: None,
            password_auth_enabled: Some(true),
            two_factor_required: None,
        };
        assert!(req.validate().is_ok());
    }
}
