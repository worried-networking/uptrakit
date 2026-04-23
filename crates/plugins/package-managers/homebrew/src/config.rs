use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

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

impl PluginConfig for HomebrewConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }
}

impl TypeSettings for HomebrewConfig {
    fn type_settings_form_schema()
    -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor,
        };
        vec![
            FormFieldDescriptor::new("package_type", "Package Type")
                .with_type(FormFieldType::Select)
                .with_options(vec![
                    FormSelectOptionDescriptor::new("both", "Both (formulae and casks)"),
                    FormSelectOptionDescriptor::new("formula", "Formula (CLI tools, libraries)"),
                    FormSelectOptionDescriptor::new("cask", "Cask (GUI applications)"),
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
