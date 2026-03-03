//! Update hook configuration types for structured hook definitions.
//!
//! Provides types for configuring pre/post-update hooks with:
//! - Predefined templates (systemd services, docker-compose)
//! - Custom commands
//! - Shell selection (bash, sh, powershell)

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error returned when hook parameter validation fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field}: {message}")]
pub struct HookValidationError {
    pub field: &'static str,
    pub message: String,
}

/// Validate a systemd service/unit name.
///
/// Allowed: `[a-zA-Z0-9._@:-]+`, max 256 characters.
fn validate_service_name(name: &str) -> Result<(), HookValidationError> {
    if name.is_empty() {
        return Err(HookValidationError {
            field: "service_name",
            message: "must not be empty".to_string(),
        });
    }
    if name.len() > 256 {
        return Err(HookValidationError {
            field: "service_name",
            message: "must be at most 256 characters".to_string(),
        });
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._@:-".contains(c))
    {
        return Err(HookValidationError {
            field: "service_name",
            message: "contains invalid characters (allowed: a-z, A-Z, 0-9, . _ @ : -)".to_string(),
        });
    }
    Ok(())
}

/// Validate a file system path used in hooks.
///
/// Allowed: `[a-zA-Z0-9._/ ~-]+`, max 4096 characters, no `..` traversal.
fn validate_hook_path(value: &str, field: &'static str) -> Result<(), HookValidationError> {
    if value.is_empty() {
        return Err(HookValidationError {
            field,
            message: "must not be empty".to_string(),
        });
    }
    if value.len() > 4096 {
        return Err(HookValidationError {
            field,
            message: "must be at most 4096 characters".to_string(),
        });
    }
    if value.contains("..") {
        return Err(HookValidationError {
            field,
            message: "must not contain '..' path traversal".to_string(),
        });
    }
    if value.contains('\0') {
        return Err(HookValidationError {
            field,
            message: "must not contain null bytes".to_string(),
        });
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._/ ~-".contains(c))
    {
        return Err(HookValidationError {
            field,
            message: "contains invalid characters (allowed: a-z, A-Z, 0-9, . _ / ~ - space)"
                .to_string(),
        });
    }
    Ok(())
}

/// Re-export the canonical `HookShell` type from the shared types crate.
pub use uptrakit_shared_types::HookShell;

/// Systemd service action - explicit, maps directly to command.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SystemdAction {
    Start,
    Stop,
    Restart,
    Reload,
}

impl SystemdAction {
    /// Returns the systemctl command verb for this action.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Reload => "reload",
        }
    }
}

impl std::fmt::Display for SystemdAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Docker-compose action - explicit, maps directly to command.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DockerComposeAction {
    Up,
    Down,
    Restart,
    Pull,
}

impl DockerComposeAction {
    /// Returns the docker-compose command verb for this action.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Restart => "restart",
            Self::Pull => "pull",
        }
    }
}

impl std::fmt::Display for DockerComposeAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Predefined systemd service hook configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemdServiceHook {
    /// Name of the systemd service (e.g., "myapp", "nginx").
    pub service_name: String,
    /// Action to perform on the service.
    pub action: SystemdAction,
}

impl SystemdServiceHook {
    /// Validate that the service name is safe for command execution.
    pub fn validate(&self) -> Result<(), HookValidationError> {
        validate_service_name(&self.service_name)
    }
}

/// Predefined docker-compose hook configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DockerComposeHook {
    /// Action to perform.
    pub action: DockerComposeAction,
    /// Path to the compose file (optional, uses default docker-compose.yml if not set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compose_file: Option<String>,
    /// Project directory to run the command in (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
}

impl DockerComposeHook {
    /// Validate that paths are safe for command execution.
    pub fn validate(&self) -> Result<(), HookValidationError> {
        if let Some(ref compose_file) = self.compose_file {
            validate_hook_path(compose_file, "compose_file")?;
        }
        if let Some(ref project_dir) = self.project_dir {
            validate_hook_path(project_dir, "project_dir")?;
        }
        Ok(())
    }
}

/// Predefined hook templates with explicit actions.
///
/// Each variant directly maps to a specific command - no hidden magic.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PredefinedHook {
    /// Systemd service management (e.g., stop/start a service).
    SystemdService(SystemdServiceHook),
    /// Docker-compose operations (e.g., down/up containers).
    DockerCompose(DockerComposeHook),
}

impl PredefinedHook {
    /// Validate that all parameters are safe for command execution.
    pub fn validate(&self) -> Result<(), HookValidationError> {
        match self {
            Self::SystemdService(hook) => hook.validate(),
            Self::DockerCompose(hook) => hook.validate(),
        }
    }
}

/// Single hook phase configuration (pre_update or post_update).
///
/// Either `predefined` or `commands` should be set, not both.
/// If both are set, `predefined` takes precedence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateHookConfig {
    /// Use a predefined hook template with explicit action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predefined: Option<PredefinedHook>,
    /// Custom commands to execute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<String>>,
    /// Shell to use for execution (default: Bash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<HookShell>,
}

impl UpdateHookConfig {
    /// Validate the hook phase configuration.
    ///
    /// Validates:
    /// - Predefined hook parameters (service names, paths)
    /// - Custom command count (max [`MAX_HOOK_COMMANDS_PER_PHASE`](uptrakit_shared_types::command_validation::MAX_HOOK_COMMANDS_PER_PHASE))
    /// - Custom command length (max [`MAX_COMMAND_LENGTH`](uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH))
    /// - No empty commands in the array
    pub fn validate(&self, phase: &'static str) -> Result<(), HookValidationError> {
        if let Some(ref predefined) = self.predefined {
            predefined.validate()?;
        }
        if let Some(ref commands) = self.commands {
            if commands.len()
                > uptrakit_shared_types::command_validation::MAX_HOOK_COMMANDS_PER_PHASE
            {
                return Err(HookValidationError {
                    field: phase,
                    message: format!(
                        "too many commands ({}, max {})",
                        commands.len(),
                        uptrakit_shared_types::command_validation::MAX_HOOK_COMMANDS_PER_PHASE,
                    ),
                });
            }
            for (i, cmd) in commands.iter().enumerate() {
                if let Err(msg) =
                    uptrakit_shared_types::command_validation::validate_command_length(
                        cmd,
                        &format!("{phase}.commands[{i}]"),
                    )
                {
                    return Err(HookValidationError {
                        field: phase,
                        message: msg,
                    });
                }
            }
        }
        Ok(())
    }
}

/// Full hooks configuration for a software item.
///
/// Stored in plugin_config or software item's config_override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HooksConfig {
    /// Pre-update hook configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_update: Option<UpdateHookConfig>,
    /// Post-update hook configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_update: Option<UpdateHookConfig>,
}

impl HooksConfig {
    /// Validate all hooks in this config (predefined + custom commands).
    pub fn validate(&self) -> Result<(), HookValidationError> {
        if let Some(ref pre) = self.pre_update {
            pre.validate("pre_update")?;
        }
        if let Some(ref post) = self.post_update {
            post.validate("post_update")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── HookShell tests ──────────────────────────────────────────────────────

    #[test]
    fn hook_shell_serde_roundtrip() {
        for shell in [HookShell::Bash, HookShell::Sh, HookShell::PowerShell] {
            let json = serde_json::to_string(&shell).unwrap();
            let deserialized: HookShell = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, shell);
        }
    }

    #[test]
    fn hook_shell_as_str_values() {
        assert_eq!(HookShell::Bash.as_str(), "bash");
        assert_eq!(HookShell::Sh.as_str(), "sh");
        assert_eq!(HookShell::PowerShell.as_str(), "powershell");
    }

    #[test]
    fn hook_shell_from_str_valid() {
        assert_eq!("bash".parse::<HookShell>().ok(), Some(HookShell::Bash));
        assert_eq!("sh".parse::<HookShell>().ok(), Some(HookShell::Sh));
        assert_eq!(
            "powershell".parse::<HookShell>().ok(),
            Some(HookShell::PowerShell)
        );
    }

    #[test]
    fn hook_shell_from_str_invalid() {
        assert!("zsh".parse::<HookShell>().is_err());
        assert!("".parse::<HookShell>().is_err());
        assert!("BASH".parse::<HookShell>().is_err());
    }

    #[test]
    fn hook_shell_default_is_bash() {
        assert_eq!(HookShell::default(), HookShell::Bash);
    }

    // ── SystemdAction tests ──────────────────────────────────────────────────

    #[test]
    fn systemd_action_serde_roundtrip() {
        for action in [
            SystemdAction::Start,
            SystemdAction::Stop,
            SystemdAction::Restart,
            SystemdAction::Reload,
        ] {
            let json = serde_json::to_string(&action).unwrap();
            let deserialized: SystemdAction = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, action);
        }
    }

    #[test]
    fn systemd_action_as_str_values() {
        assert_eq!(SystemdAction::Start.as_str(), "start");
        assert_eq!(SystemdAction::Stop.as_str(), "stop");
        assert_eq!(SystemdAction::Restart.as_str(), "restart");
        assert_eq!(SystemdAction::Reload.as_str(), "reload");
    }

    #[test]
    fn systemd_action_display_matches_as_str() {
        for action in [
            SystemdAction::Start,
            SystemdAction::Stop,
            SystemdAction::Restart,
            SystemdAction::Reload,
        ] {
            assert_eq!(format!("{action}"), action.as_str());
        }
    }

    // ── DockerComposeAction tests ────────────────────────────────────────────

    #[test]
    fn docker_compose_action_serde_roundtrip() {
        for action in [
            DockerComposeAction::Up,
            DockerComposeAction::Down,
            DockerComposeAction::Restart,
            DockerComposeAction::Pull,
        ] {
            let json = serde_json::to_string(&action).unwrap();
            let deserialized: DockerComposeAction = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, action);
        }
    }

    #[test]
    fn docker_compose_action_as_str_values() {
        assert_eq!(DockerComposeAction::Up.as_str(), "up");
        assert_eq!(DockerComposeAction::Down.as_str(), "down");
        assert_eq!(DockerComposeAction::Restart.as_str(), "restart");
        assert_eq!(DockerComposeAction::Pull.as_str(), "pull");
    }

    #[test]
    fn docker_compose_action_display_matches_as_str() {
        for action in [
            DockerComposeAction::Up,
            DockerComposeAction::Down,
            DockerComposeAction::Restart,
            DockerComposeAction::Pull,
        ] {
            assert_eq!(format!("{action}"), action.as_str());
        }
    }

    // ── SystemdServiceHook tests ─────────────────────────────────────────────

    #[test]
    fn systemd_service_hook_serde_roundtrip() {
        let hook = SystemdServiceHook {
            service_name: "myapp".to_string(),
            action: SystemdAction::Stop,
        };
        let json = serde_json::to_string(&hook).unwrap();
        let deserialized: SystemdServiceHook = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, hook);
    }

    // ── DockerComposeHook tests ──────────────────────────────────────────────

    #[test]
    fn docker_compose_hook_serde_roundtrip() {
        let hook = DockerComposeHook {
            action: DockerComposeAction::Up,
            compose_file: Some("docker-compose.prod.yml".to_string()),
            project_dir: Some("/opt/myapp".to_string()),
        };
        let json = serde_json::to_string(&hook).unwrap();
        let deserialized: DockerComposeHook = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, hook);
    }

    #[test]
    fn docker_compose_hook_omits_none_fields() {
        let hook = DockerComposeHook {
            action: DockerComposeAction::Down,
            compose_file: None,
            project_dir: None,
        };
        let json = serde_json::to_string(&hook).unwrap();
        assert!(!json.contains("compose_file"));
        assert!(!json.contains("project_dir"));
    }

    // ── PredefinedHook tests ─────────────────────────────────────────────────

    #[test]
    fn predefined_hook_systemd_serde_roundtrip() {
        let hook = PredefinedHook::SystemdService(SystemdServiceHook {
            service_name: "nginx".to_string(),
            action: SystemdAction::Restart,
        });
        let json = serde_json::to_string(&hook).unwrap();
        assert!(json.contains("systemd_service"));
        let deserialized: PredefinedHook = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, hook);
    }

    #[test]
    fn predefined_hook_docker_compose_serde_roundtrip() {
        let hook = PredefinedHook::DockerCompose(DockerComposeHook {
            action: DockerComposeAction::Pull,
            compose_file: None,
            project_dir: Some("/app".to_string()),
        });
        let json = serde_json::to_string(&hook).unwrap();
        assert!(json.contains("docker_compose"));
        let deserialized: PredefinedHook = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, hook);
    }

    // ── UpdateHookConfig tests ───────────────────────────────────────────────

    #[test]
    fn update_hook_config_with_predefined() {
        let config = UpdateHookConfig {
            predefined: Some(PredefinedHook::SystemdService(SystemdServiceHook {
                service_name: "myapp".to_string(),
                action: SystemdAction::Stop,
            })),
            commands: None,
            shell: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: UpdateHookConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn update_hook_config_with_commands() {
        let config = UpdateHookConfig {
            predefined: None,
            commands: Some(vec![
                "echo 'Starting backup'".to_string(),
                "backup.sh".to_string(),
            ]),
            shell: Some(HookShell::Bash),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: UpdateHookConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn update_hook_config_omits_none_fields() {
        let config = UpdateHookConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(json, "{}");
    }

    // ── HooksConfig tests ────────────────────────────────────────────────────

    #[test]
    fn hooks_config_full_roundtrip() {
        let config = HooksConfig {
            pre_update: Some(UpdateHookConfig {
                predefined: Some(PredefinedHook::SystemdService(SystemdServiceHook {
                    service_name: "myapp".to_string(),
                    action: SystemdAction::Stop,
                })),
                commands: None,
                shell: None,
            }),
            post_update: Some(UpdateHookConfig {
                predefined: Some(PredefinedHook::SystemdService(SystemdServiceHook {
                    service_name: "myapp".to_string(),
                    action: SystemdAction::Start,
                })),
                commands: None,
                shell: None,
            }),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: HooksConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn hooks_config_empty_roundtrip() {
        let config = HooksConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(json, "{}");
        let deserialized: HooksConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    // ── JSON format tests ────────────────────────────────────────────────────

    #[test]
    fn hooks_config_json_format_predefined_systemd() {
        let json = r#"{
            "pre_update": {
                "predefined": {
                    "systemd_service": {
                        "service_name": "myapp",
                        "action": "stop"
                    }
                }
            },
            "post_update": {
                "predefined": {
                    "systemd_service": {
                        "service_name": "myapp",
                        "action": "start"
                    }
                }
            }
        }"#;
        let config: HooksConfig = serde_json::from_str(json).unwrap();

        let pre = config.pre_update.unwrap();
        assert!(matches!(
            pre.predefined,
            Some(PredefinedHook::SystemdService(SystemdServiceHook {
                action: SystemdAction::Stop,
                ..
            }))
        ));

        let post = config.post_update.unwrap();
        assert!(matches!(
            post.predefined,
            Some(PredefinedHook::SystemdService(SystemdServiceHook {
                action: SystemdAction::Start,
                ..
            }))
        ));
    }

    #[test]
    fn hooks_config_json_format_predefined_docker_compose() {
        let json = r#"{
            "pre_update": {
                "predefined": {
                    "docker_compose": {
                        "action": "down",
                        "project_dir": "/opt/myapp"
                    }
                }
            },
            "post_update": {
                "predefined": {
                    "docker_compose": {
                        "action": "up",
                        "project_dir": "/opt/myapp"
                    }
                }
            }
        }"#;
        let config: HooksConfig = serde_json::from_str(json).unwrap();

        let pre = config.pre_update.unwrap();
        if let Some(PredefinedHook::DockerCompose(hook)) = pre.predefined {
            assert_eq!(hook.action, DockerComposeAction::Down);
            assert_eq!(hook.project_dir, Some("/opt/myapp".to_string()));
        } else {
            panic!("Expected DockerCompose predefined hook");
        }
    }

    // ── Validation tests ──────────────────────────────────────────────────

    #[test]
    fn validate_systemd_service_name_valid() {
        let hook = SystemdServiceHook {
            service_name: "nginx".to_string(),
            action: SystemdAction::Restart,
        };
        assert!(hook.validate().is_ok());
    }

    #[test]
    fn validate_systemd_service_name_with_special_chars() {
        for name in ["my-app.service", "foo@bar", "my_app:v2", "postgresql-15"] {
            let hook = SystemdServiceHook {
                service_name: name.to_string(),
                action: SystemdAction::Restart,
            };
            assert!(hook.validate().is_ok(), "should accept: {name}");
        }
    }

    #[test]
    fn validate_systemd_service_name_rejects_shell_metacharacters() {
        for name in [
            "; rm -rf /",
            "myapp$(whoami)",
            "myapp`id`",
            "my|app",
            "my&app",
            "my\napp",
            "my\0app",
            "my app",
        ] {
            let hook = SystemdServiceHook {
                service_name: name.to_string(),
                action: SystemdAction::Restart,
            };
            assert!(hook.validate().is_err(), "should reject: {name:?}");
        }
    }

    #[test]
    fn validate_systemd_service_name_empty() {
        let hook = SystemdServiceHook {
            service_name: String::new(),
            action: SystemdAction::Stop,
        };
        assert!(hook.validate().is_err());
    }

    #[test]
    fn validate_docker_compose_paths_valid() {
        let hook = DockerComposeHook {
            action: DockerComposeAction::Up,
            compose_file: Some("docker-compose.prod.yml".to_string()),
            project_dir: Some("/opt/my-app".to_string()),
        };
        assert!(hook.validate().is_ok());
    }

    #[test]
    fn validate_docker_compose_path_with_spaces() {
        let hook = DockerComposeHook {
            action: DockerComposeAction::Up,
            compose_file: None,
            project_dir: Some("/opt/my app".to_string()),
        };
        assert!(hook.validate().is_ok());
    }

    #[test]
    fn validate_docker_compose_rejects_traversal() {
        let hook = DockerComposeHook {
            action: DockerComposeAction::Up,
            compose_file: None,
            project_dir: Some("/opt/../etc/shadow".to_string()),
        };
        assert!(hook.validate().is_err());
    }

    #[test]
    fn validate_docker_compose_rejects_shell_metacharacters() {
        for path in [
            "/opt/$(whoami)",
            "/opt/`id`",
            "/opt;rm -rf /",
            "/opt|cat /etc/passwd",
            "/opt&bg",
        ] {
            let hook = DockerComposeHook {
                action: DockerComposeAction::Up,
                compose_file: None,
                project_dir: Some(path.to_string()),
            };
            assert!(hook.validate().is_err(), "should reject: {path:?}");
        }
    }

    #[test]
    fn validate_hooks_config_validates_predefined() {
        let config = HooksConfig {
            pre_update: Some(UpdateHookConfig {
                predefined: Some(PredefinedHook::SystemdService(SystemdServiceHook {
                    service_name: "; rm -rf /".to_string(),
                    action: SystemdAction::Stop,
                })),
                commands: None,
                shell: None,
            }),
            post_update: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_hooks_config_ok_with_no_predefined() {
        let config = HooksConfig {
            pre_update: Some(UpdateHookConfig {
                predefined: None,
                commands: Some(vec!["echo hello".to_string()]),
                shell: None,
            }),
            post_update: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_hooks_config_custom_command_over_limit_rejected() {
        let long_cmd = "x".repeat(
            uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH + 1,
        );
        let config = HooksConfig {
            pre_update: Some(UpdateHookConfig {
                predefined: None,
                commands: Some(vec![long_cmd]),
                shell: None,
            }),
            post_update: None,
        };
        let err = config.validate().unwrap_err();
        assert_eq!(err.field, "pre_update");
        assert!(err.message.contains("exceeds maximum length"));
    }

    #[test]
    fn validate_hooks_config_too_many_commands_rejected() {
        let commands = (0..=uptrakit_shared_types::command_validation::MAX_HOOK_COMMANDS_PER_PHASE)
            .map(|i| format!("echo {i}"))
            .collect();
        let config = HooksConfig {
            pre_update: Some(UpdateHookConfig {
                predefined: None,
                commands: Some(commands),
                shell: None,
            }),
            post_update: None,
        };
        let err = config.validate().unwrap_err();
        assert_eq!(err.field, "pre_update");
        assert!(err.message.contains("too many commands"));
    }

    #[test]
    fn validate_hooks_config_empty_command_rejected() {
        let config = HooksConfig {
            post_update: Some(UpdateHookConfig {
                predefined: None,
                commands: Some(vec!["".to_string()]),
                shell: None,
            }),
            pre_update: None,
        };
        let err = config.validate().unwrap_err();
        assert_eq!(err.field, "post_update");
        assert!(err.message.contains("must not be empty"));
    }

    #[test]
    fn validate_hooks_config_commands_at_limit_ok() {
        let commands = (0..uptrakit_shared_types::command_validation::MAX_HOOK_COMMANDS_PER_PHASE)
            .map(|i| format!("echo {i}"))
            .collect();
        let config = HooksConfig {
            pre_update: Some(UpdateHookConfig {
                predefined: None,
                commands: Some(commands),
                shell: None,
            }),
            post_update: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_hooks_config_post_update_validated_too() {
        let long_cmd = "x".repeat(
            uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH + 1,
        );
        let config = HooksConfig {
            pre_update: None,
            post_update: Some(UpdateHookConfig {
                predefined: None,
                commands: Some(vec![long_cmd]),
                shell: None,
            }),
        };
        let err = config.validate().unwrap_err();
        assert_eq!(err.field, "post_update");
    }

    #[test]
    fn hooks_config_json_format_custom_commands() {
        let json = r#"{
            "pre_update": {
                "commands": ["echo 'Starting backup'", "backup.sh"],
                "shell": "bash"
            },
            "post_update": {
                "commands": ["systemctl restart myapp"],
                "shell": "bash"
            }
        }"#;
        let config: HooksConfig = serde_json::from_str(json).unwrap();

        let pre = config.pre_update.unwrap();
        assert_eq!(
            pre.commands,
            Some(vec![
                "echo 'Starting backup'".to_string(),
                "backup.sh".to_string()
            ])
        );
        assert_eq!(pre.shell, Some(HookShell::Bash));
    }
}
