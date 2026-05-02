use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;

use crate::validation::{Validate, ValidationError};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateProfileRequest {
    #[cfg_attr(feature = "openapi", schema(example = "Jane"))]
    pub first_name: String,
    #[cfg_attr(feature = "openapi", schema(example = "Doe"))]
    pub last_name: String,
}

impl Validate for UpdateProfileRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.first_name.is_empty() {
            return Err(ValidationError {
                field: "first_name",
                message: "first_name must not be empty".to_string(),
            });
        }
        if self.first_name.len() > 100 {
            return Err(ValidationError {
                field: "first_name",
                message: "first_name must not exceed 100 characters".to_string(),
            });
        }
        if self.last_name.len() > 100 {
            return Err(ValidationError {
                field: "last_name",
                message: "last_name must not exceed 100 characters".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InitiateEmailChangeRequest {
    #[cfg_attr(feature = "openapi", schema(example = "currentpassword123"))]
    pub current_password: SecretString,
    #[cfg_attr(feature = "openapi", schema(example = "newemail@example.com"))]
    pub new_email: String,
}

impl Validate for InitiateEmailChangeRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if !self.new_email.contains('@') {
            return Err(ValidationError {
                field: "new_email",
                message: "new_email must contain '@'".to_string(),
            });
        }
        if self.new_email.len() > 254 {
            return Err(ValidationError {
                field: "new_email",
                message: "new_email must not exceed 254 characters".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChangePasswordRequest {
    #[cfg_attr(feature = "openapi", schema(example = "currentpassword123"))]
    pub current_password: SecretString,
    #[cfg_attr(
        feature = "openapi",
        schema(example = "newpassword123", min_length = 8)
    )]
    pub new_password: SecretString,
}

impl Validate for ChangePasswordRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        let len = self.new_password.expose_secret().len();
        if len < 8 {
            return Err(ValidationError {
                field: "new_password",
                message: "new_password must be at least 8 characters".to_string(),
            });
        }
        if len > 128 {
            return Err(ValidationError {
                field: "new_password",
                message: "new_password must not exceed 128 characters".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    // ── UpdateProfileRequest ─────────────────────────────────────────────────

    fn valid_update_profile() -> UpdateProfileRequest {
        UpdateProfileRequest {
            first_name: "Jane".to_string(),
            last_name: "Doe".to_string(),
        }
    }

    #[test]
    fn update_profile_valid() {
        assert!(valid_update_profile().validate().is_ok());
    }

    #[test]
    fn update_profile_empty_first_name_fails() {
        let mut req = valid_update_profile();
        req.first_name = String::new();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "first_name");
    }

    #[test]
    fn update_profile_first_name_too_long() {
        let mut req = valid_update_profile();
        req.first_name = "a".repeat(101);
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "first_name");
    }

    #[test]
    fn update_profile_last_name_too_long() {
        let mut req = valid_update_profile();
        req.last_name = "a".repeat(101);
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "last_name");
    }

    #[test]
    fn update_profile_empty_last_name_ok() {
        let req = UpdateProfileRequest {
            first_name: "Jane".to_string(),
            last_name: String::new(),
        };
        assert!(req.validate().is_ok());
    }

    // ── InitiateEmailChangeRequest ───────────────────────────────────────────

    fn valid_email_change() -> InitiateEmailChangeRequest {
        InitiateEmailChangeRequest {
            current_password: SecretString::new("currentpassword123"),
            new_email: "newemail@example.com".to_string(),
        }
    }

    #[test]
    fn initiate_email_change_valid() {
        assert!(valid_email_change().validate().is_ok());
    }

    #[test]
    fn initiate_email_change_missing_at_fails() {
        let mut req = valid_email_change();
        req.new_email = "notanemail".to_string();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "new_email");
    }

    #[test]
    fn initiate_email_change_email_too_long() {
        let mut req = valid_email_change();
        req.new_email = format!("{}@x.com", "a".repeat(250));
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "new_email");
    }

    // ── ChangePasswordRequest ────────────────────────────────────────────────

    fn valid_change_password() -> ChangePasswordRequest {
        ChangePasswordRequest {
            current_password: SecretString::new("oldpassword123"),
            new_password: SecretString::new("newpassword123"),
        }
    }

    #[test]
    fn change_password_valid() {
        assert!(valid_change_password().validate().is_ok());
    }

    #[test]
    fn change_password_too_short_fails() {
        let mut req = valid_change_password();
        req.new_password = SecretString::new("short");
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "new_password");
    }

    #[test]
    fn change_password_too_long_fails() {
        let mut req = valid_change_password();
        req.new_password = SecretString::new("a".repeat(129));
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "new_password");
    }

    #[test]
    fn change_password_exactly_8_chars_ok() {
        let req = ChangePasswordRequest {
            current_password: SecretString::new("oldpass"),
            new_password: SecretString::new("12345678"),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn change_password_exactly_128_chars_ok() {
        let req = ChangePasswordRequest {
            current_password: SecretString::new("oldpass"),
            new_password: SecretString::new("a".repeat(128)),
        };
        assert!(req.validate().is_ok());
    }
}
