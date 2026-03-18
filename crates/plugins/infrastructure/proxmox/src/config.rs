use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType};
use uptrakit_plugin_infrastructure_core::{PluginConfig, SecretString};
use url::Url;

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

fn default_true() -> bool {
    true
}

/// Configuration for the Proxmox VE infrastructure plugin.
///
/// Stores connection details for a Proxmox VE API endpoint. The plugin uses
/// API token authentication (`PVEAPIToken=USER@REALM!TOKENID=SECRET`).
///
/// Private/loopback hosts are explicitly allowed since Proxmox VE is typically
/// deployed on-premise.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxmoxConfig {
    /// Proxmox VE API URL (e.g., `"https://pve.local:8006"`).
    pub api_url: String,
    /// API token in PVE format: `"USER@REALM!TOKENID=SECRET"`.
    pub api_token: SecretString,
    /// Verify TLS certificates (default: `true`).
    ///
    /// Set to `false` for self-signed certificates common in on-premise PVE
    /// installations.
    #[serde(default = "default_true")]
    pub verify_tls: bool,
    /// Restrict discovery to these Proxmox nodes (empty = all nodes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_filter: Vec<String>,
}

impl Default for ProxmoxConfig {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            api_token: SecretString::new(String::new()),
            verify_tls: true,
            node_filter: vec![],
        }
    }
}

impl PluginConfig for ProxmoxConfig {
    fn validate(&self) -> Result<(), String> {
        // Validate api_url
        if self.api_url.is_empty() {
            return Err("api_url is required".to_string());
        }
        let parsed = Url::parse(&self.api_url).map_err(|e| format!("invalid api_url: {e}"))?;
        if parsed.scheme() != "https" {
            return Err("api_url must use https".to_string());
        }
        if parsed.host_str().is_none() {
            return Err("api_url must include a host".to_string());
        }

        // Validate api_token format: USER@REALM!TOKENID=SECRET
        let token = self.api_token.expose_secret();
        if !token.is_empty() && !is_valid_pve_token(token) {
            return Err("api_token must be in PVE format: USER@REALM!TOKENID=SECRET".to_string());
        }

        Ok(())
    }

    fn with_secrets_masked(mut self) -> Self {
        self.api_token = SecretString::new(SECRET_MASK);
        self
    }

    fn restore_secrets_from(&mut self, existing: &Self) {
        if self.api_token.expose_secret() == SECRET_MASK {
            self.api_token = existing.api_token.clone();
        }
    }

    fn form_schema() -> Vec<FieldDef> {
        vec![
            FieldDef::new("api_url", "API URL")
                .required()
                .with_placeholder("https://pve.example.com:8006")
                .with_help_text("Proxmox VE API endpoint URL"),
            FieldDef::new("api_token", "API Token")
                .with_type(FieldType::Password)
                .required()
                .sensitive()
                .with_placeholder("USER@REALM!TOKENID=SECRET")
                .with_help_text("PVE API token in USER@REALM!TOKENID=SECRET format"),
            FieldDef::new("verify_tls", "Verify TLS")
                .with_type(FieldType::Toggle)
                .with_default_value(serde_json::json!(true))
                .with_help_text("Verify TLS certificates (disable for self-signed certs)"),
            FieldDef::new("node_filter", "Node Filter")
                .with_type(FieldType::Textarea)
                .list()
                .with_help_text(
                    "Restrict discovery to these node names (one per line, empty = all)",
                ),
        ]
    }
}

impl ProxmoxConfig {
    /// Validate the configuration using the rich error type.
    ///
    /// This method returns the full `ProxmoxError`-based `Result` for use
    /// in internal plugin code that wants structured errors. The
    /// `PluginConfig::validate` trait method delegates to this and maps to
    /// `String`.
    pub fn validate_rich(&self) -> crate::error::Result<()> {
        use crate::error::ProxmoxError;
        use rootcause::prelude::*;

        // Validate api_url
        if self.api_url.is_empty() {
            bail!(ProxmoxError::Configuration(
                "api_url is required".to_string()
            ));
        }
        let parsed = Url::parse(&self.api_url)
            .map_err(|e| report!(ProxmoxError::Configuration(format!("invalid api_url: {e}"))))?;
        if parsed.scheme() != "https" {
            bail!(ProxmoxError::Configuration(
                "api_url must use https".to_string()
            ));
        }
        if parsed.host_str().is_none() {
            bail!(ProxmoxError::Configuration(
                "api_url must include a host".to_string()
            ));
        }

        // Validate api_token format: USER@REALM!TOKENID=SECRET
        let token = self.api_token.expose_secret();
        if !token.is_empty() && !is_valid_pve_token(token) {
            bail!(ProxmoxError::Configuration(
                "api_token must be in PVE format: USER@REALM!TOKENID=SECRET".to_string()
            ));
        }

        Ok(())
    }

    /// Validate a Proxmox package identifier (unused — Proxmox plugin has no
    /// package identifiers).
    pub fn validate_identifier(_value: &str) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// Check if a string matches the PVE API token format.
///
/// Expected format: `USER@REALM!TOKENID=UUID-OR-SECRET`
fn is_valid_pve_token(token: &str) -> bool {
    // Must contain @ (realm separator) and ! (token separator) and = (secret separator)
    let Some(at_pos) = token.find('@') else {
        return false;
    };
    let after_at = &token[at_pos + 1..];
    let Some(bang_pos) = after_at.find('!') else {
        return false;
    };
    let after_bang = &after_at[bang_pos + 1..];
    let Some(eq_pos) = after_bang.find('=') else {
        return false;
    };

    // All parts must be non-empty
    let user = &token[..at_pos];
    let realm = &after_at[..bang_pos];
    let token_id = &after_bang[..eq_pos];
    let secret = &after_bang[eq_pos + 1..];

    !user.is_empty() && !realm.is_empty() && !token_id.is_empty() && !secret.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_invalid() {
        let config = ProxmoxConfig::default();
        assert!(config.validate().is_err(), "empty api_url should fail");
    }

    #[test]
    fn valid_config() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new(
                "root@pam!mytoken=12345678-1234-1234-1234-123456789012".to_string(),
            ),
            verify_tls: false,
            node_filter: vec![],
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_rejects_http() {
        let config = ProxmoxConfig {
            api_url: "http://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=secret"),
            ..ProxmoxConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("https"));
    }

    #[test]
    fn validation_rejects_invalid_token_format() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("just-a-random-string"),
            ..ProxmoxConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("PVE format"));
    }

    #[test]
    fn pve_token_validation() {
        assert!(is_valid_pve_token("root@pam!mytoken=secret"));
        assert!(is_valid_pve_token(
            "user@pve!token=12345678-1234-1234-1234-123456789012"
        ));
        assert!(!is_valid_pve_token("nope"));
        assert!(!is_valid_pve_token("user@realm!token"));
        assert!(!is_valid_pve_token("@realm!token=secret"));
        assert!(!is_valid_pve_token("user@!token=secret"));
        assert!(!is_valid_pve_token("user@realm!=secret"));
    }

    #[test]
    fn secret_masking() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=real-secret"),
            ..ProxmoxConfig::default()
        };
        let masked = config.clone().with_secrets_masked();
        assert_eq!(masked.api_token.expose_secret(), SECRET_MASK);
        assert_eq!(masked.api_url, "https://pve.local:8006");
    }

    #[test]
    fn restore_secrets() {
        let existing = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=real-secret"),
            ..ProxmoxConfig::default()
        };
        let mut incoming = existing.clone().with_secrets_masked();
        incoming.restore_secrets_from(&existing);
        assert_eq!(
            incoming.api_token.expose_secret(),
            "root@pam!tok=real-secret"
        );
    }

    #[test]
    fn restore_secrets_keeps_new_token() {
        let existing = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=old-secret"),
            ..ProxmoxConfig::default()
        };
        let mut incoming = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=new-secret"),
            ..ProxmoxConfig::default()
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(
            incoming.api_token.expose_secret(),
            "root@pam!tok=new-secret"
        );
    }

    #[test]
    fn serialization_roundtrip() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=secret"),
            verify_tls: false,
            node_filter: vec!["pve1".to_string()],
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ProxmoxConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.api_url, config.api_url);
        assert_eq!(
            deserialized.api_token.expose_secret(),
            config.api_token.expose_secret()
        );
        assert_eq!(deserialized.verify_tls, config.verify_tls);
        assert_eq!(deserialized.node_filter, config.node_filter);
    }

    #[test]
    fn verify_tls_defaults_to_true() {
        let config: ProxmoxConfig =
            serde_json::from_str(r#"{"api_url":"https://pve:8006","api_token":"tok"}"#)
                .expect("deserialize");
        assert!(config.verify_tls);
    }

    #[test]
    fn private_hosts_allowed() {
        let config = ProxmoxConfig {
            api_url: "https://192.168.1.1:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=secret"),
            ..ProxmoxConfig::default()
        };
        assert!(
            config.validate().is_ok(),
            "private hosts should be allowed for Proxmox"
        );
    }
}
