use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

/// Discovery filter: which packages to surface during autodiscovery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnfDiscoveryFilter {
    /// All installed RPM packages reported by `rpm -qa`.
    #[default]
    All,
    /// Only packages the user explicitly installed (`dnf repoquery --userinstalled`).
    UserInstalled,
}

/// Configuration for the DNF plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the RPM
/// package name (e.g., `nginx`, `python3`).
///
/// The `discovery_filter` controls which packages are surfaced during
/// autodiscovery. The default (`All`) surfaces all installed packages.
/// A [`DiscoveryTarget`] is always emitted so the controller can
/// find-or-create the plugin config and role assignments.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnfConfig {
    /// Discovery filter controlling which packages to surface.
    #[serde(default)]
    pub discovery_filter: DnfDiscoveryFilter,
}

impl PluginConfig for DnfConfig {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        Ok(())
    }

    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }
}

impl DnfConfig {
    /// Returns the discovery filter to apply.
    pub(crate) fn effective_filter(&self) -> DnfDiscoveryFilter {
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
            DnfConfig::default().effective_filter(),
            DnfDiscoveryFilter::All
        );
    }

    #[test]
    fn effective_filter_explicit_all() {
        let config = DnfConfig {
            discovery_filter: DnfDiscoveryFilter::All,
        };
        assert_eq!(config.effective_filter(), DnfDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit_user_installed() {
        let config = DnfConfig {
            discovery_filter: DnfDiscoveryFilter::UserInstalled,
        };
        assert_eq!(config.effective_filter(), DnfDiscoveryFilter::UserInstalled);
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn default_config_is_all() {
        let config = DnfConfig::default();
        assert_eq!(config.discovery_filter, DnfDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_empty_object_gives_all() {
        let config: DnfConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, DnfDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_user_installed() {
        let config: DnfConfig =
            serde_json::from_str(r#"{"discovery_filter": "user_installed"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, DnfDiscoveryFilter::UserInstalled);
    }

    #[test]
    fn deserialize_all() {
        let config: DnfConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, DnfDiscoveryFilter::All);
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<DnfConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_roundtrip_all() {
        let config = DnfConfig {
            discovery_filter: DnfDiscoveryFilter::All,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: DnfConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_user_installed() {
        let config = DnfConfig {
            discovery_filter: DnfDiscoveryFilter::UserInstalled,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "user_installed");
        let deserialized: DnfConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = DnfConfig::default();
        assert!(PluginConfig::validate(&config).is_ok());
    }

    #[test]
    fn validate_accepts_all_config() {
        let config = DnfConfig {
            discovery_filter: DnfDiscoveryFilter::All,
        };
        assert!(PluginConfig::validate(&config).is_ok());
    }

    #[test]
    fn validate_accepts_user_installed_config() {
        let config = DnfConfig {
            discovery_filter: DnfDiscoveryFilter::UserInstalled,
        };
        assert!(PluginConfig::validate(&config).is_ok());
    }
}
