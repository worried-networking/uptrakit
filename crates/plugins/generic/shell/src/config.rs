use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

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
#[non_exhaustive]
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

    /// Indicate that a successful update requires a host restart to take effect.
    ///
    /// When `true`, the controller transitions the software item to
    /// `AwaitingRestart` after a successful update instead of marking it
    /// `UpToDate` immediately. The item moves to `UpToDate` once the agent
    /// reports a version that matches the target (i.e. after the host has
    /// restarted and the update has been applied).
    ///
    /// Defaults to `false`. Set to `true` for update scripts that install
    /// packages requiring a reboot or service restart to become active.
    #[serde(default)]
    pub resumable: bool,
}

impl PluginConfig for ShellConfig {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if self.version_command.is_none() && self.update_command.is_none() {
            return Err(PluginConfigValidationError::Contract(
                "at least one of version_command or update_command must be set".to_string(),
            ));
        }
        if let Some(ref cmd) = self.version_command
            && let Err(e) = uptrakit_shared_types::command_validation::validate_command_length(
                cmd,
                "version_command",
            )
        {
            return Err(PluginConfigValidationError::invalid_field(
                "version_command",
                e,
            ));
        }
        if let Some(ref cmd) = self.update_command
            && let Err(e) = uptrakit_shared_types::command_validation::validate_command_length(
                cmd,
                "update_command",
            )
        {
            return Err(PluginConfigValidationError::invalid_field(
                "update_command",
                e,
            ));
        }
        Ok(())
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("version_command", "Version Command")
                .with_type(FormFieldType::Textarea)
                .with_help_text("Shell command to detect the installed version (supports {package_identifier})"),
            FormFieldDescriptor::new("update_command", "Update Command")
                .with_type(FormFieldType::Textarea)
                .with_help_text("Shell command to execute an update (supports {version}, {tag}, {package_identifier})"),
            FormFieldDescriptor::new("prefer_interactive", "Interactive Mode")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Allocate a PTY for the update command. Enable for scripts that read from /dev/tty (e.g. interactive prompts)."),
            FormFieldDescriptor::new("resumable", "Resumable (Restart Required)")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Mark the update as requiring a host restart to take effect. The item stays in AwaitingRestart until the agent reports the expected version after reboot."),
        ]
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;

    #[test]
    fn validate_both_none_fails() {
        let config = ShellConfig::default();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("at least one"));
    }

    #[test]
    fn validate_version_command_only_passes() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: None,
            prefer_interactive: false,
            resumable: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_update_command_only_passes() {
        let config = ShellConfig {
            version_command: None,
            update_command: Some("apt-get install -y myapp".to_string()),
            prefer_interactive: false,
            resumable: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_both_set_passes() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: Some("apt-get install -y myapp".to_string()),
            prefer_interactive: false,
            resumable: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn serde_absent_fields_omitted() {
        let config = ShellConfig {
            version_command: Some("myapp --version".to_string()),
            update_command: None,
            prefer_interactive: false,
            resumable: false,
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
            resumable: false,
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
            resumable: false,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ShellConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.version_command, config.version_command);
        assert_eq!(deserialized.update_command, config.update_command);
        assert_eq!(deserialized.prefer_interactive, config.prefer_interactive);
        assert_eq!(deserialized.resumable, config.resumable);
    }

    #[test]
    fn validate_version_command_at_limit_passes() {
        let cmd = "x".repeat(uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH);
        let config = ShellConfig {
            version_command: Some(cmd),
            update_command: None,
            prefer_interactive: false,
            resumable: false,
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
            resumable: false,
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
            resumable: false,
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
            resumable: false,
        };
        let masked = config.clone().with_secrets_masked();
        assert_eq!(masked.version_command, config.version_command);
        assert_eq!(masked.update_command, config.update_command);
        assert_eq!(masked.prefer_interactive, config.prefer_interactive);
        assert_eq!(masked.resumable, config.resumable);
    }

    #[test]
    fn test_shell_plugin_resumable_config_defaults_false() {
        let config: ShellConfig = serde_json::from_str(r#"{"update_command":"echo hi"}"#).unwrap();
        assert!(!config.resumable);
    }

    #[test]
    fn test_shell_plugin_resumable_config_true() {
        let config: ShellConfig =
            serde_json::from_str(r#"{"update_command":"echo hi","resumable":true}"#).unwrap();
        assert!(config.resumable);
    }
}
