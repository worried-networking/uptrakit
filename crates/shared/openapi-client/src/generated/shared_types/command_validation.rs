// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Command validation utilities for plugin configuration.
//!
//! Provides length and count limits for shell commands in plugin configs,
//! update hooks, and custom hook commands. Placed in `shared-types` because
//! plugin crates need access for their `validate()` methods and `shared-types`
//! is already a dependency of every plugin.
/// Maximum length of a single shell command string (8 KiB).
///
/// Generous for legitimate scripts while preventing megabyte-sized payloads
/// from being stored and later executed on managed hosts.
pub const MAX_COMMAND_LENGTH: usize = 8192;
/// Maximum number of custom hook commands per phase (pre_update or post_update).
pub const MAX_HOOK_COMMANDS_PER_PHASE: usize = 20;
/// Validate that a command string does not exceed [`MAX_COMMAND_LENGTH`].
///
/// Returns `Ok(())` if the command is within the limit, or `Err` with a
/// human-readable message identifying the field that failed validation.
pub fn validate_command_length(command: &str, field_name: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err(format!("{field_name} must not be empty"));
    }
    if command.len() > MAX_COMMAND_LENGTH {
        return Err(format!(
            "{field_name} exceeds maximum length of {MAX_COMMAND_LENGTH} bytes ({} bytes)",
            command.len()
        ));
    }
    Ok(())
}
