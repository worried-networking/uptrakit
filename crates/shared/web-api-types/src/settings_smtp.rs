//! Request/response types for the SMTP settings API.
//!
//! `GET /api/v1/settings/smtp` returns [`SmtpSettingsResponse`].
//! `PUT /api/v1/settings/smtp` accepts [`UpdateSmtpSettingsRequest`].
//!
//! The SMTP password is never returned in the response; `has_password` indicates
//! whether one is configured.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

/// Response body for `GET /api/v1/settings/smtp`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SmtpSettingsResponse {
    /// SMTP server hostname or IP address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// SMTP server port (e.g. 587 for STARTTLS, 465 for TLS, 25 for plain).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// SMTP authentication username.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Whether an SMTP password is configured (password itself is never returned).
    pub has_password: bool,
    /// Envelope sender address (e.g. `notifications@example.com`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,
    /// Optional human-readable sender name (e.g. `Uptrakit Notifications`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
    /// TLS mode: `"starttls"` (default), `"tls"`, or `"none"`.
    pub tls_mode: String,
    /// Optional EHLO hostname override for the SMTP EHLO command.
    ///
    /// When absent, the email plugin derives the hostname from the `from_address` domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helo_host: Option<String>,
}

/// Request body for `PUT /api/v1/settings/smtp`.
///
/// All fields are optional; omitting a field leaves the current value
/// unchanged. Nullable fields (`username`, `password`, `from_name`) accept
/// either a string value or JSON `null` to clear the stored value.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateSmtpSettingsRequest {
    /// SMTP server hostname or IP address. Non-empty if provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// SMTP server port (1–65535).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// SMTP authentication username. `null` = clear, omit = keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<serde_json::Value>,
    /// SMTP authentication password. `null` = clear, omit = keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<serde_json::Value>,
    /// Envelope sender address. Non-empty and must contain `@` if provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,
    /// Sender display name. `null` = clear, omit = keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_name: Option<serde_json::Value>,
    /// TLS mode: `"starttls"` (default), `"tls"`, or `"none"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_mode: Option<String>,
    /// SMTP EHLO hostname override. `null` = clear, omit = keep existing.
    ///
    /// When set, this value is sent in the SMTP EHLO command instead of the domain
    /// derived from `from_address`. Useful when using a relay that requires a specific FQDN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helo_host: Option<serde_json::Value>,
}

impl Validate for UpdateSmtpSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref host) = self.host {
            if host.is_empty() {
                return Err(ValidationError {
                    field: "host",
                    message: "must not be empty".to_string(),
                });
            }
            if host.len() > 253 {
                return Err(ValidationError {
                    field: "host",
                    message: "must not exceed 253 characters".to_string(),
                });
            }
        }

        if let Some(ref val) = self.username {
            if let Some(s) = val.as_str() {
                if s.len() > 256 {
                    return Err(ValidationError {
                        field: "username",
                        message: "must not exceed 256 characters".to_string(),
                    });
                }
            } else if !val.is_null() {
                return Err(ValidationError {
                    field: "username",
                    message: "must be a string or null".to_string(),
                });
            }
        }

        if let Some(ref val) = self.password {
            if let Some(s) = val.as_str() {
                if s.len() > 256 {
                    return Err(ValidationError {
                        field: "password",
                        message: "must not exceed 256 characters".to_string(),
                    });
                }
            } else if !val.is_null() {
                return Err(ValidationError {
                    field: "password",
                    message: "must be a string or null".to_string(),
                });
            }
        }

        if let Some(ref from_address) = self.from_address {
            if from_address.is_empty() {
                return Err(ValidationError {
                    field: "from_address",
                    message: "must not be empty".to_string(),
                });
            }
            if !from_address.contains('@') {
                return Err(ValidationError {
                    field: "from_address",
                    message: "must be a valid email address".to_string(),
                });
            }
            if from_address.len() > 254 {
                return Err(ValidationError {
                    field: "from_address",
                    message: "must not exceed 254 characters".to_string(),
                });
            }
        }

        if let Some(ref val) = self.from_name {
            if let Some(s) = val.as_str() {
                if s.len() > 256 {
                    return Err(ValidationError {
                        field: "from_name",
                        message: "must not exceed 256 characters".to_string(),
                    });
                }
            } else if !val.is_null() {
                return Err(ValidationError {
                    field: "from_name",
                    message: "must be a string or null".to_string(),
                });
            }
        }

        if let Some(ref tls_mode) = self.tls_mode
            && !matches!(tls_mode.as_str(), "starttls" | "tls" | "none")
        {
            return Err(ValidationError {
                field: "tls_mode",
                message: "must be one of: starttls, tls, none".to_string(),
            });
        }

        if let Some(ref val) = self.helo_host {
            if let Some(s) = val.as_str() {
                if s.len() > 253 {
                    return Err(ValidationError {
                        field: "helo_host",
                        message: "must not exceed 253 characters".to_string(),
                    });
                }
            } else if !val.is_null() {
                return Err(ValidationError {
                    field: "helo_host",
                    message: "must be a string or null".to_string(),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_req() -> UpdateSmtpSettingsRequest {
        UpdateSmtpSettingsRequest {
            host: None,
            port: None,
            username: None,
            password: None,
            from_address: None,
            from_name: None,
            tls_mode: None,
            helo_host: None,
        }
    }

    #[test]
    fn validate_accepts_all_none() {
        assert!(empty_req().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_host() {
        let req = UpdateSmtpSettingsRequest {
            host: Some(String::new()),
            ..empty_req()
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "host");
    }

    #[test]
    fn validate_rejects_from_address_without_at() {
        let req = UpdateSmtpSettingsRequest {
            from_address: Some("notanemail".to_string()),
            ..empty_req()
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "from_address");
    }

    #[test]
    fn validate_rejects_empty_from_address() {
        let req = UpdateSmtpSettingsRequest {
            from_address: Some(String::new()),
            ..empty_req()
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "from_address");
    }

    #[test]
    fn validate_rejects_invalid_tls_mode() {
        let req = UpdateSmtpSettingsRequest {
            tls_mode: Some("ssl".to_string()),
            ..empty_req()
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "tls_mode");
        assert!(err.message.contains("starttls"));
    }

    #[test]
    fn validate_accepts_valid_tls_modes() {
        for mode in ["starttls", "tls", "none"] {
            let req = UpdateSmtpSettingsRequest {
                tls_mode: Some(mode.to_string()),
                ..empty_req()
            };
            assert!(req.validate().is_ok(), "mode {mode} should be valid");
        }
    }

    #[test]
    fn validate_accepts_valid_from_address() {
        let req = UpdateSmtpSettingsRequest {
            from_address: Some("notifications@example.com".to_string()),
            ..empty_req()
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_rejects_oversized_username() {
        let req = UpdateSmtpSettingsRequest {
            username: Some(serde_json::Value::String("x".repeat(257))),
            ..empty_req()
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "username");
    }

    #[test]
    fn validate_rejects_non_string_username() {
        let req = UpdateSmtpSettingsRequest {
            username: Some(serde_json::json!(42)),
            ..empty_req()
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "username");
    }

    #[test]
    fn validate_accepts_null_password() {
        // null = clear stored password
        let req = UpdateSmtpSettingsRequest {
            password: Some(serde_json::Value::Null),
            ..empty_req()
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn smtp_settings_response_round_trip() {
        let resp = SmtpSettingsResponse {
            host: Some("smtp.example.com".to_string()),
            port: Some(587),
            username: Some("user@example.com".to_string()),
            has_password: true,
            from_address: Some("noreply@example.com".to_string()),
            from_name: Some("Uptrakit".to_string()),
            tls_mode: "starttls".to_string(),
            helo_host: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SmtpSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.host.as_deref(), Some("smtp.example.com"));
        assert_eq!(deserialized.port, Some(587));
        assert!(deserialized.has_password);
        assert_eq!(deserialized.tls_mode, "starttls");
    }
}
