use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Configuration for the npm package manager plugin.
///
/// No secrets — npm registry queries are unauthenticated public API calls.
/// The `package_identifier` in `SoftwareItem` is the npm package name
/// (e.g., `n8n`, `@angular/cli`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpmConfig {
    /// Include pre-release dist-tags (`next`, `beta`, `alpha`, `rc`, `canary`)
    /// in `fetch_releases` results.
    ///
    /// When `false` (default), only the `latest` dist-tag is returned.
    #[serde(default)]
    pub include_prereleases: bool,
}

impl SecretMasking for NpmConfig {}

impl NpmConfig {
    /// Validate the configuration.
    ///
    /// Currently accepts all valid deserialized configs.
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_include_prereleases_is_false() {
        let config = NpmConfig::default();
        assert!(!config.include_prereleases);
    }

    #[test]
    fn deserialize_empty_object() {
        let config: NpmConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(!config.include_prereleases);
    }

    #[test]
    fn deserialize_include_prereleases_true() {
        let config: NpmConfig =
            serde_json::from_str(r#"{"include_prereleases": true}"#).expect("deserialize");
        assert!(config.include_prereleases);
    }

    #[test]
    fn deserialize_include_prereleases_false() {
        let config: NpmConfig =
            serde_json::from_str(r#"{"include_prereleases": false}"#).expect("deserialize");
        assert!(!config.include_prereleases);
    }

    #[test]
    fn serialization_roundtrip_default() {
        let config = NpmConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        let deserialized: NpmConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_prereleases_enabled() {
        let config = NpmConfig {
            include_prereleases: true,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["include_prereleases"], true);
        let deserialized: NpmConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        assert!(NpmConfig::default().validate().is_ok());
    }

    #[test]
    fn secret_masking_is_noop() {
        use uptrakit_plugin_infrastructure_core::SecretMasking;
        let config = NpmConfig {
            include_prereleases: true,
        };
        let expected = config.clone();
        let masked = config.with_secrets_masked();
        assert_eq!(masked, expected);
    }
}
