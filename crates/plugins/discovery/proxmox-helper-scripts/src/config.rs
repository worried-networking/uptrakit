use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::PluginConfig;

/// Configuration for the Proxmox Helper Scripts plugin.
///
/// PHS is discovery-only: it reads `/usr/bin/update`, fetches each referenced
/// CT script from `raw.githubusercontent.com` (or `github.com/…/raw/…`, which
/// is normalised to the same host), and analyses the script to
/// determine whether the app is GitHub-managed or APT-managed.  The resulting
/// `DiscoveredSoftware` items carry `github_owner`/`github_repo` or
/// `apt_package` in their `extra` field so the controller can synthesize the
/// appropriate downstream plugin config automatically.
///
/// No configuration fields are needed; the config is always serialized as `{}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProxmoxHelperScriptsConfig {}

impl PluginConfig for ProxmoxHelperScriptsConfig {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_always_succeeds() {
        assert!(ProxmoxHelperScriptsConfig::default().validate().is_ok());
    }

    #[test]
    fn deserialize_empty_object() {
        let config: ProxmoxHelperScriptsConfig =
            serde_json::from_str("{}").expect("deserialize empty config");
        let _ = config;
    }

    #[test]
    fn serialize_produces_empty_object() {
        let config = ProxmoxHelperScriptsConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert_eq!(json, "{}");
    }

    #[test]
    fn secret_masking_is_noop() {
        let config = ProxmoxHelperScriptsConfig::default();
        let masked = config.with_secrets_masked();
        let json = serde_json::to_string(&masked).expect("serialize masked");
        assert_eq!(json, "{}");
    }

    #[test]
    fn secret_restore_is_noop() {
        let existing = ProxmoxHelperScriptsConfig::default();
        let mut incoming = ProxmoxHelperScriptsConfig::default();
        incoming.restore_secrets_from(&existing);
        let json = serde_json::to_string(&incoming).expect("serialize restored");
        assert_eq!(json, "{}");
    }
}
