use serde::{Deserialize, Serialize};
use uptrakit_plugin_core::SecretMasking;

use crate::error::{Result, ShellError};

/// Configuration for the Shell plugin.
///
/// Provides two independently-optional shell commands for agent-side operations:
/// - `version_command`: detect the installed version of a package.
/// - `update_command`: execute an update for a package.
///
/// At least one field must be set — a config with both absent is invalid.
///
/// Both command strings support placeholder substitution at runtime:
/// - `{package_identifier}` — the software item's package identifier
///   (shell-escaped before substitution).
/// - `{version}` — the target version string (`update_command` only;
///   shell-escaped before substitution).
/// - `{tag}` — the upstream release tag (`update_command` only; falls back to
///   `{version}` when no release info is available; shell-escaped).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShellConfig {
    /// Shell command to detect the installed version.
    ///
    /// Supports the `{package_identifier}` placeholder (shell-escaped at
    /// runtime). The first non-empty trimmed line of stdout is used as the
    /// version string. If absent, `detect_installed_version()` returns
    /// `Ok(None)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_command: Option<String>,

    /// Shell command to execute an update.
    ///
    /// Supports `{version}`, `{tag}`, and `{package_identifier}` placeholders
    /// (all shell-escaped at runtime). `{tag}` falls back to `{version}` when
    /// `release_info` is absent. If absent, `execute_update()` returns an
    /// unsupported error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_command: Option<String>,
}

impl ShellConfig {
    /// Validate the configuration.
    ///
    /// Fails when **both** `version_command` and `update_command` are `None`
    /// — a no-op config is invalid. Either field set alone is valid.
    pub fn validate(&self) -> Result<()> {
        if self.version_command.is_none() && self.update_command.is_none() {
            rootcause::bail!(ShellError::Configuration(
                "at least one of version_command or update_command must be set".to_string()
            ));
        }
        Ok(())
    }
}

/// Shell plugin has no secret fields — masking is a no-op.
impl SecretMasking for ShellConfig {
    fn with_secrets_masked(self) -> Self {
        self
    }

    fn restore_secrets_from(&mut self, _existing: &Self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_both_none_fails() {
        let config = ShellConfig::default();
        assert!(config.validate().is_err());
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("at least one")
        );
    }

    #[test]
    fn validate_version_command_only_passes() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_update_command_only_passes() {
        let config = ShellConfig {
            version_command: None,
            update_command: Some("apt-get install -y myapp".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_both_set_passes() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: Some("apt-get install -y myapp".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serde_absent_fields_omitted() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: None,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("update_command"));
        assert!(json.contains("version_command"));
    }

    #[test]
    fn serde_roundtrip() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: Some("myapp update".to_string()),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ShellConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.version_command, config.version_command);
        assert_eq!(deserialized.update_command, config.update_command);
    }

    #[test]
    fn masking_is_noop() {
        let config = ShellConfig {
            version_command: Some("cmd".to_string()),
            update_command: Some("update".to_string()),
        };
        let masked = config.clone().with_secrets_masked();
        assert_eq!(masked.version_command, config.version_command);
        assert_eq!(masked.update_command, config.update_command);
    }
}
