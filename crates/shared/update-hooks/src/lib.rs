//! Hook merging and resolution logic for software updates.
//!
//! Supports two configuration formats:
//!
//! ## Legacy format (backward compatible)
//! - `pre_update_commands: Vec<String>`
//! - `post_update_commands: Vec<String>`
//!
//! ## Structured format (new)
//! - `hooks: HooksConfig` with predefined templates or custom commands
//!
//! Merges pre/post-update commands from plugin config (base) with
//! software item config_override (override). The override completely
//! replaces the base when present.

use uptrakit_internal_wire::HookCommand;
use uptrakit_web_api_types::update_hooks::{
    DockerComposeAction, DockerComposeHook, HookShell, HooksConfig, PredefinedHook,
    SystemdServiceHook, UpdateHookConfig,
};

/// Resolved hooks ready for execution.
#[derive(Debug, Clone, Default)]
pub struct ResolvedHooks {
    /// Pre-update hook commands to execute.
    pub pre_update_hooks: Vec<HookCommand>,
    /// Post-update hook commands to execute.
    pub post_update_hooks: Vec<HookCommand>,
}

/// Resolve hooks from plugin config and config_override.
///
/// Supports both legacy format (`pre_update_commands`, `post_update_commands`)
/// and new structured format (`hooks: HooksConfig`).
///
/// # Arguments
///
/// * `plugin_config` - Base plugin configuration JSON
/// * `config_override` - Optional override configuration JSON from software item
///
/// # Returns
///
/// Resolved hooks with commands and shell type.
pub fn resolve_hooks(
    plugin_config: &serde_json::Value,
    config_override: Option<&serde_json::Value>,
) -> ResolvedHooks {
    // First, try to parse structured hooks from override, then from base
    let merged_hooks = merge_hooks_config(plugin_config, config_override);

    if let Some(hooks_config) = merged_hooks {
        return resolve_hooks_config(&hooks_config);
    }

    // Fall back to legacy format (custom commands → Shell variant)
    let (pre, post) = merge_hooks(plugin_config, config_override);
    let shell = HookShell::default();
    ResolvedHooks {
        pre_update_hooks: pre
            .into_iter()
            .map(|cmd| HookCommand::Shell {
                command: cmd,
                shell,
            })
            .collect(),
        post_update_hooks: post
            .into_iter()
            .map(|cmd| HookCommand::Shell {
                command: cmd,
                shell,
            })
            .collect(),
    }
}

/// Parse HooksConfig from a JSON value's "hooks" key.
fn parse_hooks_config(value: &serde_json::Value) -> Option<HooksConfig> {
    value
        .get("hooks")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Merge structured hooks config from plugin config and override.
///
/// Strategy: Override's hooks completely replace base's hooks when present.
fn merge_hooks_config(
    plugin_config: &serde_json::Value,
    config_override: Option<&serde_json::Value>,
) -> Option<HooksConfig> {
    // Check if override has hooks
    if let Some(override_val) = config_override
        && override_val.get("hooks").is_some()
    {
        return parse_hooks_config(override_val);
    }

    // Fall back to base config hooks
    parse_hooks_config(plugin_config)
}

/// Resolve a HooksConfig into hook commands.
fn resolve_hooks_config(config: &HooksConfig) -> ResolvedHooks {
    let pre_hooks = resolve_update_hook_config(config.pre_update.as_ref());
    let post_hooks = resolve_update_hook_config(config.post_update.as_ref());

    ResolvedHooks {
        pre_update_hooks: pre_hooks,
        post_update_hooks: post_hooks,
    }
}

/// Resolve a single UpdateHookConfig into hook commands.
fn resolve_update_hook_config(config: Option<&UpdateHookConfig>) -> Vec<HookCommand> {
    let Some(config) = config else {
        return Vec::new();
    };

    let shell = config.shell.unwrap_or_default();

    // Predefined takes precedence over commands → produces Exec variants
    if let Some(predefined) = &config.predefined {
        return vec![resolve_predefined_hook(predefined)];
    }

    // Fall back to custom commands → Shell variants
    config
        .commands
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|cmd| HookCommand::Shell {
            command: cmd.clone(),
            shell,
        })
        .collect()
}

/// Resolve a predefined hook to a structured `HookCommand::Exec`.
///
/// Direct mapping — never passes through a shell:
/// - `SystemdService { service_name: "x", action: Stop }` → `Exec { program: "systemctl", args: ["stop", "x"] }`
/// - `DockerCompose { action: Down, project_dir: Some("/opt/x"), .. }` → `Exec { program: "docker-compose", args: ["down"], working_dir: "/opt/x" }`
pub fn resolve_predefined_hook(hook: &PredefinedHook) -> HookCommand {
    match hook {
        PredefinedHook::SystemdService(systemd) => resolve_systemd_hook(systemd),
        PredefinedHook::DockerCompose(compose) => resolve_docker_compose_hook(compose),
        _ => {
            tracing::warn!(hook = ?hook, "unknown PredefinedHook variant; skipping hook (no-op)");
            HookCommand::Exec {
                program: "true".to_string(),
                args: vec![],
                working_dir: None,
            }
        }
    }
}

/// Resolve a systemd service hook to a structured command.
fn resolve_systemd_hook(hook: &SystemdServiceHook) -> HookCommand {
    HookCommand::Exec {
        program: "systemctl".to_string(),
        args: vec![hook.action.as_str().to_string(), hook.service_name.clone()],
        working_dir: None,
    }
}

/// Resolve a docker-compose hook to a structured command.
fn resolve_docker_compose_hook(hook: &DockerComposeHook) -> HookCommand {
    let mut args = Vec::new();

    // Add compose file flag if specified
    if let Some(compose_file) = &hook.compose_file {
        args.push("-f".to_string());
        args.push(compose_file.clone());
    }

    // Add the action
    args.push(hook.action.as_str().to_string());

    // Add -d flag for "up" action
    if hook.action == DockerComposeAction::Up {
        args.push("-d".to_string());
    }

    HookCommand::Exec {
        program: "docker-compose".to_string(),
        args,
        working_dir: hook.project_dir.clone(),
    }
}

/// Extract string array from JSON value at the given key.
fn extract_string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Merge hooks from plugin config and config_override.
///
/// Strategy: Override keys completely replace base keys. Missing keys
/// fall back to base.
///
/// # Arguments
///
/// * `plugin_config` - Base plugin configuration JSON
/// * `config_override` - Optional override configuration JSON from software item
///
/// # Returns
///
/// Tuple of `(pre_update_commands, post_update_commands)`
pub fn merge_hooks(
    plugin_config: &serde_json::Value,
    config_override: Option<&serde_json::Value>,
) -> (Vec<String>, Vec<String>) {
    let base_pre = extract_string_array(plugin_config, "pre_update_commands");
    let base_post = extract_string_array(plugin_config, "post_update_commands");

    let Some(override_config) = config_override else {
        return (base_pre, base_post);
    };

    // Check if override has the key - if so, use it (even if empty), otherwise fall back
    let pre = if override_config.get("pre_update_commands").is_some() {
        extract_string_array(override_config, "pre_update_commands")
    } else {
        base_pre
    };

    let post = if override_config.get("post_update_commands").is_some() {
        extract_string_array(override_config, "post_update_commands")
    } else {
        base_post
    };

    (pre, post)
}

/// Merge plugin config and override into a single config object.
///
/// The override object's keys completely replace the base object's keys.
pub fn merge_config(
    plugin_config: &serde_json::Value,
    config_override: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut merged = plugin_config.clone();

    if let Some(override_config) = config_override
        && let (Some(base_obj), Some(over_obj)) =
            (merged.as_object_mut(), override_config.as_object())
    {
        for (k, v) in over_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }

    merged
}

/// Resolve the effective configuration by merging three layers:
///
/// 1. **Type settings** (`plugin_type_settings.config`) — tenant-level defaults
/// 2. **Profile config** (`plugin_configs.config`) — credential/access profile
/// 3. **Assignment config** (`host_software_item_plugins.config`) — per-item overrides
///
/// Each layer's top-level keys override the previous layer (shallow merge).
/// `None` layers are skipped.
pub fn resolve_effective_config(
    type_settings: Option<&serde_json::Value>,
    profile_config: Option<&serde_json::Value>,
    assignment_config: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut merged = serde_json::Value::Object(Default::default());

    // Layer 1: type settings as the base.
    if let Some(ts) = type_settings {
        shallow_merge_into(&mut merged, ts);
    }

    // Layer 2: profile config overrides type settings.
    if let Some(pc) = profile_config {
        shallow_merge_into(&mut merged, pc);
    }

    // Layer 3: assignment config overrides everything.
    if let Some(ac) = assignment_config {
        shallow_merge_into(&mut merged, ac);
    }

    merged
}

/// Shallow-merge `source` object keys into `target`.
///
/// Non-object sources are ignored. Each source key completely replaces
/// the target key (no deep merge).
fn shallow_merge_into(target: &mut serde_json::Value, source: &serde_json::Value) {
    if let (Some(target_obj), Some(source_obj)) = (target.as_object_mut(), source.as_object()) {
        for (k, v) in source_obj {
            target_obj.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uptrakit_web_api_types::update_hooks::{
        DockerComposeAction, DockerComposeHook, PredefinedHook, SystemdAction, SystemdServiceHook,
    };

    // ── Predefined hook resolution tests ─────────────────────────────────────

    #[test]
    fn resolve_systemd_hook_stop() {
        let hook = PredefinedHook::SystemdService(SystemdServiceHook {
            service_name: "myapp".to_string(),
            action: SystemdAction::Stop,
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "systemctl".to_string(),
                args: vec!["stop".to_string(), "myapp".to_string()],
                working_dir: None,
            }
        );
    }

    #[test]
    fn resolve_systemd_hook_start() {
        let hook = PredefinedHook::SystemdService(SystemdServiceHook {
            service_name: "nginx".to_string(),
            action: SystemdAction::Start,
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "systemctl".to_string(),
                args: vec!["start".to_string(), "nginx".to_string()],
                working_dir: None,
            }
        );
    }

    #[test]
    fn resolve_systemd_hook_restart() {
        let hook = PredefinedHook::SystemdService(SystemdServiceHook {
            service_name: "postgresql".to_string(),
            action: SystemdAction::Restart,
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "systemctl".to_string(),
                args: vec!["restart".to_string(), "postgresql".to_string()],
                working_dir: None,
            }
        );
    }

    #[test]
    fn resolve_systemd_hook_reload() {
        let hook = PredefinedHook::SystemdService(SystemdServiceHook {
            service_name: "apache2".to_string(),
            action: SystemdAction::Reload,
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "systemctl".to_string(),
                args: vec!["reload".to_string(), "apache2".to_string()],
                working_dir: None,
            }
        );
    }

    #[test]
    fn resolve_docker_compose_down_with_project_dir() {
        let hook = PredefinedHook::DockerCompose(DockerComposeHook {
            action: DockerComposeAction::Down,
            compose_file: None,
            project_dir: Some("/opt/myapp".to_string()),
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "docker-compose".to_string(),
                args: vec!["down".to_string()],
                working_dir: Some("/opt/myapp".to_string()),
            }
        );
    }

    #[test]
    fn resolve_docker_compose_up_with_project_dir() {
        let hook = PredefinedHook::DockerCompose(DockerComposeHook {
            action: DockerComposeAction::Up,
            compose_file: None,
            project_dir: Some("/opt/myapp".to_string()),
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "docker-compose".to_string(),
                args: vec!["up".to_string(), "-d".to_string()],
                working_dir: Some("/opt/myapp".to_string()),
            }
        );
    }

    #[test]
    fn resolve_docker_compose_with_compose_file() {
        let hook = PredefinedHook::DockerCompose(DockerComposeHook {
            action: DockerComposeAction::Pull,
            compose_file: Some("docker-compose.prod.yml".to_string()),
            project_dir: Some("/app".to_string()),
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "docker-compose".to_string(),
                args: vec![
                    "-f".to_string(),
                    "docker-compose.prod.yml".to_string(),
                    "pull".to_string(),
                ],
                working_dir: Some("/app".to_string()),
            }
        );
    }

    #[test]
    fn resolve_docker_compose_restart_no_project_dir() {
        let hook = PredefinedHook::DockerCompose(DockerComposeHook {
            action: DockerComposeAction::Restart,
            compose_file: None,
            project_dir: None,
        });
        let cmd = resolve_predefined_hook(&hook);
        assert_eq!(
            cmd,
            HookCommand::Exec {
                program: "docker-compose".to_string(),
                args: vec!["restart".to_string()],
                working_dir: None,
            }
        );
    }

    // ── Structured hooks resolution tests ────────────────────────────────────

    #[test]
    fn resolve_hooks_structured_systemd() {
        let config = json!({
            "hooks": {
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
            }
        });

        let resolved = resolve_hooks(&config, None);

        assert_eq!(resolved.pre_update_hooks.len(), 1);
        assert!(matches!(
            &resolved.pre_update_hooks[0],
            HookCommand::Exec { program, args, .. }
                if program == "systemctl" && args == &["stop", "myapp"]
        ));
        assert_eq!(resolved.post_update_hooks.len(), 1);
        assert!(matches!(
            &resolved.post_update_hooks[0],
            HookCommand::Exec { program, args, .. }
                if program == "systemctl" && args == &["start", "myapp"]
        ));
    }

    #[test]
    fn resolve_hooks_structured_docker_compose() {
        let config = json!({
            "hooks": {
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
            }
        });

        let resolved = resolve_hooks(&config, None);

        assert!(matches!(
            &resolved.pre_update_hooks[0],
            HookCommand::Exec { program, args, working_dir }
                if program == "docker-compose" && args == &["down"]
                    && *working_dir == Some("/opt/myapp".to_string())
        ));
        assert!(matches!(
            &resolved.post_update_hooks[0],
            HookCommand::Exec { program, args, working_dir }
                if program == "docker-compose" && args == &["up", "-d"]
                    && *working_dir == Some("/opt/myapp".to_string())
        ));
    }

    #[test]
    fn resolve_hooks_structured_custom_commands() {
        let config = json!({
            "hooks": {
                "pre_update": {
                    "commands": ["echo 'Starting backup'", "backup.sh"],
                    "shell": "sh"
                },
                "post_update": {
                    "commands": ["systemctl restart myapp"],
                    "shell": "bash"
                }
            }
        });

        let resolved = resolve_hooks(&config, None);

        assert_eq!(resolved.pre_update_hooks.len(), 2);
        assert!(matches!(
            &resolved.pre_update_hooks[0],
            HookCommand::Shell { command, shell }
                if command == "echo 'Starting backup'" && *shell == uptrakit_internal_wire::HookShell::Sh
        ));
        assert_eq!(resolved.post_update_hooks.len(), 1);
        assert!(matches!(
            &resolved.post_update_hooks[0],
            HookCommand::Shell { command, shell }
                if command == "systemctl restart myapp" && *shell == uptrakit_internal_wire::HookShell::Bash
        ));
    }

    #[test]
    fn resolve_hooks_override_replaces_base_structured() {
        let base = json!({
            "hooks": {
                "pre_update": {
                    "predefined": {
                        "systemd_service": {
                            "service_name": "base-service",
                            "action": "stop"
                        }
                    }
                }
            }
        });
        let override_config = json!({
            "hooks": {
                "pre_update": {
                    "predefined": {
                        "systemd_service": {
                            "service_name": "override-service",
                            "action": "restart"
                        }
                    }
                }
            }
        });

        let resolved = resolve_hooks(&base, Some(&override_config));

        assert!(matches!(
            &resolved.pre_update_hooks[0],
            HookCommand::Exec { program, args, .. }
                if program == "systemctl" && args == &["restart", "override-service"]
        ));
    }

    #[test]
    fn resolve_hooks_fallback_to_legacy_format() {
        let config = json!({
            "pre_update_commands": ["legacy-pre"],
            "post_update_commands": ["legacy-post"]
        });

        let resolved = resolve_hooks(&config, None);

        assert_eq!(resolved.pre_update_hooks.len(), 1);
        assert!(matches!(
            &resolved.pre_update_hooks[0],
            HookCommand::Shell { command, .. } if command == "legacy-pre"
        ));
        assert_eq!(resolved.post_update_hooks.len(), 1);
        assert!(matches!(
            &resolved.post_update_hooks[0],
            HookCommand::Shell { command, .. } if command == "legacy-post"
        ));
    }

    #[test]
    fn resolve_hooks_empty_config() {
        let config = json!({});

        let resolved = resolve_hooks(&config, None);

        assert!(resolved.pre_update_hooks.is_empty());
        assert!(resolved.post_update_hooks.is_empty());
    }

    #[test]
    fn resolve_hooks_predefined_takes_precedence_over_commands() {
        let config = json!({
            "hooks": {
                "pre_update": {
                    "predefined": {
                        "systemd_service": {
                            "service_name": "myapp",
                            "action": "stop"
                        }
                    },
                    "commands": ["should-be-ignored"]
                }
            }
        });

        let resolved = resolve_hooks(&config, None);

        assert_eq!(resolved.pre_update_hooks.len(), 1);
        assert!(matches!(
            &resolved.pre_update_hooks[0],
            HookCommand::Exec { program, args, .. }
                if program == "systemctl" && args == &["stop", "myapp"]
        ));
    }

    // ── Legacy merge_hooks tests ─────────────────────────────────────────────

    #[test]
    fn merge_hooks_base_only() {
        let base = json!({
            "pre_update_commands": ["systemctl stop app"],
            "post_update_commands": ["systemctl start app"]
        });

        let (pre, post) = merge_hooks(&base, None);

        assert_eq!(pre, vec!["systemctl stop app"]);
        assert_eq!(post, vec!["systemctl start app"]);
    }

    #[test]
    fn merge_hooks_override_replaces_base() {
        let base = json!({
            "pre_update_commands": ["base-pre-1", "base-pre-2"],
            "post_update_commands": ["base-post"]
        });
        let override_config = json!({
            "pre_update_commands": ["override-pre"],
            "post_update_commands": ["override-post-1", "override-post-2"]
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert_eq!(pre, vec!["override-pre"]);
        assert_eq!(post, vec!["override-post-1", "override-post-2"]);
    }

    #[test]
    fn merge_hooks_partial_override() {
        let base = json!({
            "pre_update_commands": ["base-pre"],
            "post_update_commands": ["base-post"]
        });
        let override_config = json!({
            "pre_update_commands": ["override-pre"]
            // post_update_commands not present - should fall back to base
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert_eq!(pre, vec!["override-pre"]);
        assert_eq!(post, vec!["base-post"]);
    }

    #[test]
    fn merge_hooks_override_clears_with_empty_array() {
        let base = json!({
            "pre_update_commands": ["base-pre"],
            "post_update_commands": ["base-post"]
        });
        let override_config = json!({
            "pre_update_commands": [],
            "post_update_commands": []
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert!(pre.is_empty());
        assert!(post.is_empty());
    }

    #[test]
    fn merge_hooks_empty_base_and_override() {
        let base = json!({});
        let override_config = json!({});

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert!(pre.is_empty());
        assert!(post.is_empty());
    }

    #[test]
    fn merge_hooks_no_hooks_in_base() {
        let base = json!({
            "tag_strip_prefix": "v"
        });

        let (pre, post) = merge_hooks(&base, None);

        assert!(pre.is_empty());
        assert!(post.is_empty());
    }

    #[test]
    fn merge_hooks_only_pre_in_override() {
        let base = json!({});
        let override_config = json!({
            "pre_update_commands": ["pre-1", "pre-2"]
        });

        let (pre, post) = merge_hooks(&base, Some(&override_config));

        assert_eq!(pre, vec!["pre-1", "pre-2"]);
        assert!(post.is_empty());
    }

    #[test]
    fn merge_config_basic() {
        let base = json!({
            "tag_strip_prefix": "v",
            "include_prereleases": false
        });
        let override_config = json!({
            "tag_strip_prefix": "release-",
            "asset_patterns": [".*\\.tar\\.gz$"]
        });

        let merged = merge_config(&base, Some(&override_config));

        assert_eq!(merged["tag_strip_prefix"], "release-");
        assert_eq!(merged["include_prereleases"], false);
        assert_eq!(merged["asset_patterns"][0], ".*\\.tar\\.gz$");
    }

    #[test]
    fn merge_config_no_override() {
        let base = json!({
            "tag_strip_prefix": "v",
            "include_prereleases": false
        });

        let merged = merge_config(&base, None);

        assert_eq!(merged, base);
    }

    #[test]
    fn merge_config_empty_override() {
        let base = json!({
            "tag_strip_prefix": "v",
            "include_prereleases": false
        });
        let override_config = json!({});

        let merged = merge_config(&base, Some(&override_config));

        assert_eq!(merged, base);
    }

    // ── resolve_effective_config tests ────────────────────────────────────

    #[test]
    fn resolve_effective_config_all_three_layers() {
        let type_settings = json!({"discovery_filter": "all", "timeout": 30});
        let profile_config = json!({"auth_token": "secret", "timeout": 60});
        let assignment_config = json!({"include_prereleases": true, "timeout": 90});

        let result = resolve_effective_config(
            Some(&type_settings),
            Some(&profile_config),
            Some(&assignment_config),
        );

        assert_eq!(result["discovery_filter"], "all");
        assert_eq!(result["auth_token"], "secret");
        assert_eq!(result["include_prereleases"], true);
        assert_eq!(result["timeout"], 90);
    }

    #[test]
    fn resolve_effective_config_type_settings_only() {
        let type_settings = json!({"discovery_filter": "all"});

        let result = resolve_effective_config(Some(&type_settings), None, None);

        assert_eq!(result["discovery_filter"], "all");
    }

    #[test]
    fn resolve_effective_config_profile_only() {
        let profile = json!({"auth_token": "secret"});

        let result = resolve_effective_config(None, Some(&profile), None);

        assert_eq!(result["auth_token"], "secret");
    }

    #[test]
    fn resolve_effective_config_assignment_only() {
        let assignment = json!({"include_prereleases": true});

        let result = resolve_effective_config(None, None, Some(&assignment));

        assert_eq!(result["include_prereleases"], true);
    }

    #[test]
    fn resolve_effective_config_all_none() {
        let result = resolve_effective_config(None, None, None);
        assert_eq!(result, json!({}));
    }

    #[test]
    fn resolve_effective_config_later_layers_override_earlier() {
        let ts = json!({"key": "from_type_settings"});
        let pc = json!({"key": "from_profile"});
        let ac = json!({"key": "from_assignment"});

        // Profile overrides type settings.
        let r1 = resolve_effective_config(Some(&ts), Some(&pc), None);
        assert_eq!(r1["key"], "from_profile");

        // Assignment overrides profile.
        let r2 = resolve_effective_config(None, Some(&pc), Some(&ac));
        assert_eq!(r2["key"], "from_assignment");

        // Assignment overrides both.
        let r3 = resolve_effective_config(Some(&ts), Some(&pc), Some(&ac));
        assert_eq!(r3["key"], "from_assignment");
    }

    #[test]
    fn resolve_effective_config_non_object_layers_ignored() {
        let ts = json!("not an object");
        let result = resolve_effective_config(Some(&ts), None, None);
        assert_eq!(result, json!({}));
    }
}
