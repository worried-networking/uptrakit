//! Sudoers management helpers for the SSH agent.
//!
//! This module handles sudo detection, sudoers file generation, and writing
//! the sudoers drop-in file on remote hosts. It is used by both the bootstrap
//! command and the `sync` command.

use rootcause::prelude::*;
use uptrakit_command::{RemoteExecutor, shell_escape};
use uptrakit_plugin_infrastructure_registry::SudoHelperScript;

use crate::error::{Error, Result};

// ── Detection ──────────────────────────────────────────────────────────

/// Detect whether the current SSH user is root by running `id -u`.
///
/// Returns `false` on any error so callers can proceed conservatively.
pub async fn detect_is_root(executor: &dyn RemoteExecutor) -> Result<bool> {
    let result = executor.exec_command("id -u").await.context_to::<Error>()?;
    Ok(result.exit_code == 0 && result.stdout.trim() == "0")
}

/// Detect whether passwordless sudo is available via `sudo -n true`.
///
/// Returns `false` on any error or non-zero exit. Only meaningful when
/// [`detect_is_root`] returned `false`.
pub async fn detect_sudo_available(executor: &dyn RemoteExecutor) -> Result<bool> {
    let result = executor
        .exec_command("sudo -n true")
        .await
        .context_to::<Error>()?;
    Ok(result.exit_code == 0)
}

// ── Sudoers content ────────────────────────────────────────────────────

/// A command entry with its resolved absolute path on the remote host.
pub struct ResolvedSudoCommand {
    /// Absolute path of the command on the remote host (e.g. `/usr/bin/apt-get`).
    pub command_path: String,
    /// Human-readable explanation, shown as a sudoers comment.
    pub explanation: String,
    /// When `true`, the sudoers entry includes `SETENV:` so the agent can pass
    /// inline `NAME=VALUE` env var assignments before the program name.
    /// Propagated from [`SudoCommandEntry::needs_setenv`].
    pub needs_setenv: bool,
}

/// Describes what to write to the sudoers file.
pub enum SudoersContent {
    /// Write `NOPASSWD: ALL` — maximum permissions, legacy fallback.
    AllCommands,
    /// Write one entry per resolved command — minimal, specific permissions.
    SpecificCommands(Vec<ResolvedSudoCommand>),
}

/// Install a helper script on the remote host at `helper.install_path`.
///
/// The script is written with mode `0755`. If a file already exists at the
/// path it is overwritten — the content is deterministic so idempotency is
/// preserved.
///
/// `privileged` controls whether write commands are prefixed with `sudo`:
/// - `true` when the auth user is non-root and has passwordless sudo.
/// - `false` when the auth user is root.
pub async fn install_helper_script(
    executor: &dyn RemoteExecutor,
    helper: &SudoHelperScript,
    privileged: bool,
) -> Result<()> {
    let sudo_prefix = if privileged { "sudo " } else { "" };
    let escaped_path = shell_escape(helper.install_path);
    let escaped_content = shell_escape(helper.content);

    // Write the script and make it executable.  `printf '%s'` avoids the
    // backslash interpretation that some `echo` implementations apply.
    let cmd = format!(
        "printf '%s' {escaped_content} | {sudo_prefix}tee {escaped_path} > /dev/null && \
         {sudo_prefix}chmod 755 {escaped_path}"
    );
    let result = executor.exec_command(&cmd).await.context_to::<Error>()?;
    if result.exit_code != 0 {
        bail!(Error::SshCommand(format!(
            "failed to install helper script '{}': {}",
            helper.install_path,
            result.stderr.trim()
        )));
    }

    tracing::debug!(path = helper.install_path, "installed sudo helper script");
    Ok(())
}

/// Resolve a command name to its absolute path on the remote host via
/// `command -v <name>`.
///
/// Returns `None` if the command is not found or the session fails.
pub async fn resolve_command_path(
    executor: &dyn RemoteExecutor,
    command: &str,
) -> Result<Option<String>> {
    let escaped = shell_escape(command);
    let cmd = format!("command -v {escaped}");
    let result = executor.exec_command(&cmd).await.context_to::<Error>()?;
    if result.exit_code != 0 {
        return Ok(None);
    }
    let path = result.stdout.trim().to_string();
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(path))
    }
}

/// Generate the text content of a sudoers drop-in file for `username`.
///
/// # Format
///
/// For `AllCommands`:
/// ```text
/// # Managed by Uptrakit - DO NOT EDIT MANUALLY
/// # Regenerate: uptrakit-agent-ssh host sync <host>
/// alice ALL=(root) NOPASSWD: ALL
/// ```
///
/// For `SpecificCommands`:
/// ```text
/// # Managed by Uptrakit - DO NOT EDIT MANUALLY
/// # Regenerate: uptrakit-agent-ssh host sync <host>
/// # /usr/bin/apt-get: Package installation and index refresh require root privileges
/// alice ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get
/// ```
///
/// ## `SETENV:` tag
///
/// The `SETENV:` tag is included on a per-entry basis, controlled by
/// [`ResolvedSudoCommand::needs_setenv`] (propagated from
/// [`SudoCommandEntry::needs_setenv`]). It allows the agent to pass env vars
/// as inline `NAME=VALUE` assignments before the program name when calling
/// `sudo` (e.g. `sudo DEBIAN_FRONTEND=noninteractive apt-get update`).
/// Without it, sudo rejects those assignments.
///
/// `SETENV:` does **not** override `env_reset` — sudo's built-in `env_delete`
/// list still strips dangerous vars like `LD_PRELOAD` before they reach the
/// privileged process. Only set `needs_setenv` when the plugin invokes that
/// command with [`CommandSpec::with_env`] combined with `.privileged()`.
pub fn generate_sudoers_content(username: &str, content: &SudoersContent) -> String {
    let mut out = String::new();
    out.push_str("# Managed by Uptrakit - DO NOT EDIT MANUALLY\n");
    out.push_str("# Regenerate: uptrakit-agent-ssh host sync <host>\n");

    match content {
        SudoersContent::AllCommands => {
            out.push_str(&format!("{username} ALL=(root) NOPASSWD: ALL\n"));
        }
        SudoersContent::SpecificCommands(entries) => {
            for entry in entries {
                out.push_str(&format!(
                    "# {}: {}\n",
                    entry.command_path, entry.explanation
                ));
                let setenv = if entry.needs_setenv { "SETENV: " } else { "" };
                out.push_str(&format!(
                    "{username} ALL=(root) NOPASSWD: {setenv}{}\n",
                    entry.command_path
                ));
            }
        }
    }

    out
}

/// Write a sudoers drop-in file for `username` to `/etc/sudoers.d/uptrakit-{username}`,
/// set permissions to 440, and validate with `visudo -cf`.
///
/// `privileged` controls whether write commands are prefixed with `sudo`:
/// - `true` when the auth user is non-root and has passwordless sudo.
/// - `false` when the auth user is root.
pub async fn write_sudoers_file(
    executor: &dyn RemoteExecutor,
    username: &str,
    content: &SudoersContent,
    privileged: bool,
) -> Result<()> {
    let sudoers_text = generate_sudoers_content(username, content);
    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{username}");
    let escaped_file = shell_escape(&sudoers_file);
    let escaped_content = shell_escape(&sudoers_text);
    let sudo_prefix = if privileged { "sudo " } else { "" };

    // Write the file and chmod 440.  Use `printf '%s'` rather than `echo`
    // so that (a) no extra trailing newline is appended (the generated
    // content already ends with '\n') and (b) backslash sequences are not
    // interpreted by the shell built-in `echo` on some platforms.
    let write_cmd = format!(
        "printf '%s' {escaped_content} | {sudo_prefix}tee {escaped_file} > /dev/null && \
         {sudo_prefix}chmod 440 {escaped_file}"
    );
    let write_result = executor
        .exec_command(&write_cmd)
        .await
        .context_to::<Error>()?;
    if write_result.exit_code != 0 {
        bail!(Error::SshCommand(format!(
            "failed to write sudoers file '{}': {}",
            sudoers_file,
            write_result.stderr.trim()
        )));
    }

    // Validate with visudo.
    let validate_cmd = if privileged {
        format!("sudo visudo -cf {escaped_file}")
    } else {
        format!("visudo -cf {escaped_file}")
    };
    let validate_result = executor
        .exec_command(&validate_cmd)
        .await
        .context_to::<Error>()?;
    if validate_result.exit_code != 0 {
        bail!(Error::SshCommand(format!(
            "sudoers validation failed (visudo -cf {}): {}",
            sudoers_file,
            validate_result.stderr.trim()
        )));
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── generate_sudoers_content ─────────────────────────────────────────

    #[test]
    fn generate_sudoers_all_commands() {
        let content = SudoersContent::AllCommands;
        let text = generate_sudoers_content("alice", &content);

        assert!(text.contains("# Managed by Uptrakit"));
        assert!(text.contains("Regenerate: uptrakit-agent-ssh host sync"));
        assert!(text.contains("alice ALL=(root) NOPASSWD: ALL"));
        assert!(!text.contains("/usr/bin/"));
    }

    #[test]
    fn generate_sudoers_specific_commands_with_setenv() {
        let content = SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
            command_path: "/usr/bin/apt-get".to_string(),
            explanation: "Package installation requires root".to_string(),
            needs_setenv: true,
        }]);
        let text = generate_sudoers_content("bob", &content);

        assert!(text.contains("# Managed by Uptrakit"));
        assert!(text.contains("# /usr/bin/apt-get: Package installation requires root"));
        // SETENV: is emitted when needs_setenv is true.
        assert!(text.contains("bob ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get"));
        assert!(!text.contains("NOPASSWD: ALL\n"));
    }

    #[test]
    fn generate_sudoers_specific_commands_without_setenv() {
        let content = SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
            command_path: "/usr/bin/npm".to_string(),
            explanation: "Global npm installation requires root".to_string(),
            needs_setenv: false,
        }]);
        let text = generate_sudoers_content("bob", &content);

        // SETENV: must NOT be present for commands that don't pass env vars.
        assert!(text.contains("bob ALL=(root) NOPASSWD: /usr/bin/npm"));
        assert!(!text.contains("SETENV:"));
    }

    #[test]
    fn generate_sudoers_mixed_setenv_and_no_setenv() {
        let content = SudoersContent::SpecificCommands(vec![
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get".to_string(),
                explanation: "Package management".to_string(),
                needs_setenv: true,
            },
            ResolvedSudoCommand {
                command_path: "/usr/bin/npm".to_string(),
                explanation: "npm management".to_string(),
                needs_setenv: false,
            },
        ]);
        let text = generate_sudoers_content("deploy", &content);

        assert!(text.contains("deploy ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get"));
        assert!(text.contains("deploy ALL=(root) NOPASSWD: /usr/bin/npm"));
        assert!(!text.contains("NOPASSWD: SETENV: /usr/bin/npm"));
    }

    #[test]
    fn generate_sudoers_specific_commands_no_setenv_in_all_commands() {
        // SETENV: only applies to per-command entries, not AllCommands (which grants ALL anyway).
        let text = generate_sudoers_content("alice", &SudoersContent::AllCommands);
        assert!(text.contains("alice ALL=(root) NOPASSWD: ALL"));
        assert!(!text.contains("SETENV:"));
    }

    #[test]
    fn generate_sudoers_header_present_in_all_variants() {
        let all = generate_sudoers_content("u", &SudoersContent::AllCommands);
        let specific = generate_sudoers_content(
            "u",
            &SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
                command_path: "/bin/true".to_string(),
                explanation: "test".to_string(),
                needs_setenv: false,
            }]),
        );

        for text in [&all, &specific] {
            assert!(
                text.contains("# Managed by Uptrakit - DO NOT EDIT MANUALLY"),
                "header missing in: {text}"
            );
        }
    }

    #[test]
    fn generate_sudoers_args_suffix_in_command_path() {
        // The command_path field already includes the args_suffix (appended
        // by bootstrap/sync). This test verifies the suffix is rendered verbatim.
        let content = SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
            command_path: "/usr/bin/apt-get update *".to_string(),
            explanation: "Package index refresh requires root privileges".to_string(),
            needs_setenv: true,
        }]);
        let text = generate_sudoers_content("alice", &content);

        assert!(
            text.contains("bob ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get update *")
                || text.contains("alice ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get update *"),
            "expected full sudoers line with args suffix: {text}"
        );
        assert!(text.contains("/usr/bin/apt-get update *"));
    }

    #[test]
    fn generate_sudoers_multiple_entries_same_binary() {
        // Multiple entries for the same binary (e.g. apt-get update * and
        // apt-get install *) each generate their own sudoers line.
        let content = SudoersContent::SpecificCommands(vec![
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get update *".to_string(),
                explanation: "Package index refresh requires root".to_string(),
                needs_setenv: true,
            },
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get install *".to_string(),
                explanation: "Package installation requires root".to_string(),
                needs_setenv: true,
            },
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get -o Dir::Etc::Preferences=/tmp/uptrakit-apt-batch.pref upgrade *"
                    .to_string(),
                explanation: "Batch upgrade requires root".to_string(),
                needs_setenv: true,
            },
        ]);
        let text = generate_sudoers_content("uptrakit", &content);

        assert!(text.contains("uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get update *"));
        assert!(text.contains("uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get install *"));
        assert!(text.contains(
            "uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get -o Dir::Etc::Preferences=/tmp/uptrakit-apt-batch.pref upgrade *"
        ));
        // Should have 3 sudoers lines (one per entry)
        let entry_count = text
            .lines()
            .filter(|l| l.starts_with("uptrakit ALL="))
            .count();
        assert_eq!(entry_count, 3, "expected 3 sudoers entries:\n{text}");
    }

    #[test]
    fn generate_sudoers_setenv_absent_on_restricted_entries() {
        // Restricted entries without needs_setenv must not have SETENV:.
        let content = SudoersContent::SpecificCommands(vec![
            ResolvedSudoCommand {
                command_path: "/usr/bin/pacman -Sy".to_string(),
                explanation: "Pacman sync".to_string(),
                needs_setenv: false,
            },
            ResolvedSudoCommand {
                command_path: "/usr/bin/pacman -S --noconfirm *".to_string(),
                explanation: "Pacman install".to_string(),
                needs_setenv: false,
            },
        ]);
        let text = generate_sudoers_content("deploy", &content);

        assert!(text.contains("deploy ALL=(root) NOPASSWD: /usr/bin/pacman -Sy"));
        assert!(text.contains("deploy ALL=(root) NOPASSWD: /usr/bin/pacman -S --noconfirm *"));
        assert!(!text.contains("SETENV:"));
    }
}
