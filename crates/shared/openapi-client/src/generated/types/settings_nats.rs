// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
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
use crate::generated::types::masked_url::MaskedUrl;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
/// Response body for `GET /api/v1/settings/nats`.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
