//! Per-channel configuration for the Webhook notification plugin.

use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

/// Header names that are always rejected in webhook custom headers.
///
/// These headers could be used for credential injection, host header
/// poisoning, or IP spoofing if an attacker controls the header values.
pub(crate) const BLOCKED_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "host",
    "proxy-authorization",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
];

/// Returns an error if `key` matches any entry in [`BLOCKED_HEADERS`].
///
/// Used in both [`PluginConfig::validate`] (structural validation) and
/// the delivery path (defence-in-depth).
pub(crate) fn check_header_allowed(key: &str) -> Result<(), PluginConfigValidationError> {
    let lower = key.to_lowercase();
    if BLOCKED_HEADERS.contains(&lower.as_str()) {
        return Err(PluginConfigValidationError::invalid_field(
            "headers",
            format!("header '{key}' is not allowed in webhook custom headers"),
        ));
    }
    Ok(())
}

/// Per-channel config for webhook notification channels.
///
/// Stored in the `notification_channels` table's `config` JSON column.
/// Validated via the [`PluginConfig`] trait implementation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebhookChannelConfig {
    /// Target URL to POST the notification payload to.
    #[serde(default)]
    pub url: String,
    /// Optional HMAC-SHA256 secret for request signing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// Optional custom HTTP headers added to the webhook request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<serde_json::Map<String, serde_json::Value>>,
}

impl PluginConfig for WebhookChannelConfig {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if self.url.is_empty() {
            return Err(PluginConfigValidationError::invalid_field(
                "url",
                "is required",
            ));
        }
        if !self.url.starts_with("http://") && !self.url.starts_with("https://") {
            return Err(PluginConfigValidationError::invalid_field(
                "url",
                "must start with http:// or https://",
            ));
        }
        if let Some(headers) = &self.headers {
            for key in headers.keys() {
                check_header_allowed(key)?;
            }
        }
        Ok(())
    }

    fn with_secrets_masked(mut self) -> Self {
        if self.secret.is_some() {
            self.secret = Some("***".to_string());
        }
        self
    }

    fn restore_secrets_from(&mut self, existing: &Self) {
        if self.secret.as_deref() == Some("***") {
            self.secret = existing.secret.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_url() {
        let cfg = WebhookChannelConfig::default();
        assert!(cfg.validate().is_err());
        let msg = cfg.validate().unwrap_err();
        assert_eq!(msg.field(), Some("url"));
        assert!(msg.to_string().contains("is required"), "got: {msg}");
    }

    #[test]
    fn validate_rejects_non_http_url() {
        let cfg = WebhookChannelConfig {
            url: "ftp://example.com".to_string(),
            ..Default::default()
        };
        let msg = cfg.validate().unwrap_err();
        assert_eq!(msg.field(), Some("url"));
        assert!(
            msg.to_string().contains("http:// or https://"),
            "got: {msg}"
        );
    }

    #[test]
    fn validate_accepts_https_url() {
        let cfg = WebhookChannelConfig {
            url: "https://example.com/hook".to_string(),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_accepts_http_url() {
        let cfg = WebhookChannelConfig {
            url: "http://example.com/hook".to_string(),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_blocked_header() {
        let mut headers = serde_json::Map::new();
        headers.insert(
            "Authorization".to_string(),
            serde_json::Value::String("Bearer token".to_string()),
        );
        let cfg = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            headers: Some(headers),
            ..Default::default()
        };
        let msg = cfg.validate().unwrap_err();
        assert_eq!(msg.field(), Some("headers"));
        assert!(msg.to_string().contains("Authorization"), "got: {msg}");
    }

    #[test]
    fn validate_accepts_custom_header() {
        let mut headers = serde_json::Map::new();
        headers.insert(
            "X-Custom".to_string(),
            serde_json::Value::String("val".to_string()),
        );
        let cfg = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            headers: Some(headers),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn mask_secrets_replaces_secret() {
        let cfg = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            secret: Some("super-secret".to_string()),
            ..Default::default()
        };
        let masked = cfg.with_secrets_masked();
        assert_eq!(masked.secret.as_deref(), Some("***"));
        assert_eq!(masked.url, "https://example.com");
    }

    #[test]
    fn mask_secrets_preserves_none_secret() {
        let cfg = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            secret: None,
            ..Default::default()
        };
        let masked = cfg.with_secrets_masked();
        assert!(masked.secret.is_none());
    }

    #[test]
    fn restore_secrets_replaces_masked_value() {
        let existing = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            secret: Some("real-secret".to_string()),
            ..Default::default()
        };
        let mut incoming = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            secret: Some("***".to_string()),
            ..Default::default()
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.secret.as_deref(), Some("real-secret"));
    }

    #[test]
    fn restore_secrets_keeps_new_value() {
        let existing = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            secret: Some("old-secret".to_string()),
            ..Default::default()
        };
        let mut incoming = WebhookChannelConfig {
            url: "https://example.com".to_string(),
            secret: Some("new-secret".to_string()),
            ..Default::default()
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.secret.as_deref(), Some("new-secret"));
    }

    #[test]
    fn check_header_allowed_rejects_all_blocked() {
        for blocked in BLOCKED_HEADERS {
            let result = check_header_allowed(blocked);
            assert!(result.is_err(), "should reject '{blocked}'");
            let msg = result.unwrap_err();
            assert!(
                msg.to_string().contains(blocked),
                "error should mention header; got: {msg}"
            );
        }
    }

    #[test]
    fn check_header_allowed_case_insensitive() {
        assert!(check_header_allowed("Authorization").is_err());
        assert!(check_header_allowed("AUTHORIZATION").is_err());
        assert!(check_header_allowed("AuThOrIzAtIoN").is_err());
    }

    #[test]
    fn check_header_allowed_permits_custom_headers() {
        assert!(check_header_allowed("X-Custom-Header").is_ok());
        assert!(check_header_allowed("X-Api-Key").is_ok());
        assert!(check_header_allowed("Accept").is_ok());
    }
}
