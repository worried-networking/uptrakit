use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

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

impl SecretMasking for PacmanConfig {}

impl uptrakit_plugin_infrastructure_core::ConfigFormSchema for PacmanConfig {
    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        vec![]
    }

    fn type_settings_form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef>
    {
        use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType, SelectOption};
        vec![
            FieldDef::new("discovery_filter", "Discovery Filter")
                .with_type(FieldType::Select)
                .with_options(vec![
                    SelectOption::new("all", "All installed packages"),
                    SelectOption::new("explicit", "Explicitly installed only"),
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
    /// Validate a Pacman package identifier string.
    ///
    /// Delegates to the crate-level [`validate_identifier`](crate::validate_identifier)
    /// function. A valid identifier is a non-empty Arch Linux package name.
    ///
    /// Called by the plugin registry's `validate_package_identifier` dispatch.
    pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

    /// Validate the configuration.
    ///
    /// Currently accepts all valid deserialized configs.
    pub fn validate(&self) -> crate::error::Result<()> {
        Ok(())
    }

    /// Returns the discovery filter to apply.
    pub(crate) fn effective_filter(&self) -> PacmanDiscoveryFilter {
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
