use serde::{Deserialize, Serialize};
use uptrakit_plugin_core::SecretMasking;

/// Configuration for the Proxmox Helper Scripts provider.
///
/// PHS is discovery-only: it reads `/usr/bin/update`, fetches each referenced
/// CT script from `raw.githubusercontent.com`, and analyses the script to
/// determine whether the app is GitHub-managed or APT-managed.  The resulting
/// `DiscoveredSoftware` items carry `github_owner`/`github_repo` or
/// `apt_package` in their `extra` field so the controller can synthesize the
/// appropriate downstream provider config automatically.
///
/// No configuration fields are needed; the config is always serialized as `{}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProxmoxHelperScriptsConfig {}

impl SecretMasking for ProxmoxHelperScriptsConfig {
    fn with_secrets_masked(self) -> Self {
        self
    }

    fn restore_secrets_from(&mut self, _existing: &Self) {}
}

impl ProxmoxHelperScriptsConfig {
    /// Validate the configuration.
    ///
    /// Always succeeds — the PHS provider has no required configuration fields.
    pub fn validate(&self) -> uptrakit_plugin_core::Result<()> {
        Ok(())
    }
}

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
