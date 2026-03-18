//! Per-channel configuration for the Email notification plugin.

use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::PluginConfig;

/// Minimal email format validation: must contain exactly one `@` with
/// non-empty local and domain parts and at least one `.` in the domain.
pub(crate) fn is_valid_email(addr: &str) -> bool {
    let Some((local, domain)) = addr.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

/// Per-channel config for email notification channels.
///
/// Contains only `to_addresses`. SMTP settings are merged from global/tenant
/// settings at delivery time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmailChannelConfig {
    /// Recipient email addresses for this channel.
    #[serde(default)]
    pub to_addresses: Vec<String>,
}

impl PluginConfig for EmailChannelConfig {
    fn validate(&self) -> Result<(), String> {
        if self.to_addresses.is_empty() {
            return Err("'to_addresses' must not be empty".to_string());
        }
        for addr in &self.to_addresses {
            if !is_valid_email(addr) {
                return Err(format!("invalid email address: '{addr}'"));
            }
        }
        Ok(())
    }

    // No secrets in per-channel config -- SMTP credentials are in global settings.
    // Default `with_secrets_masked()` and `restore_secrets_from()` are correct.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_to_addresses() {
        let cfg = EmailChannelConfig::default();
        let err = cfg.validate().unwrap_err();
        assert!(
            err.contains("to_addresses"),
            "expected to_addresses mention, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_invalid_email_format() {
        let cfg = EmailChannelConfig {
            to_addresses: vec!["not-an-email".to_string()],
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.contains("invalid email address"),
            "expected invalid email error, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_email_without_dot_in_domain() {
        let cfg = EmailChannelConfig {
            to_addresses: vec!["user@nodomain".to_string()],
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.contains("invalid email address"),
            "expected invalid email error, got: {err}"
        );
    }

    #[test]
    fn validate_accepts_valid_config() {
        let cfg = EmailChannelConfig {
            to_addresses: vec!["user@example.com".to_string()],
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_accepts_multiple_valid_addresses() {
        let cfg = EmailChannelConfig {
            to_addresses: vec![
                "alice@example.com".to_string(),
                "bob@example.org".to_string(),
            ],
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn is_valid_email_accepts_standard_addresses() {
        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("user+tag@sub.domain.org"));
        assert!(is_valid_email("a@b.io"));
    }

    #[test]
    fn is_valid_email_rejects_no_at_sign() {
        assert!(!is_valid_email("notanemail"));
        assert!(!is_valid_email("no-at-sign.com"));
    }

    #[test]
    fn is_valid_email_rejects_empty_local_or_domain() {
        assert!(!is_valid_email("@domain.com"));
        assert!(!is_valid_email("local@"));
    }

    #[test]
    fn is_valid_email_rejects_domain_without_dot() {
        assert!(!is_valid_email("user@nodomain"));
    }

    #[test]
    fn mask_config_secrets_returns_config_unchanged() {
        let cfg = EmailChannelConfig {
            to_addresses: vec!["user@example.com".to_string()],
        };
        let masked = cfg.clone().with_secrets_masked();
        let original_json = serde_json::to_value(&cfg).expect("serialize");
        let masked_json = serde_json::to_value(masked).expect("serialize");
        assert_eq!(
            original_json, masked_json,
            "per-channel config has no secrets to mask"
        );
    }

    #[test]
    fn deserialize_empty_object() {
        let cfg: EmailChannelConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(cfg.to_addresses.is_empty());
    }

    #[test]
    fn serialization_roundtrip() {
        let cfg = EmailChannelConfig {
            to_addresses: vec!["a@b.com".to_string(), "c@d.org".to_string()],
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: EmailChannelConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.to_addresses, cfg.to_addresses);
    }
}
