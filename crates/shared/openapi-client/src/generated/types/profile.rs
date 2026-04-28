use crate::generated::shared_types::SecretString;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
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
