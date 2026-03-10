use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

/// Discovery filter for the APK plugin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
/// autodiscovery:
///
/// - `None` (default, serialises to `{}`) — the server sent an empty config
///   because no pre-existing APK plugin config exists yet. The plugin discovers
///   **all** installed packages and emits [`DiscoveryTarget`] values so the
///   controller can auto-create the plugin config and role assignments.
/// - `Some(All)` — explicitly configured to surface all installed packages.
///   Uses the config-ID path (no targets emitted).
/// - `Some(World)` — explicitly configured to surface only packages the user
///   explicitly installed (listed in `/etc/apk/world`). Uses the config-ID path.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkConfig {
    /// Discovery filter. `None` (the default when the config is `{}`) means
    /// "discover all packages" and causes targets to be emitted so the
    /// controller can auto-create the plugin config. An explicit `Some(_)`
    /// value means the plugin was given a real pre-existing config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_filter: Option<ApkDiscoveryFilter>,
}

impl SecretMasking for ApkConfig {}

impl uptrakit_plugin_infrastructure_core::ConfigFormSchema for ApkConfig {
    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType, SelectOption};
        vec![
            FieldDef::new("discovery_filter", "Discovery Filter")
                .with_type(FieldType::Select)
                .with_options(vec![
                    SelectOption::new("all", "All installed packages"),
                    SelectOption::new("world", "Explicitly installed packages (world file)"),
                ])
                .with_help_text("Which packages to discover during autodiscovery"),
        ]
    }
}

impl ApkConfig {
    /// Validate an APK package identifier string.
    pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> crate::error::Result<()> {
        Ok(())
    }

    /// Returns `true` when the config is at its default — i.e. it was produced
    /// by deserialising an empty JSON object `{}` with no explicit
    /// `discovery_filter` key.
    pub(crate) fn is_discover_all_mode(&self) -> bool {
        self.discovery_filter.is_none()
    }

    /// Returns the effective discovery filter to apply.
    pub(crate) fn effective_filter(&self) -> ApkDiscoveryFilter {
        self.discovery_filter
            .clone()
            .unwrap_or(ApkDiscoveryFilter::All)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_discover_all_mode_true_for_default_config() {
        assert!(ApkConfig::default().is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_explicit_all_filter() {
        let config = ApkConfig {
            discovery_filter: Some(ApkDiscoveryFilter::All),
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_for_world_filter() {
        let config = ApkConfig {
            discovery_filter: Some(ApkDiscoveryFilter::World),
        };
        assert!(!config.is_discover_all_mode());
    }

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
            discovery_filter: Some(ApkDiscoveryFilter::All),
        };
        assert_eq!(config.effective_filter(), ApkDiscoveryFilter::All);
    }

    #[test]
    fn effective_filter_explicit_world() {
        let config = ApkConfig {
            discovery_filter: Some(ApkDiscoveryFilter::World),
        };
        assert_eq!(config.effective_filter(), ApkDiscoveryFilter::World);
    }

    #[test]
    fn default_config_has_no_filter() {
        let config = ApkConfig::default();
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_empty_object_gives_none() {
        let config: ApkConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.discovery_filter, None);
    }

    #[test]
    fn deserialize_world() {
        let config: ApkConfig =
            serde_json::from_str(r#"{"discovery_filter": "world"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, Some(ApkDiscoveryFilter::World));
    }

    #[test]
    fn deserialize_all() {
        let config: ApkConfig =
            serde_json::from_str(r#"{"discovery_filter": "all"}"#).expect("deserialize");
        assert_eq!(config.discovery_filter, Some(ApkDiscoveryFilter::All));
    }

    #[test]
    fn deserialize_invalid_filter_fails() {
        let result = serde_json::from_str::<ApkConfig>(r#"{"discovery_filter": "invalid"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn serialization_none_gives_empty_object() {
        let config = ApkConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json, serde_json::json!({}));
        let deserialized: ApkConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_world() {
        let config = ApkConfig {
            discovery_filter: Some(ApkDiscoveryFilter::World),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "world");
        let deserialized: ApkConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_all() {
        let config = ApkConfig {
            discovery_filter: Some(ApkDiscoveryFilter::All),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["discovery_filter"], "all");
        let deserialized: ApkConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn validate_accepts_default_config() {
        let config = ApkConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_explicit_all_config() {
        let config = ApkConfig {
            discovery_filter: Some(ApkDiscoveryFilter::All),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_world_config() {
        let config = ApkConfig {
            discovery_filter: Some(ApkDiscoveryFilter::World),
        };
        assert!(config.validate().is_ok());
    }
}
