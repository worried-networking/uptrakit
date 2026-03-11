use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
/// autodiscovery:
///
/// - `None` (default, serialises to `{}`) — the server sent an empty config
///   because no pre-existing pkg plugin config exists yet. The plugin discovers
///   **all** installed packages and emits [`DiscoveryTarget`] values so the
///   controller can auto-create the plugin config and role assignments.
/// - `Some(All)` — explicitly configured to surface all installed packages.
///   Uses the config-ID path (no targets emitted).
/// - `Some(Manual)` — explicitly configured to surface only packages where the
///   automatic install flag is `0` (i.e. explicitly installed). Uses the
///   config-ID path (no targets emitted).
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkgConfig {
    /// Discovery filter. `None` (the default when the config is `{}`) means
    /// "discover all packages" and causes targets to be emitted so the
    /// controller can auto-create the plugin config. An explicit `Some(_)`
    /// value means the plugin was given a real pre-existing config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_filter: Option<PkgDiscoveryFilter>,
}

impl SecretMasking for PkgConfig {}

impl uptrakit_plugin_infrastructure_core::ConfigFormSchema for PkgConfig {
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

    fn type_settings_form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef>
    {
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

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({
            "discovery_filter": "all"
        })
    }
}

impl PkgConfig {
    /// Validate a BSD pkg package identifier string.
    ///
    /// Delegates to the crate-level [`validate_identifier`](crate::validate_identifier)
    /// function.
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
    /// pre-existing pkg plugin config exists for the tenant. `discover_software()`
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
    /// `None` (default config) behaves as `All` — all installed packages are
    /// reported.
    pub(crate) fn effective_filter(&self) -> PkgDiscoveryFilter {
        self.discovery_filter
            .clone()
            .unwrap_or(PkgDiscoveryFilter::All)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_discover_all_mode ──────────────────────────────────────────────────

    #[test]
    fn is_discover_all_mode_true_for_default_config() {
        assert!(PkgConfig::default().is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_explicit_all_filter() {
        let config = PkgConfig {
            discovery_filter: Some(PkgDiscoveryFilter::All),
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_manual_filter() {
        let config = PkgConfig {
            discovery_filter: Some(PkgDiscoveryFilter::Manual),
        };
        assert!(!config.is_discover_all_mode());
    }

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
            discovery_filter: Some(PkgDiscoveryFilter::All),
        };
        assert_eq!(config.effective_filter(), PkgDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit_manual() {
        let config = PkgConfig {
            discovery_filter: Some(PkgDiscoveryFilter::Manual),
        };
        assert_eq!(config.effective_filter(), PkgDiscoveryFilter::Manual);
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn default_config_has_no_filter() {
        let config = PkgConfig::default();
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_empty_object_gives_none() {
        let config: PkgConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_manual() {
        let config: PkgConfig =
            serde_json::from_str(r#"{"discovery_filter": "manual"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, Some(PkgDiscoveryFilter::Manual));
    }

    #[test]
    fn deserialize_all() {
        let config: PkgConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, Some(PkgDiscoveryFilter::All));
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<PkgConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_none_gives_empty_object() {
        let config = PkgConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json, serde_json::json!({}));
        let deserialized: PkgConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_manual() {
        let config = PkgConfig {
            discovery_filter: Some(PkgDiscoveryFilter::Manual),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "manual");
        let deserialized: PkgConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_all() {
        let config = PkgConfig {
            discovery_filter: Some(PkgDiscoveryFilter::All),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: PkgConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        assert!(PkgConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_accepts_explicit_all_config() {
        let config = PkgConfig {
            discovery_filter: Some(PkgDiscoveryFilter::All),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_manual_config() {
        let config = PkgConfig {
            discovery_filter: Some(PkgDiscoveryFilter::Manual),
        };
        assert!(config.validate().is_ok());
    }
}
