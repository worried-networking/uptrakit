use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
/// autodiscovery:
///
/// - `None` (default, serialises to `{}`) — the server sent an empty config
///   because no pre-existing APT plugin config exists yet. The plugin discovers
///   **all** installed packages and emits [`DiscoveryTarget`] values so the
///   controller can auto-create the plugin config and role assignments.
/// - `Some(All)` — explicitly configured to surface all installed packages.
///   Uses the config-ID path (no targets emitted).
/// - `Some(Manual)` — explicitly configured to surface only packages the user
///   explicitly installed via `apt-mark showmanual`. Uses the config-ID path
///   (no targets emitted).
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AptConfig {
    /// Discovery filter. `None` (the default when the config is `{}`) means
    /// "discover all packages" and causes targets to be emitted so the
    /// controller can auto-create the plugin config. An explicit `Some(_)`
    /// value means the plugin was given a real pre-existing config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_filter: Option<AptDiscoveryFilter>,
}

impl SecretMasking for AptConfig {}

impl uptrakit_plugin_infrastructure_core::ConfigFormSchema for AptConfig {
    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType, SelectOption};
        vec![
            FieldDef::new("discovery_filter", "Discovery Filter")
                .with_type(FieldType::Select)
                .with_options(vec![
                    SelectOption::new("all", "All installed packages"),
                    SelectOption::new("manual", "Manually installed only"),
                ])
                .with_help_text("Which packages to discover during autodiscovery"),
        ]
    }
}

impl AptConfig {
    /// Validate an APT package identifier string.
    ///
    /// Delegates to the crate-level [`validate_identifier`](crate::validate_identifier)
    /// function. A valid identifier is a non-empty, lowercase Debian package name.
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

    /// Returns `true` when the config is at its default — i.e. it was produced
    /// by deserialising an empty JSON object `{}` with no explicit
    /// `discovery_filter` key.
    ///
    /// The server sends an empty config with `plugin_config_id: None` when no
    /// pre-existing APT plugin config exists for the tenant. `discover_software()`
    /// uses this to decide whether to emit
    /// [`uptrakit_plugin_infrastructure_core::DiscoveryTarget`] values so the
    /// controller can auto-create the default plugin config and role assignments.
    /// When a real config is present the server sends `plugin_config_id: Some(_)`
    /// and items are processed via the config-ID path (no targets needed).
    pub(crate) fn is_discover_all_mode(&self) -> bool {
        self.discovery_filter.is_none()
    }

    /// Returns the effective discovery filter to apply.
    ///
    /// `None` (default config) behaves as `All` — all installed dpkg packages
    /// are reported.
    pub(crate) fn effective_filter(&self) -> AptDiscoveryFilter {
        self.discovery_filter
            .clone()
            .unwrap_or(AptDiscoveryFilter::All)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_discover_all_mode ──────────────────────────────────────────────────

    #[test]
    fn is_discover_all_mode_true_for_default_config() {
        // Default config (discovery_filter: None) → discover-all mode.
        assert!(AptConfig::default().is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_explicit_all_filter() {
        // Explicit Some(All) means a pre-existing config → config-ID path.
        let config = AptConfig {
            discovery_filter: Some(AptDiscoveryFilter::All),
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_manual_filter() {
        // Explicit Some(Manual) means a pre-existing config → config-ID path.
        let config = AptConfig {
            discovery_filter: Some(AptDiscoveryFilter::Manual),
        };
        assert!(!config.is_discover_all_mode());
    }

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
            discovery_filter: Some(AptDiscoveryFilter::All),
        };
        assert_eq!(config.effective_filter(), AptDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit_manual() {
        let config = AptConfig {
            discovery_filter: Some(AptDiscoveryFilter::Manual),
        };
        assert_eq!(config.effective_filter(), AptDiscoveryFilter::Manual);
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn default_config_has_no_filter() {
        let config = AptConfig::default();
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_empty_object_gives_none() {
        let config: AptConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_manual() {
        let config: AptConfig =
            serde_json::from_str(r#"{"discovery_filter": "manual"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, Some(AptDiscoveryFilter::Manual));
    }

    #[test]
    fn deserialize_all() {
        let config: AptConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, Some(AptDiscoveryFilter::All));
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<AptConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_none_gives_empty_object() {
        let config = AptConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        // None is skipped → empty object → is_discover_all_mode() stays true on
        // the next round-trip.
        assert_eq!(json, serde_json::json!({}));
        let deserialized: AptConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_manual() {
        let config = AptConfig {
            discovery_filter: Some(AptDiscoveryFilter::Manual),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "manual");
        let deserialized: AptConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_all() {
        let config = AptConfig {
            discovery_filter: Some(AptDiscoveryFilter::All),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: AptConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = AptConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_explicit_all_config() {
        let config = AptConfig {
            discovery_filter: Some(AptDiscoveryFilter::All),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_manual_config() {
        let config = AptConfig {
            discovery_filter: Some(AptDiscoveryFilter::Manual),
        };
        assert!(config.validate().is_ok());
    }
}
