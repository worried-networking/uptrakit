use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::PluginConfig;

/// Configuration for the uptrakit self-update discovery plugin.
///
/// When `enabled` is `false` (the default), `detect_host_compatibility` returns
/// `Incompatible` immediately — no I/O is performed and no sudoers entries are
/// installed. Set `enabled = true` in the controller-standalone config to opt in.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UptrakitSelfUpdateConfig {
    /// Enable self-update discovery. Defaults to `false`.
    ///
    /// Must be explicitly set to `true` — controller-standalone ships with
    /// `enabled: false` so the plugin is inert unless the operator opts in.
    #[serde(default)]
    pub enabled: bool,
}

impl PluginConfig for UptrakitSelfUpdateConfig {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        assert!(!UptrakitSelfUpdateConfig::default().enabled);
    }

    #[test]
    fn validate_always_succeeds() {
        assert!(UptrakitSelfUpdateConfig::default().validate().is_ok());
    }

    #[test]
    fn deserialize_empty_object_defaults_to_disabled() {
        let config: UptrakitSelfUpdateConfig =
            serde_json::from_str("{}").expect("deserialize empty config");
        assert!(!config.enabled);
    }

    #[test]
    fn deserialize_enabled_true() {
        let config: UptrakitSelfUpdateConfig =
            serde_json::from_str(r#"{"enabled":true}"#).expect("deserialize enabled config");
        assert!(config.enabled);
    }

    #[test]
    fn serialize_default_produces_disabled() {
        let config = UptrakitSelfUpdateConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert_eq!(json, r#"{"enabled":false}"#);
    }
}
