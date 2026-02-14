use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_provider_core::ProviderError;

/// Configuration for the Proxmox Helper Scripts provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxmoxHelperScriptsConfig {
    /// URL of the Proxmox helper script to execute for updates.
    pub script_url: String,
}

impl ProxmoxHelperScriptsConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> uptrakit_provider_core::Result<()> {
        if self.script_url.is_empty() {
            bail!(ProviderError::MissingConfig("script_url".to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_config() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/script.sh".to_string(),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn empty_script_url_fails() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: String::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn deserialization() {
        let json = r#"{"script_url":"https://example.com/update.sh"}"#;
        let config: ProxmoxHelperScriptsConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.script_url, "https://example.com/update.sh");
    }
}
