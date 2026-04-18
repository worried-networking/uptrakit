use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PkgDiscoveryFilter {
    /// All installed packages reported by `pkg query -a`.
    #[default]
    All,
    /// Only packages explicitly installed by the user (auto-install flag == 0).
    Manual,
}

/// Configuration for the BSD pkg plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the FreeBSD
/// package name (e.g., `nginx`, `python39`).
///
/// The `discovery_filter` controls which packages are surfaced during
/// autodiscovery. The default (`All`) surfaces all installed packages.
/// A [`DiscoveryTarget`] is always emitted so the controller can
/// find-or-create the plugin config and role assignments.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkgConfig {
    /// Discovery filter controlling which packages to surface.
    #[serde(default)]
    pub discovery_filter: PkgDiscoveryFilter,
}

impl PluginConfig for PkgConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value).map_err(PluginConfigValidationError::InvalidIdentifier)
    }
}

impl TypeSettings for PkgConfig {
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

impl PkgConfig {
    /// Returns the discovery filter to apply.
    pub(crate) fn effective_filter(&self) -> PkgDiscoveryFilter {
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
            PkgConfig::default().effective_filter(),
            PkgDiscoveryFilter::All
        );
    }

    #[test]
    fn effective_filter_explicit_all() {
        let config = PkgConfig {
            discovery_filter: PkgDiscoveryFilter::All,
        };
        assert_eq!(config.effective_filter(), PkgDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit_manual() {
        let config = PkgConfig {
            discovery_filter: PkgDiscoveryFilter::Manual,
        };
        assert_eq!(config.effective_filter(), PkgDiscoveryFilter::Manual);
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn default_config_is_all() {
        let config = PkgConfig::default();
        assert_eq!(config.discovery_filter, PkgDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_empty_object_gives_all() {
        let config: PkgConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, PkgDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_manual() {
        let config: PkgConfig =
            serde_json::from_str(r#"{"discovery_filter": "manual"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, PkgDiscoveryFilter::Manual);
    }

    #[test]
    fn deserialize_all() {
        let config: PkgConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, PkgDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<PkgConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_roundtrip_all() {
        let config = PkgConfig {
            discovery_filter: PkgDiscoveryFilter::All,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: PkgConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_manual() {
        let config = PkgConfig {
            discovery_filter: PkgDiscoveryFilter::Manual,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "manual");
        let deserialized: PkgConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        assert!(PkgConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_accepts_all_config() {
        let config = PkgConfig {
            discovery_filter: PkgDiscoveryFilter::All,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_manual_config() {
        let config = PkgConfig {
            discovery_filter: PkgDiscoveryFilter::Manual,
        };
        assert!(config.validate().is_ok());
    }
}
