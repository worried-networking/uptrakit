use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AptDiscoveryFilter {
    /// Only packages the user explicitly installed (`apt-mark showmanual`).
    #[default]
    Manual,
    /// All installed packages reported by dpkg.
    All,
}

/// Configuration for the APT plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the Debian
/// package name (e.g., `nginx`, `python3`).
///
/// The `discovery_filter` controls which packages are surfaced during
/// autodiscovery. `Manual` (default) surfaces only packages the user
/// explicitly installed via `apt-mark showmanual`. `All` surfaces every
/// installed package reported by `dpkg`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AptConfig {
    /// Discovery filter (default: `manual`).
    #[serde(default)]
    pub discovery_filter: AptDiscoveryFilter,
}

impl SecretMasking for AptConfig {}

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
    /// by deserialising an empty JSON object `{}`.
    ///
    /// The server sends an empty config with `plugin_config_id: None` when no
    /// pre-existing APT plugin config exists for the tenant. `discover_software()`
    /// uses this to decide whether to emit
    /// [`uptrakit_plugin_infrastructure_core::DiscoveryTarget`] values so the
    /// controller can auto-create the default plugin config and role assignments.
    /// When a real config is present (e.g. `discovery_filter: "all"`) the server
    /// sends `plugin_config_id: Some(_)` and items are processed via the
    /// config-ID path (no targets needed).
    pub(crate) fn is_discover_all_mode(&self) -> bool {
        self.discovery_filter == AptDiscoveryFilter::Manual
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_discover_all_mode ──────────────────────────────────────────────────

    #[test]
    fn is_discover_all_mode_true_for_default_config() {
        assert!(AptConfig::default().is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_true_for_manual_filter() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::Manual,
        };
        assert!(config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_all_filter() {
        let config = AptConfig {
            discovery_filter: AptDiscoveryFilter::All,
        };
        assert!(!config.is_discover_all_mode());
    }

    // ── existing tests ────────────────────────────────────────────────────────

    #[test]
    fn default_config_is_manual() {
        let config = AptConfig::default();
        assert_eq!(config.discovery_filter, AptDiscoveryFilter::Manual);
    }

    #[test]
    fn deserialize_empty_object() {
        let config: AptConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, AptDiscoveryFilter::Manual);
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
}
