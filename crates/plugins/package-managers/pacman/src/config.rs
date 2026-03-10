use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
/// autodiscovery:
///
/// - `None` (default, serialises to `{}`) — the server sent an empty config
///   because no pre-existing Pacman plugin config exists yet. The plugin
///   discovers **all** installed packages and emits [`DiscoveryTarget`] values
///   so the controller can auto-create the plugin config and role assignments.
/// - `Some(All)` — explicitly configured to surface all installed packages.
///   Uses the config-ID path (no targets emitted).
/// - `Some(Explicit)` — explicitly configured to surface only packages the
///   user explicitly installed (`pacman -Qe`). Uses the config-ID path (no
///   targets emitted).
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacmanConfig {
    /// Discovery filter. `None` (the default when the config is `{}`) means
    /// "discover all packages" and causes targets to be emitted so the
    /// controller can auto-create the plugin config. An explicit `Some(_)`
    /// value means the plugin was given a real pre-existing config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_filter: Option<PacmanDiscoveryFilter>,
}

impl SecretMasking for PacmanConfig {}

impl uptrakit_plugin_infrastructure_core::ConfigFormSchema for PacmanConfig {
    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
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

    /// Returns `true` when the config is at its default — i.e. it was produced
    /// by deserialising an empty JSON object `{}` with no explicit
    /// `discovery_filter` key.
    ///
    /// The server sends an empty config with `plugin_config_id: None` when no
    /// pre-existing Pacman plugin config exists for the tenant.
    /// `discover_software()` uses this to decide whether to emit
    /// [`uptrakit_plugin_infrastructure_core::DiscoveryTarget`] values so the
    /// controller can auto-create the default plugin config and role
    /// assignments. When a real config is present the server sends
    /// `plugin_config_id: Some(_)` and items are processed via the config-ID
    /// path (no targets needed).
    pub(crate) fn is_discover_all_mode(&self) -> bool {
        self.discovery_filter.is_none()
    }

    /// Returns the effective discovery filter to apply.
    ///
    /// `None` (default config) behaves as `All` — all installed packages are
    /// reported.
    pub(crate) fn effective_filter(&self) -> PacmanDiscoveryFilter {
        self.discovery_filter
            .clone()
            .unwrap_or(PacmanDiscoveryFilter::All)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_discover_all_mode ──────────────────────────────────────────────────

    #[test]
    fn is_discover_all_mode_true_for_default_config() {
        assert!(PacmanConfig::default().is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_explicit_all_filter() {
        let config = PacmanConfig {
            discovery_filter: Some(PacmanDiscoveryFilter::All),
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_explicit_filter() {
        let config = PacmanConfig {
            discovery_filter: Some(PacmanDiscoveryFilter::Explicit),
        };
        assert!(!config.is_discover_all_mode());
    }

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
            discovery_filter: Some(PacmanDiscoveryFilter::All),
        };
        assert_eq!(config.effective_filter(), PacmanDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit() {
        let config = PacmanConfig {
            discovery_filter: Some(PacmanDiscoveryFilter::Explicit),
        };
        assert_eq!(config.effective_filter(), PacmanDiscoveryFilter::Explicit);
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn default_config_has_no_filter() {
        let config = PacmanConfig::default();
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_empty_object_gives_none() {
        let config: PacmanConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_explicit() {
        let config: PacmanConfig =
            serde_json::from_str(r#"{"discovery_filter": "explicit"}"#).expect("deserialize");
        assert_eq!(
            config.discovery_filter,
            Some(PacmanDiscoveryFilter::Explicit)
        );
    }

    #[test]
    fn deserialize_all() {
        let config: PacmanConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, Some(PacmanDiscoveryFilter::All));
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<PacmanConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_none_gives_empty_object() {
        let config = PacmanConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json, serde_json::json!({}));
        let deserialized: PacmanConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_explicit() {
        let config = PacmanConfig {
            discovery_filter: Some(PacmanDiscoveryFilter::Explicit),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "explicit");
        let deserialized: PacmanConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_all() {
        let config = PacmanConfig {
            discovery_filter: Some(PacmanDiscoveryFilter::All),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: PacmanConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = PacmanConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_explicit_all_config() {
        let config = PacmanConfig {
            discovery_filter: Some(PacmanDiscoveryFilter::All),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_explicit_config() {
        let config = PacmanConfig {
            discovery_filter: Some(PacmanDiscoveryFilter::Explicit),
        };
        assert!(config.validate().is_ok());
    }
}
