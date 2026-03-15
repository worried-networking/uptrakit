use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{ConfigFormSchema, HookShell, SecretMasking};

/// Maximum length for a hook command string.
const MAX_COMMAND_LEN: usize = 4096;

/// Configuration for the shell hook plugin.
///
/// Runs arbitrary shell commands before and/or after an update.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellHookConfig {
    /// Shell command to run before the update. Skipped if empty/absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_command: Option<String>,

    /// Shell command to run after the update. Skipped if empty/absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_command: Option<String>,

    /// Whether to run `post_command` even when the update fails.
    ///
    /// Defaults to `true` (always run post-command for cleanup).
    #[serde(default = "default_on_failure")]
    pub on_failure: bool,

    /// Shell interpreter to use. Defaults to Bash.
    #[serde(default)]
    pub shell: HookShell,
}

fn default_on_failure() -> bool {
    true
}

impl ShellHookConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        let has_pre = self
            .pre_command
            .as_ref()
            .is_some_and(|c| !c.trim().is_empty());
        let has_post = self
            .post_command
            .as_ref()
            .is_some_and(|c| !c.trim().is_empty());

        if !has_pre && !has_post {
            return Err("at least one of pre_command or post_command must be set".to_string());
        }

        if let Some(cmd) = &self.pre_command
            && cmd.len() > MAX_COMMAND_LEN
        {
            return Err(format!(
                "pre_command exceeds maximum length of {MAX_COMMAND_LEN}"
            ));
        }
        if let Some(cmd) = &self.post_command
            && cmd.len() > MAX_COMMAND_LEN
        {
            return Err(format!(
                "post_command exceeds maximum length of {MAX_COMMAND_LEN}"
            ));
        }

        Ok(())
    }

    /// Validate a package identifier (no-op for hook plugins).
    pub fn validate_identifier(_value: &str) -> Result<(), String> {
        Ok(())
    }
}

impl ConfigFormSchema for ShellHookConfig {
    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FieldDef> {
        use uptrakit_plugin_infrastructure_core::form_schema::{FieldDef, FieldType, SelectOption};
        vec![
            FieldDef::new("pre_command", "Pre-Update Command")
                .with_type(FieldType::Textarea)
                .with_help_text("Shell command to run before the update"),
            FieldDef::new("post_command", "Post-Update Command")
                .with_type(FieldType::Textarea)
                .with_help_text("Shell command to run after the update"),
            FieldDef::new("on_failure", "Run Post-Command on Failure")
                .with_type(FieldType::Toggle)
                .with_help_text("Whether to run the post-command even when the update fails"),
            FieldDef::new("shell", "Shell")
                .with_type(FieldType::Select)
                .with_options(vec![
                    SelectOption::new("bash", "Bash"),
                    SelectOption::new("sh", "POSIX sh"),
                ])
                .with_help_text("Shell interpreter"),
        ]
    }
}

impl SecretMasking for ShellHookConfig {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_config_pre_only() {
        let config = ShellHookConfig {
            pre_command: Some("echo pre".to_string()),
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn valid_config_post_only() {
        let config = ShellHookConfig {
            pre_command: None,
            post_command: Some("echo post".to_string()),
            on_failure: false,
            shell: HookShell::Sh,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn valid_config_both() {
        let config = ShellHookConfig {
            pre_command: Some("echo pre".to_string()),
            post_command: Some("echo post".to_string()),
            on_failure: true,
            shell: HookShell::Bash,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn invalid_config_neither() {
        let config = ShellHookConfig {
            pre_command: None,
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_config_empty_strings() {
        let config = ShellHookConfig {
            pre_command: Some("  ".to_string()),
            post_command: Some("  ".to_string()),
            on_failure: true,
            shell: HookShell::Bash,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_config_command_too_long() {
        let config = ShellHookConfig {
            pre_command: Some("a".repeat(MAX_COMMAND_LEN + 1)),
            post_command: None,
            on_failure: true,
            shell: HookShell::Bash,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = ShellHookConfig {
            pre_command: Some("echo pre".to_string()),
            post_command: Some("echo post".to_string()),
            on_failure: false,
            shell: HookShell::Sh,
        };
        let json = serde_json::to_value(&config).unwrap();
        let deserialized: ShellHookConfig = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn config_defaults() {
        let json = serde_json::json!({"pre_command": "echo hi"});
        let config: ShellHookConfig = serde_json::from_value(json).unwrap();
        assert!(config.on_failure);
        assert_eq!(config.shell, HookShell::Bash);
    }
}
