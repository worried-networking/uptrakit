use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Discovery filter for the APK plugin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApkDiscoveryFilter {
    /// All installed packages reported by `apk list --installed`.
    #[default]
    All,
    /// Only packages explicitly listed in `/etc/apk/world`.
    World,
}

/// Configuration for the APK (Alpine Linux) plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the Alpine
/// package name (e.g., `nginx`, `openssl`).
///
/// The `discovery_filter` controls which packages are surfaced during
/// autodiscovery. The default (`All`) surfaces all installed packages.
/// A [`DiscoveryTarget`] is always emitted so the controller can
/// find-or-create the plugin config and role assignments.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkConfig {
    /// Discovery filter controlling which packages to surface.
    #[serde(default)]
    pub discovery_filter: ApkDiscoveryFilter,
}

impl PluginConfig for ApkConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }
}

impl TypeSettings for ApkConfig {
    fn type_settings_form_schema()
    -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor,
        };
        vec![
            FormFieldDescriptor::new("discovery_filter", "Discovery Filter")
                .with_type(FormFieldType::Select)
                .with_options(vec![
                    FormSelectOptionDescriptor::new("all", "All installed packages"),
                    FormSelectOptionDescriptor::new(
                        "world",
                        "Explicitly installed packages (world file)",
                    ),
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

impl ApkConfig {
    /// Returns the discovery filter to apply.
    pub(crate) fn effective_filter(&self) -> ApkDiscoveryFilter {
        self.discovery_filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_filter_default_is_all() {
        assert_eq!(
            ApkConfig::default().effective_filter(),
            ApkDiscoveryFilter::All
        );
    }

    #[test]
    fn effective_filter_explicit_all() {
        let config = ApkConfig {
            discovery_filter: ApkDiscoveryFilter::All,
        };
        assert_eq!(config.effective_filter(), ApkDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit_world() {
        let config = ApkConfig {
            discovery_filter: ApkDiscoveryFilter::World,
        };
        assert_eq!(config.effective_filter(), ApkDiscoveryFilter::World);
    }

    #[test]
    fn default_config_is_all() {
        let config = ApkConfig::default();
        assert_eq!(config.discovery_filter, ApkDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_empty_object_gives_all() {
        let config: ApkConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, ApkDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_world() {
        let config: ApkConfig =
            serde_json::from_str(r#"{"discovery_filter": "world"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, ApkDiscoveryFilter::World);
    }

    #[test]
    fn deserialize_all() {
        let config: ApkConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, ApkDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<ApkConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn serialization_roundtrip_all() {
        let config = ApkConfig {
            discovery_filter: ApkDiscoveryFilter::All,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: ApkConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_world() {
        let config = ApkConfig {
            discovery_filter: ApkDiscoveryFilter::World,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "world");
        let deserialized: ApkConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = ApkConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_all_config() {
        let config = ApkConfig {
            discovery_filter: ApkDiscoveryFilter::All,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_world_config() {
        let config = ApkConfig {
            discovery_filter: ApkDiscoveryFilter::World,
        };
        assert!(config.validate().is_ok());
    }
}
