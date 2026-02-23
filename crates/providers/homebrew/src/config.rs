use serde::{Deserialize, Serialize};
use uptrakit_provider_core::SecretMasking;

/// Homebrew package type: formula (CLI tools, libraries) or cask (GUI applications).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomebrewPackageType {
    /// Standard Homebrew formula (default).
    #[default]
    Formula,
    /// Homebrew cask (macOS GUI application).
    Cask,
}

/// Configuration for the Homebrew provider.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the formula/cask
/// name (e.g., `wget`, `firefox`).
///
/// When `package_type` is `None`, the provider discovers all installed packages
/// (both formulae and casks) and annotates each with
/// `extra = {"package_type": "formula"}` or `extra = {"package_type": "cask"}`.
/// This is used during autodiscovery when no pre-existing provider config exists.
///
/// When `package_type` is set, only that type is discovered (the existing
/// version-tracking behaviour).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomebrewConfig {
    /// Whether to track formulae, casks, or both (`None` = discover all).
    #[serde(default)]
    pub package_type: Option<HomebrewPackageType>,
}

impl SecretMasking for HomebrewConfig {}

impl HomebrewConfig {
    /// Validate the configuration.
    ///
    /// Currently accepts all valid deserialized configs since there are no
    /// required fields beyond `package_type` which has a default.
    pub fn validate(&self) -> crate::error::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_none() {
        let config = HomebrewConfig::default();
        assert_eq!(config.package_type, None);
    }

    #[test]
    fn deserialize_empty_object() {
        let config: HomebrewConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.package_type, None);
    }

    #[test]
    fn deserialize_formula() {
        let config: HomebrewConfig =
            serde_json::from_str(r#"{"package_type": "formula"}"#).expect("deserialize");
        assert_eq!(config.package_type, Some(HomebrewPackageType::Formula));
    }

    #[test]
    fn deserialize_cask() {
        let config: HomebrewConfig =
            serde_json::from_str(r#"{"package_type": "cask"}"#).expect("deserialize");
        assert_eq!(config.package_type, Some(HomebrewPackageType::Cask));
    }

    #[test]
    fn deserialize_null_package_type() {
        let config: HomebrewConfig =
            serde_json::from_str(r#"{"package_type": null}"#).expect("deserialize");
        assert_eq!(config.package_type, None);
    }

    #[test]
    fn serialization_roundtrip_none() {
        let config = HomebrewConfig { package_type: None };
        let json = serde_json::to_value(&config).expect("serialize");
        // None serializes as null with #[serde(default)]
        let deserialized: HomebrewConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_formula() {
        let config = HomebrewConfig {
            package_type: Some(HomebrewPackageType::Formula),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["package_type"], "formula");
        let deserialized: HomebrewConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_cask() {
        let config = HomebrewConfig {
            package_type: Some(HomebrewPackageType::Cask),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["package_type"], "cask");
        let deserialized: HomebrewConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = HomebrewConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_cask_config() {
        let config = HomebrewConfig {
            package_type: Some(HomebrewPackageType::Cask),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn deserialize_invalid_package_type_fails() {
        let result = serde_json::from_str::<HomebrewConfig>(r#"{"package_type": "invalid"}"#);
        assert!(result.is_err());
    }
}
