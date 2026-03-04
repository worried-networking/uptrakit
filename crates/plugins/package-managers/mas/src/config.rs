use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Configuration for the Mac App Store (`mas`) plugin.
///
/// `mas` manages App Store apps identified by their numeric App Store ID
/// (e.g. `497799835` for Xcode). There are no per-config secrets or
/// filtering options — all App Store apps share the same discovery mechanism.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasConfig {}

impl SecretMasking for MasConfig {}

impl MasConfig {
    /// Validate a Mac App Store package identifier string.
    ///
    /// Delegates to the crate-level [`validate_identifier`](crate::validate_identifier)
    /// function. A valid identifier is a non-empty, all-digit string of at most 15
    /// characters (App Store IDs are 9–10 digits as of 2025).
    ///
    /// Called by the plugin registry's `validate_package_identifier` dispatch.
    pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

    /// Validate the configuration.
    ///
    /// `MasConfig` has no fields that require validation; always returns `Ok(())`.
    pub fn validate(&self) -> crate::error::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_empty() {
        let config = MasConfig::default();
        assert_eq!(config, MasConfig {});
    }

    #[test]
    fn deserialize_empty_object() {
        let config: MasConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config, MasConfig {});
    }

    #[test]
    fn serialization_roundtrip() {
        let config = MasConfig {};
        let json = serde_json::to_value(&config).expect("serialize");
        let deserialized: MasConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = MasConfig::default();
        assert!(config.validate().is_ok());
    }
}
