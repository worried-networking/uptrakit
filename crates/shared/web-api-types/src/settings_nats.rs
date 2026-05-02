//! Request/response types for the NATS URL settings API.
//!
//! `GET /api/v1/settings/nats` returns [`NatsSettingsResponse`].
//! `PUT /api/v1/settings/nats` accepts [`UpdateNatsSettingsRequest`].
//!
//! The NATS URL may contain embedded credentials (`nats://user:password@host`).
//! The password is **never** returned in the response; the [`MaskedUrl`] type
//! automatically redacts the password in all serialized output.
//!
//! **Hot-reload is not supported.** Changes to the NATS URL take effect after
//! the controller is restarted.

use serde::{Deserialize, Serialize};

use crate::masked_url::MaskedUrl;
use crate::validation::{Validate, ValidationError};

/// Response body for `GET /api/v1/settings/nats`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NatsSettingsResponse {
    /// NATS URL with any password component redacted, e.g. `nats://user:***@host:4222`.
    /// `None` when no NATS URL is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<MaskedUrl>,
    /// Whether a NATS URL is currently configured.
    pub has_url: bool,
}

/// Request body for `PUT /api/v1/settings/nats`.
///
/// The `url` field is a nullable JSON value:
/// - omit (absent from JSON) — keep the current value unchanged
/// - `null` — clear the stored NATS URL (disables NATS after next restart)
/// - `"nats://…"` — set a new URL (encrypted at rest)
///
/// Changes take effect after the controller is restarted.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateNatsSettingsRequest {
    /// NATS server URL. `null` = clear, omit = keep existing.
    ///
    /// Must start with `nats://` or `nats-tls://`. Maximum 1024 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<serde_json::Value>,
}

impl Validate for UpdateNatsSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref val) = self.url {
            if val.is_null() {
                // Null is valid — it clears the URL
                return Ok(());
            }
            if let Some(s) = val.as_str() {
                if s.is_empty() {
                    return Err(ValidationError {
                        field: "url",
                        message: "must not be empty".to_string(),
                    });
                }
                if s.len() > 1024 {
                    return Err(ValidationError {
                        field: "url",
                        message: "must not exceed 1024 characters".to_string(),
                    });
                }
                if !s.starts_with("nats://") && !s.starts_with("nats-tls://") {
                    return Err(ValidationError {
                        field: "url",
                        message: "must start with nats:// or nats-tls://".to_string(),
                    });
                }
            } else {
                return Err(ValidationError {
                    field: "url",
                    message: "must be a string or null".to_string(),
                });
            }
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

    fn empty_req() -> UpdateNatsSettingsRequest {
        UpdateNatsSettingsRequest { url: None }
    }

    #[test]
    fn validate_accepts_all_none() {
        assert!(empty_req().validate().is_ok());
    }

    #[test]
    fn validate_accepts_null() {
        let req = UpdateNatsSettingsRequest {
            url: Some(serde_json::Value::Null),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_valid_nats_url() {
        for url in ["nats://host:4222", "nats-tls://user:pw@host:4222"] {
            let req = UpdateNatsSettingsRequest {
                url: Some(serde_json::json!(url)),
            };
            assert!(req.validate().is_ok(), "should accept {url}");
        }
    }

    #[test]
    fn validate_rejects_empty_string() {
        let req = UpdateNatsSettingsRequest {
            url: Some(serde_json::json!("")),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "url");
        assert!(err.message.contains("empty"));
    }

    #[test]
    fn validate_rejects_wrong_scheme() {
        let req = UpdateNatsSettingsRequest {
            url: Some(serde_json::json!("mqtt://host:1883")),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "url");
        assert!(err.message.contains("nats://"));
    }

    #[test]
    fn validate_rejects_non_string() {
        let req = UpdateNatsSettingsRequest {
            url: Some(serde_json::json!(42)),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "url");
        assert!(err.message.contains("string or null"));
    }

    #[test]
    fn validate_rejects_oversized_url() {
        let long_url = format!("nats://host:4222/{}", "x".repeat(1100));
        let req = UpdateNatsSettingsRequest {
            url: Some(serde_json::Value::String(long_url)),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "url");
        assert!(err.message.contains("1024"));
    }

    #[test]
    fn response_serialization_masks_password() {
        let resp = NatsSettingsResponse {
            url: Some(MaskedUrl::new("nats://user:secret@host:4222")),
            has_url: true,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        let url_str = json["url"].as_str().expect("url field");
        assert!(!url_str.contains("secret"), "password must be masked");
        assert!(url_str.contains("***"), "masked placeholder must appear");
        assert_eq!(json["has_url"], true);
    }

    #[test]
    fn response_serialization_no_url() {
        let resp = NatsSettingsResponse {
            url: None,
            has_url: false,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert!(json.get("url").is_none(), "absent url must not appear");
        assert_eq!(json["has_url"], false);
    }
}
