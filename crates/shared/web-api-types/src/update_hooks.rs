//! Update hook configuration types for structured hook definitions.
//!
//! Provides types for configuring pre/post-update hooks with:
//! - Predefined templates (systemd services, docker-compose)
//! - Custom commands
//! - Shell selection (bash, sh, powershell)

use serde::{Deserialize, Serialize};

/// Shell type for hook execution.
///
/// Determines which shell interpreter and fail-early settings are used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookShell {
    /// Bash shell with `set -euo pipefail`
    #[default]
    Bash,
    /// POSIX sh with `set -eu`
    Sh,
    /// PowerShell with `$ErrorActionPreference = 'Stop'` (future Windows support)
    PowerShell,
}

impl HookShell {
    /// Returns the string representation of the shell type.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Sh => "sh",
            Self::PowerShell => "powershell",
        }
    }

    /// Parses a string into a HookShell variant.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bash" => Some(Self::Bash),
            "sh" => Some(Self::Sh),
            "powershell" => Some(Self::PowerShell),
            _ => None,
        }
    }
}

/// Systemd service action - explicit, maps directly to command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Docker-compose action - explicit, maps directly to command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Predefined systemd service hook configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemdServiceHook {
    /// Name of the systemd service (e.g., "myapp", "nginx").
    pub service_name: String,
    /// Action to perform on the service.
    pub action: SystemdAction,
}

/// Predefined docker-compose hook configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Predefined hook templates with explicit actions.
///
/// Each variant directly maps to a specific command - no hidden magic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredefinedHook {
    /// Systemd service management (e.g., stop/start a service).
    SystemdService(SystemdServiceHook),
    /// Docker-compose operations (e.g., down/up containers).
    DockerCompose(DockerComposeHook),
}

/// Single hook phase configuration (pre_update or post_update).
///
/// Either `predefined` or `commands` should be set, not both.
/// If both are set, `predefined` takes precedence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Full hooks configuration for a software item.
///
/// Stored in provider_config or software item's config_override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Pre-update hook configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_update: Option<UpdateHookConfig>,
    /// Post-update hook configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_update: Option<UpdateHookConfig>,
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
    fn hook_shell_parse_valid() {
        assert_eq!(HookShell::parse("bash"), Some(HookShell::Bash));
        assert_eq!(HookShell::parse("sh"), Some(HookShell::Sh));
        assert_eq!(HookShell::parse("powershell"), Some(HookShell::PowerShell));
    }

    #[test]
    fn hook_shell_parse_invalid() {
        assert_eq!(HookShell::parse("zsh"), None);
        assert_eq!(HookShell::parse(""), None);
        assert_eq!(HookShell::parse("BASH"), None);
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
