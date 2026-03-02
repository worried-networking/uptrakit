//! Request/response types for the system services settings API.
//!
//! `GET /api/v1/settings/system-services` returns [`SystemServicesSettingsResponse`].
//! `PUT /api/v1/settings/system-services` accepts [`UpdateSystemServicesSettingsRequest`].
//!
//! The enrollment token is a shared secret that system services (e.g. the MQTT bridge)
//! must supply during enrollment to be automatically approved. It is stored encrypted at
//! rest with the master key. Unlike SMTP passwords, it **is** returned in API responses
//! so that operators can copy it into service deployment configurations.
//!
//! When no token is configured (`has_token: false`), all system service enrollments
//! are placed in `Pending` status and require manual approval.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

/// Response body for `GET /api/v1/settings/system-services`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemServicesSettingsResponse {
    /// Plaintext enrollment token, or `null` when no token is configured.
    ///
    /// When present, system services that supply this token during enrollment are
    /// automatically approved. When absent, all enrollments require manual approval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment_token: Option<String>,
    /// Whether an enrollment token is currently configured.
    pub has_token: bool,
}

/// Request body for `PUT /api/v1/settings/system-services`.
///
/// The `enrollment_token` field is a nullable JSON value:
/// - omit (absent from JSON) — keep the current value unchanged
/// - `null` — clear the stored token (all future enrollments will require manual approval)
/// - `"some-token"` — set a new token (encrypted at rest)
///
/// Maximum token length is 512 characters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateSystemServicesSettingsRequest {
    /// Enrollment token. `null` = clear, omit = keep existing.
    ///
    /// Must not be empty if provided as a string. Maximum 512 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment_token: Option<serde_json::Value>,
}

impl Validate for UpdateSystemServicesSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref val) = self.enrollment_token {
            if val.is_null() {
                // Null is valid — it clears the token
                return Ok(());
            }
            if let Some(s) = val.as_str() {
                if s.is_empty() {
                    return Err(ValidationError {
                        field: "enrollment_token",
                        message: "must not be empty".to_string(),
                    });
                }
                if s.len() > 512 {
                    return Err(ValidationError {
                        field: "enrollment_token",
                        message: "must not exceed 512 characters".to_string(),
                    });
                }
            } else {
                return Err(ValidationError {
                    field: "enrollment_token",
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

    fn empty_req() -> UpdateSystemServicesSettingsRequest {
        UpdateSystemServicesSettingsRequest {
            enrollment_token: None,
        }
    }

    #[test]
    fn validate_accepts_all_none() {
        assert!(empty_req().validate().is_ok());
    }

    #[test]
    fn validate_accepts_null() {
        let req = UpdateSystemServicesSettingsRequest {
            enrollment_token: Some(serde_json::Value::Null),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_valid_token() {
        for token in ["abc123", "my-enrollment-token", "x".repeat(512).as_str()] {
            let req = UpdateSystemServicesSettingsRequest {
                enrollment_token: Some(serde_json::json!(token)),
            };
            assert!(req.validate().is_ok(), "should accept token of length {}", token.len());
        }
    }

    #[test]
    fn validate_rejects_empty_string() {
        let req = UpdateSystemServicesSettingsRequest {
            enrollment_token: Some(serde_json::json!("")),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "enrollment_token");
        assert!(err.message.contains("empty"));
    }

    #[test]
    fn validate_rejects_non_string() {
        let req = UpdateSystemServicesSettingsRequest {
            enrollment_token: Some(serde_json::json!(42)),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "enrollment_token");
        assert!(err.message.contains("string or null"));
    }

    #[test]
    fn validate_rejects_oversized_token() {
        let long_token = "x".repeat(513);
        let req = UpdateSystemServicesSettingsRequest {
            enrollment_token: Some(serde_json::Value::String(long_token)),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "enrollment_token");
        assert!(err.message.contains("512"));
    }

    #[test]
    fn response_serialization_with_token() {
        let resp = SystemServicesSettingsResponse {
            enrollment_token: Some("my-token".to_string()),
            has_token: true,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["enrollment_token"], "my-token");
        assert_eq!(json["has_token"], true);
    }

    #[test]
    fn response_serialization_no_token() {
        let resp = SystemServicesSettingsResponse {
            enrollment_token: None,
            has_token: false,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert!(json.get("enrollment_token").is_none(), "absent token must not appear");
        assert_eq!(json["has_token"], false);
    }
}
