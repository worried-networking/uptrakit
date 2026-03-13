use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Homebrew package type: formula (CLI tools, libraries), cask (GUI applications), or both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomebrewPackageType {
    /// Track both formulae and casks (default).
    #[default]
    Both,
    /// Standard Homebrew formula (CLI tools, libraries).
    Formula,
    /// Homebrew cask (macOS GUI application).
    Cask,
}

/// Configuration for the Homebrew plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the formula/cask
/// name (e.g., `wget`, `firefox`).
///
/// When `package_type` is `Both` (the default), the plugin discovers all
/// installed packages (formulae and casks) and annotates each with
/// `extra = {"package_type": "formula"}` or `extra = {"package_type": "cask"}`.
///
/// When `package_type` is `Formula` or `Cask`, only that type is discovered.
///
/// A [`DiscoveryTarget`] is always emitted so the controller can
/// find-or-create the plugin config and role assignments.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomebrewConfig {
    /// Whether to track formulae, casks, or both.
    #[serde(default)]
    pub package_type: HomebrewPackageType,
}

impl SecretMasking for HomebrewConfig {}

impl uptrakit_plugin_infrastructure_core::ConfigFormSchema for HomebrewConfig {
    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        vec![]
    }

    fn type_settings_form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef>
    {
        use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType, SelectOption};
        vec![
            FieldDef::new("package_type", "Package Type")
                .with_type(FieldType::Select)
                .with_options(vec![
                    SelectOption::new("both", "Both (formulae and casks)"),
                    SelectOption::new("formula", "Formula (CLI tools, libraries)"),
                    SelectOption::new("cask", "Cask (GUI applications)"),
                ])
                .with_help_text("Track formulae, casks, or both"),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({
            "package_type": "formula"
        })
    }
}

impl HomebrewConfig {
    /// Validate a Homebrew package identifier string.
    ///
    /// Delegates to the crate-level [`validate_identifier`](crate::validate_identifier)
    /// function. A valid identifier is a non-empty formula or cask name.
    ///
    /// Called by the plugin registry's `validate_package_identifier` dispatch.
    pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

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
    fn default_config_is_both() {
        let config = HomebrewConfig::default();
        assert_eq!(config.package_type, HomebrewPackageType::Both);
    }

    #[test]
    fn deserialize_empty_object_gives_both() {
        let config: HomebrewConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.package_type, HomebrewPackageType::Both);
    }

    #[test]
    fn deserialize_both() {
        let config: HomebrewConfig =
            serde_json::from_str(r#"{"package_type": "both"}"#).expect("deserialize");
        assert_eq!(config.package_type, HomebrewPackageType::Both);
    }

    #[test]
    fn deserialize_formula() {
        let config: HomebrewConfig =
            serde_json::from_str(r#"{"package_type": "formula"}"#).expect("deserialize");
        assert_eq!(config.package_type, HomebrewPackageType::Formula);
    }

    #[test]
    fn deserialize_cask() {
        let config: HomebrewConfig =
            serde_json::from_str(r#"{"package_type": "cask"}"#).expect("deserialize");
        assert_eq!(config.package_type, HomebrewPackageType::Cask);
    }

    #[test]
    fn serialization_roundtrip_both() {
        let config = HomebrewConfig {
            package_type: HomebrewPackageType::Both,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["package_type"], "both");
        let deserialized: HomebrewConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_formula() {
        let config = HomebrewConfig {
            package_type: HomebrewPackageType::Formula,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["package_type"], "formula");
        let deserialized: HomebrewConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_cask() {
        let config = HomebrewConfig {
            package_type: HomebrewPackageType::Cask,
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
            package_type: HomebrewPackageType::Cask,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn deserialize_invalid_package_type_fails() {
        let result = serde_json::from_str::<HomebrewConfig>(r#"{"package_type": "invalid"}"#);
        assert!(result.is_err());
    }
}
