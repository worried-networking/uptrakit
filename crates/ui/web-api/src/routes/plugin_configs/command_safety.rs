// ── Security audit helpers ────────────────────────────────────────────────────

/// Detected dangerous pattern in a command-bearing field.
#[derive(Debug)]
pub(super) struct DangerousPatternMatch {
    /// Display-friendly field name (e.g. `"version_command"`, `"hooks.pre_update.commands[0]"`).
    pub(super) field: String,
    /// Short description of the detected pattern.
    pub(super) description: &'static str,
}

/// Scan all command-bearing fields in a plugin config for dangerous patterns.
///
/// Returns a list of matches. An empty list means no dangerous patterns were found.
pub(super) fn collect_dangerous_patterns(config: &serde_json::Value) -> Vec<DangerousPatternMatch> {
    let obj = match config.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut results = Vec::new();

    // Check top-level command string fields.
    for &(field_name, display_name) in &[
        ("version_command", "version_command"),
        ("update_command", "update_command"),
        ("post_pull_command", "post_pull_command"),
    ] {
        if let Some(val) = obj.get(field_name).and_then(|v| v.as_str()) {
            let patterns =
                uptrakit_web_api_types::command_validation::detect_dangerous_patterns(val);
            for (_, desc) in patterns {
                results.push(DangerousPatternMatch {
                    field: display_name.to_string(),
                    description: desc,
                });
            }
        }
    }

    // Check structured hook commands.
    if let Some(hooks) = obj.get("hooks").and_then(|v| v.as_object()) {
        for phase in ["pre_update", "post_update"] {
            if let Some(hook) = hooks.get(phase).and_then(|v| v.as_object())
                && let Some(arr) = hook.get("commands").and_then(|v| v.as_array())
            {
                for (i, cmd) in arr.iter().enumerate() {
                    if let Some(cmd_str) = cmd.as_str() {
                        let patterns =
                            uptrakit_web_api_types::command_validation::detect_dangerous_patterns(
                                cmd_str,
                            );
                        for (_, desc) in patterns {
                            results.push(DangerousPatternMatch {
                                field: format!("hooks.{phase}.commands[{i}]"),
                                description: desc,
                            });
                        }
                    }
                }
            }
        }
    }

    results
}

/// Format a rejection error message from a list of dangerous pattern matches.
#[expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]
pub(super) fn format_dangerous_pattern_rejection(matches: &[DangerousPatternMatch]) -> String {
    use std::fmt::Write;
    let mut msg = String::from(
        "Plugin config contains dangerous command patterns and was rejected by server policy",
    );
    for m in matches {
        write!(msg, "; {}: {}", m.field, m.description).expect("write to String never fails");
    }
    msg
}

/// Known field names that carry executable commands in plugin configs.
const COMMAND_FIELD_NAMES: &[&str] = &[
    "version_command",
    "update_command",
    "post_pull_command",
    "pre_update_commands",
    "post_update_commands",
];

/// Detect command-bearing field names present in a plugin config value.
///
/// Returns a list of field names that carry executable commands (e.g.
/// `version_command`, `update_command`, `post_pull_command`, hook `commands`).
/// Used for security audit logging to highlight configs that grant effective RCE
/// on managed hosts.
pub(super) fn detect_command_fields(config: &serde_json::Value) -> Vec<&'static str> {
    let obj = match config.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut fields = Vec::new();

    for &name in COMMAND_FIELD_NAMES {
        if let Some(val) = obj.get(name) {
            // Skip null/empty values.
            if !val.is_null() {
                fields.push(name);
            }
        }
    }

    // Structured hooks: hooks.pre_update.commands, hooks.post_update.commands
    if let Some(hooks) = obj.get("hooks").and_then(|v| v.as_object()) {
        for phase in ["pre_update", "post_update"] {
            if let Some(hook) = hooks.get(phase).and_then(|v| v.as_object())
                && let Some(arr) = hook.get("commands").and_then(|v| v.as_array())
                && !arr.is_empty()
            {
                if phase == "pre_update" {
                    fields.push("hooks.pre_update.commands");
                } else {
                    fields.push("hooks.post_update.commands");
                }
            }
        }
    }

    fields
}
