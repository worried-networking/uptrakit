//! Request/response types for the zeroconf settings API.
//!
//! `GET /api/v1/settings/zeroconf` returns [`ZeroconfSettingsResponse`].
//! `PUT /api/v1/settings/zeroconf` accepts [`UpdateZeroconfSettingsRequest`].
//!
//! Zeroconf settings control automatic service discovery and enrollment via
//! mDNS/DNS-SD. The `url` field specifies the controller URL that agents
//! advertise, and `pki_addr` specifies the PKI endpoint for certificate
//! retrieval.
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
/// Response body for `GET /api/v1/settings/zeroconf`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ZeroconfSettingsResponse {
    /// Whether zeroconf discovery is enabled.
    pub enabled: bool,
    /// Controller URL advertised via zeroconf. `None` when not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// PKI endpoint URL embedded in zeroconf announcements. `None` when not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pki_addr: Option<String>,
    /// CA certificate fingerprint included in zeroconf announcements for
    /// trust-on-first-use verification. `None` when no CA is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_fingerprint: Option<String>,
}
/// Request body for `PUT /api/v1/settings/zeroconf`.
///
/// All fields are optional — omitted fields keep their current value.
///
/// - `enabled`: `true` to enable, `false` to disable zeroconf discovery.
/// - `url`: empty string clears the value; non-empty must start with `https://`.
/// - `pki_addr`: empty string clears the value; non-empty must start with
///   `http://` or `https://`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateZeroconfSettingsRequest {
    /// Whether zeroconf discovery is enabled. `None` = keep existing.
    pub enabled: Option<bool>,
    /// Controller URL advertised via zeroconf.
    /// Empty string clears the value, `None` = keep existing.
    /// Must start with `https://` when non-empty.
    pub url: Option<String>,
    /// PKI endpoint URL for zeroconf announcements.
    /// Empty string clears the value, `None` = keep existing.
    /// Must start with `http://` or `https://` when non-empty.
    pub pki_addr: Option<String>,
}
impl Validate for UpdateZeroconfSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref url) = self.url
            && !url.is_empty()
            && !url.starts_with("https://")
        {
            return Err(ValidationError {
                field: "url",
                message: "must start with https://".to_string(),
            });
        }
        if let Some(ref addr) = self.pki_addr
            && !addr.is_empty()
            && !addr.starts_with("http://")
            && !addr.starts_with("https://")
        {
            return Err(ValidationError {
                field: "pki_addr",
                message: "must start with http:// or https://".to_string(),
            });
        }
        Ok(())
    }
}
