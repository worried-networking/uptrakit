use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::form_schema::{
    FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor,
};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AptDiscoveryFilter {
    /// All installed packages reported by dpkg.
    #[default]
    All,
    /// Only packages the user explicitly installed (`apt-mark showmanual`).
    Manual,
}

/// Configuration for the APT plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the Debian
/// package name (e.g., `nginx`, `python3`).
///
/// The `discovery_filter` controls which packages are surfaced during
/// autodiscovery. The default (`All`) surfaces all installed packages.
/// A [`DiscoveryTarget`] is always emitted so the controller can
/// find-or-create the plugin config and role assignments.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AptConfig {
    /// Discovery filter controlling which packages to surface.
    #[serde(default)]
    pub discovery_filter: AptDiscoveryFilter,
}

impl PluginConfig for AptConfig {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        Ok(())
    }

    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }

    fn form_schema() -> Vec<FormFieldDescriptor> {
        vec![]
    }
}

impl TypeSettings for AptConfig {
    fn type_settings_form_schema() -> Vec<FormFieldDescriptor> {
        vec![
            FormFieldDescriptor::new("discovery_filter", "Discovery Filter")
                .with_type(FormFieldType::Select)
                .with_options(vec![
                    FormSelectOptionDescriptor::new("all", "All installed packages"),
                    FormSelectOptionDescriptor::new("manual", "Manually installed only"),
                ])
                .with_help_text("Which packages to discover during autodiscovery"),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({
            "discovery_filter": "all"
        })
    }
}

impl AptConfig {
    /// Returns the discovery filter to apply.
    pub(crate) fn effective_filter(&self) -> AptDiscoveryFilter {
        self.discovery_filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── effective_filter ──────────────────────────────────────────────────────

    #[test]
    fn effective_filter_default_is_all() {
        assert_eq!(
            AptConfig::default().effective_filter(),
            AptDiscoveryFilter::All
        );
    }

    #[test]
    fn effective_filter_explicit_all() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::All,
        };
        assert_eq!(config.effective_filter(), AptDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit_manual() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::Manual,
        };
        assert_eq!(config.effective_filter(), AptDiscoveryFilter::Manual);
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn default_config_is_all() {
        let config = AptConfig::default();
        assert_eq!(config.discovery_filter, AptDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_empty_object_gives_all() {
        let config: AptConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, AptDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_manual() {
        let config: AptConfig =
            serde_json::from_str(r#"{"discovery_filter": "manual"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, AptDiscoveryFilter::Manual);
    }

    #[test]
    fn deserialize_all() {
        let config: AptConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, AptDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<AptConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_roundtrip_all() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::All,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: AptConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_manual() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::Manual,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "manual");
        let deserialized: AptConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = AptConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_all_config() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::All,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_manual_config() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::Manual,
        };
        assert!(config.validate().is_ok());
    }
}
