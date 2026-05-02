//! Request/response types for the zeroconf settings API.
//!
//! `GET /api/v1/settings/zeroconf` returns [`ZeroconfSettingsResponse`].
//! `PUT /api/v1/settings/zeroconf` accepts [`UpdateZeroconfSettingsRequest`].
//!
//! Zeroconf settings control automatic service discovery and enrollment via
//! mDNS/DNS-SD. The `url` field specifies the controller URL that agents
//! advertise, and `pki_addr` specifies the PKI endpoint for certificate
//! retrieval.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

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

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    // ── ZeroconfSettingsResponse ─────────────────────────────────────

    #[test]
    fn response_round_trip_all_fields() {
        let resp = ZeroconfSettingsResponse {
            enabled: true,
            url: Some("https://controller.example.com".to_string()),
            pki_addr: Some("https://pki.example.com".to_string()),
            ca_fingerprint: Some("SHA256:abc123".to_string()),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: ZeroconfSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(de.enabled);
        assert_eq!(de.url.as_deref(), Some("https://controller.example.com"));
        assert_eq!(de.pki_addr.as_deref(), Some("https://pki.example.com"));
        assert_eq!(de.ca_fingerprint.as_deref(), Some("SHA256:abc123"));
    }

    #[test]
    fn response_round_trip_none_fields() {
        let resp = ZeroconfSettingsResponse {
            enabled: false,
            url: None,
            pki_addr: None,
            ca_fingerprint: None,
        };
        let json = serde_json::to_value(&resp).expect("serialization should succeed");
        assert_eq!(json["enabled"], false);
        assert!(json.get("url").is_none(), "absent url must not appear");
        assert!(
            json.get("pki_addr").is_none(),
            "absent pki_addr must not appear"
        );
        assert!(
            json.get("ca_fingerprint").is_none(),
            "absent ca_fingerprint must not appear"
        );
    }

    // ── UpdateZeroconfSettingsRequest ────────────────────────────────

    #[test]
    fn update_request_round_trip() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: Some(true),
            url: Some("https://controller.example.com".to_string()),
            pki_addr: Some("https://pki.example.com".to_string()),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateZeroconfSettingsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.enabled, Some(true));
        assert_eq!(de.url.as_deref(), Some("https://controller.example.com"));
        assert_eq!(de.pki_addr.as_deref(), Some("https://pki.example.com"));
    }

    // ── Validation: accepts valid inputs ────────────────────────────

    #[test]
    fn validate_accepts_none_fields() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: None,
            pki_addr: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_valid_https_url() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: Some(true),
            url: Some("https://controller.example.com".to_string()),
            pki_addr: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_empty_url_passthrough() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: Some(String::new()),
            pki_addr: None,
        };
        assert!(
            req.validate().is_ok(),
            "empty string should pass through (clears url)"
        );
    }

    #[test]
    fn validate_accepts_http_pki_addr() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: None,
            pki_addr: Some("http://pki.local".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_https_pki_addr() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: None,
            pki_addr: Some("https://pki.example.com".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_empty_pki_addr_passthrough() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: None,
            pki_addr: Some(String::new()),
        };
        assert!(
            req.validate().is_ok(),
            "empty string should pass through (clears pki_addr)"
        );
    }

    // ── Validation: rejects invalid inputs ──────────────────────────

    #[test]
    fn validate_rejects_non_https_url() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: Some("http://controller.example.com".to_string()),
            pki_addr: None,
        };
        let err = req.validate().expect_err("should reject http:// url");
        assert_eq!(err.field, "url");
        assert!(err.message.contains("https://"));
    }

    #[test]
    fn validate_rejects_wrong_url_scheme() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: Some("ftp://controller.example.com".to_string()),
            pki_addr: None,
        };
        let err = req.validate().expect_err("should reject ftp:// url");
        assert_eq!(err.field, "url");
        assert!(err.message.contains("https://"));
    }

    #[test]
    fn validate_rejects_invalid_pki_addr_scheme() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: None,
            pki_addr: Some("ftp://bad.example.com".to_string()),
        };
        let err = req.validate().expect_err("should reject ftp:// pki_addr");
        assert_eq!(err.field, "pki_addr");
        assert!(err.message.contains("http://"));
    }

    #[test]
    fn validate_rejects_bare_hostname_url() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: Some("controller.example.com".to_string()),
            pki_addr: None,
        };
        let err = req.validate().expect_err("should reject bare hostname");
        assert_eq!(err.field, "url");
    }

    #[test]
    fn validate_rejects_bare_hostname_pki_addr() {
        let req = UpdateZeroconfSettingsRequest {
            enabled: None,
            url: None,
            pki_addr: Some("pki.example.com".to_string()),
        };
        let err = req
            .validate()
            .expect_err("should reject bare hostname pki_addr");
        assert_eq!(err.field, "pki_addr");
    }
}
