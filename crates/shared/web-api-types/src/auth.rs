use crate::permissions::Permission;
use crate::validation::{Validate, ValidationError};
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

impl Validate for RegisterRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.email.len() > 254 {
            return Err(ValidationError {
                field: "email",
                message: "email must not exceed 254 characters".to_string(),
            });
        }
        if !self.email.contains('@') {
            return Err(ValidationError {
                field: "email",
                message: "email must contain '@'".to_string(),
            });
        }
        if self.first_name.is_empty() {
            return Err(ValidationError {
                field: "first_name",
                message: "first_name must not be empty".to_string(),
            });
        }
        let password_len = self.password.expose_secret().len();
        if password_len < 8 {
            return Err(ValidationError {
                field: "password",
                message: "password must be at least 8 characters".to_string(),
            });
        }
        if password_len > 1024 {
            return Err(ValidationError {
                field: "password",
                message: "password must not exceed 1024 characters".to_string(),
            });
        }
        Ok(())
    }
}

impl Validate for LoginRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.email.len() > 254 {
            return Err(ValidationError {
                field: "email",
                message: "email must not exceed 254 characters".to_string(),
            });
        }
        if !self.email.contains('@') {
            return Err(ValidationError {
                field: "email",
                message: "email must contain '@'".to_string(),
            });
        }
        if self.password.expose_secret().is_empty() {
            return Err(ValidationError {
                field: "password",
                message: "password must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RegisterRequest ──────────────────────────────────────────────────────

    fn valid_register() -> RegisterRequest {
        RegisterRequest {
            email: "user@example.com".to_string(),
            first_name: "Alice".to_string(),
            last_name: "Smith".to_string(),
            password: SecretString::new("password123"),
            registration_token: None,
        }
    }

    #[test]
    fn register_valid() {
        assert!(valid_register().validate().is_ok());
    }

    #[test]
    fn register_email_too_long() {
        let mut req = valid_register();
        req.email = format!("{}@x.com", "a".repeat(250));
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "email");
    }

    #[test]
    fn register_email_no_at_sign() {
        let mut req = valid_register();
        req.email = "notanemail".to_string();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "email");
    }

    #[test]
    fn register_first_name_empty() {
        let mut req = valid_register();
        req.first_name = String::new();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "first_name");
    }

    #[test]
    fn register_password_too_short() {
        let mut req = valid_register();
        req.password = SecretString::new("short");
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    #[test]
    fn register_password_too_long() {
        let mut req = valid_register();
        req.password = SecretString::new("a".repeat(1025));
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    // ── LoginRequest ─────────────────────────────────────────────────────────

    fn valid_login() -> LoginRequest {
        LoginRequest {
            email: "user@example.com".to_string(),
            password: SecretString::new("password123"),
        }
    }

    #[test]
    fn login_valid() {
        assert!(valid_login().validate().is_ok());
    }

    #[test]
    fn login_email_too_long() {
        let mut req = valid_login();
        req.email = format!("{}@x.com", "a".repeat(250));
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "email");
    }

    #[test]
    fn login_email_no_at_sign() {
        let mut req = valid_login();
        req.email = "notanemail".to_string();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "email");
    }

    #[test]
    fn login_password_empty() {
        let mut req = valid_login();
        req.password = SecretString::new(String::new());
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }
}
