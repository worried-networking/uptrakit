use crate::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use uptrakit_shared_types::{MaskedEmail, SecretString};
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
    #[cfg_attr(
        feature = "openapi",
        schema(example = "admin@example.com", value_type = String)
    )]
    pub email: MaskedEmail,
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
    #[cfg_attr(
        feature = "openapi",
        schema(example = "admin@example.com", value_type = String)
    )]
    pub email: MaskedEmail,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
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

/// Whether the access engine resolved this principal's authority.
///
/// Deliberately a closed two-variant enum (no `#[non_exhaustive]`, no
/// `Other`): the set is definitionally complete — the engine either
/// resolved grants or it did not — matching the closed-verdict-set
/// precedent rather than the wire-safe open-enum rule, which targets
/// vocabularies that can grow (spec §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AuthorityStatus {
    Ok,
    Unavailable,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    /// Expanded effective action list: wildcards expanded against the
    /// catalog, dynamic actions included per live registries. Token-scope
    /// intersection applies once scoped credentials exist (M3) — pre-M3
    /// session credentials carry no scope. Empty when `authority` is
    /// `unavailable`.
    ///
    /// Deliberately `Vec<String>`, not `Vec<Action>`: `Action`'s
    /// deserializer rejects any resource/verb the COMPILED catalog lacks,
    /// so a typed field would make a newer controller's response
    /// unparseable to an older client (CLI could not even log in) the
    /// moment the catalog grows. The action set is open — clients treat
    /// entries as opaque strings.
    pub actions: Vec<String>,
    /// Whether the access engine resolved this principal's authority for
    /// this response. `unavailable` ⇒ `actions` is empty and the client
    /// should degrade, not log out.
    pub authority: AuthorityStatus,
    pub has_pending_email_change: bool,
}

impl Validate for RegisterRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        // Email format and length are enforced by the MaskedEmail parse at
        // deserialization; nothing left to check here.
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
        // Email format and length are enforced by the MaskedEmail parse at
        // deserialization; nothing left to check here.
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
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    // ── RegisterRequest ──────────────────────────────────────────────────────

    fn valid_register() -> RegisterRequest {
        RegisterRequest {
            email: "user@example.com".parse().expect("valid test email"),
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
    fn register_rejects_email_without_at() {
        let json = r#"{"email":"notanemail","first_name":"Alice","last_name":"Smith","password":"password123"}"#;
        assert!(serde_json::from_str::<RegisterRequest>(json).is_err());
    }

    #[test]
    fn register_rejects_over_long_email() {
        let json = format!(
            r#"{{"email":"{}@x.com","first_name":"Alice","last_name":"Smith","password":"password123"}}"#,
            "a".repeat(uptrakit_shared_types::MAX_EMAIL_LEN)
        );
        assert!(serde_json::from_str::<RegisterRequest>(&json).is_err());
    }

    #[test]
    fn register_deserialize_canonicalizes_email() {
        let json = r#"{"email":" User@Example.COM ","first_name":"Alice","last_name":"Smith","password":"password123"}"#;
        let req: RegisterRequest = serde_json::from_str(json).expect("valid");
        assert_eq!(req.email.expose_email(), "user@example.com");
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
            email: "user@example.com".parse().expect("valid test email"),
            password: SecretString::new("password123"),
        }
    }

    #[test]
    fn login_valid() {
        assert!(valid_login().validate().is_ok());
    }

    #[test]
    fn login_rejects_email_without_at() {
        let json = r#"{"email":"notanemail","password":"password123"}"#;
        assert!(serde_json::from_str::<LoginRequest>(json).is_err());
    }

    #[test]
    fn login_rejects_over_long_email() {
        let json = format!(
            r#"{{"email":"{}@x.com","password":"password123"}}"#,
            "a".repeat(uptrakit_shared_types::MAX_EMAIL_LEN)
        );
        assert!(serde_json::from_str::<LoginRequest>(&json).is_err());
    }

    #[test]
    fn login_deserialize_canonicalizes_email() {
        let json = r#"{"email":" User@Example.COM ","password":"password123"}"#;
        let req: LoginRequest = serde_json::from_str(json).expect("valid");
        assert_eq!(req.email.expose_email(), "user@example.com");
    }

    #[test]
    fn login_password_empty() {
        let mut req = valid_login();
        req.password = SecretString::new(String::new());
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }
}
