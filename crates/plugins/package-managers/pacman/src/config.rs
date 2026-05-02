use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacmanDiscoveryFilter {
    /// All installed packages reported by `pacman -Q`.
    #[default]
    All,
    /// Only packages explicitly installed by the user (`pacman -Qe`).
    Explicit,
}

/// Configuration for the Pacman plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the Arch Linux
/// package name (e.g., `nginx`, `python`, `git`).
///
/// The `discovery_filter` controls which packages are surfaced during
/// autodiscovery. The default (`All`) surfaces all installed packages.
/// A [`DiscoveryTarget`] is always emitted so the controller can
/// find-or-create the plugin config and role assignments.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacmanConfig {
    /// Discovery filter controlling which packages to surface.
    #[serde(default)]
    pub discovery_filter: PacmanDiscoveryFilter,
}

impl PluginConfig for PacmanConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }
}

impl TypeSettings for PacmanConfig {
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
                    FormSelectOptionDescriptor::new("explicit", "Explicitly installed only"),
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

impl PacmanConfig {
    /// Returns the discovery filter to apply.
    pub(crate) fn effective_filter(&self) -> PacmanDiscoveryFilter {
        self.discovery_filter
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;

    // ── effective_filter ──────────────────────────────────────────────────────

    #[test]
    fn effective_filter_default_is_all() {
        assert_eq!(
            PacmanConfig::default().effective_filter(),
            PacmanDiscoveryFilter::All
        );
    }

    #[test]
    fn effective_filter_explicit_all() {
        let config = PacmanConfig {
            discovery_filter: PacmanDiscoveryFilter::All,
        };
        assert_eq!(config.effective_filter(), PacmanDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit() {
        let config = PacmanConfig {
            discovery_filter: PacmanDiscoveryFilter::Explicit,
        };
        assert_eq!(config.effective_filter(), PacmanDiscoveryFilter::Explicit);
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn default_config_is_all() {
        let config = PacmanConfig::default();
        assert_eq!(config.discovery_filter, PacmanDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_empty_object_gives_all() {
        let config: PacmanConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, PacmanDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_explicit() {
        let config: PacmanConfig =
            serde_json::from_str(r#"{"discovery_filter": "explicit"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, PacmanDiscoveryFilter::Explicit);
    }

    #[test]
    fn deserialize_all() {
        let config: PacmanConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, PacmanDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<PacmanConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_roundtrip_all() {
        let config = PacmanConfig {
            discovery_filter: PacmanDiscoveryFilter::All,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: PacmanConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_explicit() {
        let config = PacmanConfig {
            discovery_filter: PacmanDiscoveryFilter::Explicit,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "explicit");
        let deserialized: PacmanConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = PacmanConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_all_config() {
        let config = PacmanConfig {
            discovery_filter: PacmanDiscoveryFilter::All,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_explicit_config() {
        let config = PacmanConfig {
            discovery_filter: PacmanDiscoveryFilter::Explicit,
        };
        assert!(config.validate().is_ok());
    }
}
