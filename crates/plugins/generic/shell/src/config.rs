use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::SecretMasking;

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

    /// Request PTY-backed interactive execution for the update command.
    ///
    /// When `true`, the controller sets `interactive: true` in the
    /// `ExecuteUpdate` wire payload, causing the agent to allocate a PTY and
    /// keep stdin open during the update. This enables scripts that read from
    /// `/dev/tty` (e.g. the Proxmox Helper Scripts `/usr/bin/update` for
    /// low-storage warnings) to function correctly.
    ///
    /// Defaults to `false`. Set to `true` for update commands that require an
    /// interactive terminal — typically those that may prompt the user for input.
    #[serde(default)]
    pub prefer_interactive: bool,
}

impl ShellConfig {
    /// Validate a Shell plugin package identifier string.
    ///
    /// Always succeeds — the Shell plugin does not impose constraints on the
    /// identifier value; the value is shell-escaped before substitution into
    /// `version_command` and `update_command` at runtime.
    ///
    /// Called by the plugin registry's `validate_package_identifier` dispatch.
    pub fn validate_identifier(_value: &str) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Validate the configuration.
    ///
    /// Fails when **both** `version_command` and `update_command` are `None`
    /// — a no-op config is invalid. Either field set alone is valid.
    /// Also validates that command strings do not exceed the maximum length.
    pub fn validate(&self) -> Result<()> {
        if self.version_command.is_none() && self.update_command.is_none() {
            rootcause::bail!(ShellError::Configuration(
                "at least one of version_command or update_command must be set".to_string()
            ));
        }
        if let Some(ref cmd) = self.version_command
            && let Err(e) = uptrakit_shared_types::command_validation::validate_command_length(
                cmd,
                "version_command",
            )
        {
            rootcause::bail!(ShellError::Configuration(e));
        }
        if let Some(ref cmd) = self.update_command
            && let Err(e) = uptrakit_shared_types::command_validation::validate_command_length(
                cmd,
                "update_command",
            )
        {
            rootcause::bail!(ShellError::Configuration(e));
        }
        Ok(())
    }
}

impl uptrakit_plugin_infrastructure_core::ConfigFormSchema for ShellConfig {
    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType};
        vec![
            FieldDef::new("version_command", "Version Command")
                .with_type(FieldType::Textarea)
                .with_help_text("Shell command to detect the installed version (supports {package_identifier})"),
            FieldDef::new("update_command", "Update Command")
                .with_type(FieldType::Textarea)
                .with_help_text("Shell command to execute an update (supports {version}, {tag}, {package_identifier})"),
            FieldDef::new("prefer_interactive", "Interactive Mode")
                .with_type(FieldType::Toggle)
                .with_help_text("Allocate a PTY for the update command. Enable for scripts that read from /dev/tty (e.g. interactive prompts)."),
        ]
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
            prefer_interactive: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_update_command_only_passes() {
        let config = ShellConfig {
            version_command: None,
            update_command: Some("apt-get install -y myapp".to_string()),
            prefer_interactive: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_both_set_passes() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: Some("apt-get install -y myapp".to_string()),
            prefer_interactive: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serde_absent_fields_omitted() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: None,
            prefer_interactive: false,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("update_command"));
        assert!(json.contains("version_command"));
    }

    #[test]
    fn serde_prefer_interactive_roundtrip() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: Some("myapp update".to_string()),
            prefer_interactive: true,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ShellConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(deserialized.prefer_interactive);
    }

    #[test]
    fn serde_prefer_interactive_defaults_to_false() {
        let json = r#"{"version_command":"echo 1","update_command":"echo 2"}"#;
        let deserialized: ShellConfig = serde_json::from_str(json).expect("deserialize");
        assert!(!deserialized.prefer_interactive);
    }

    #[test]
    fn serde_roundtrip() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: Some("myapp update".to_string()),
            prefer_interactive: false,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ShellConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.version_command, config.version_command);
        assert_eq!(deserialized.update_command, config.update_command);
        assert_eq!(deserialized.prefer_interactive, config.prefer_interactive);
    }

    #[test]
    fn validate_version_command_at_limit_passes() {
        let cmd = "x".repeat(uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH);
        let config = ShellConfig {
            version_command: Some(cmd),
            update_command: None,
            prefer_interactive: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_version_command_over_limit_rejected() {
        let cmd = "x".repeat(uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH + 1);
        let config = ShellConfig {
            version_command: Some(cmd),
            update_command: None,
            prefer_interactive: false,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("version_command"));
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn validate_update_command_over_limit_rejected() {
        let cmd = "x".repeat(uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH + 1);
        let config = ShellConfig {
            version_command: Some("echo ok".to_string()),
            update_command: Some(cmd),
            prefer_interactive: false,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("update_command"));
    }

    #[test]
    fn masking_is_noop() {
        let config = ShellConfig {
            version_command: Some("cmd".to_string()),
            update_command: Some("update".to_string()),
            prefer_interactive: false,
        };
        let masked = config.clone().with_secrets_masked();
        assert_eq!(masked.version_command, config.version_command);
        assert_eq!(masked.update_command, config.update_command);
        assert_eq!(masked.prefer_interactive, config.prefer_interactive);
    }
}
