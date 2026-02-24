//! Sudoers management helpers for the SSH agent.
//!
//! This module handles sudo detection, sudoers file generation, and writing
//! the sudoers drop-in file on remote hosts. It is used by both the bootstrap
//! command and the `update-sudoers` command.

use rootcause::prelude::*;
use uptrakit_command::shell_escape;

use crate::error::{Error, Result};
use crate::ssh_transport::SshSession;

// ── Detection ──────────────────────────────────────────────────────────

/// Detect whether the current SSH user is root by running `id -u`.
///
/// Returns `false` on any error so callers can proceed conservatively.
pub async fn detect_is_root(session: &SshSession) -> Result<bool> {
    let result = session.exec_command("id -u").await?;
    Ok(result.exit_code == 0 && result.stdout.trim() == "0")
}

/// Detect whether passwordless sudo is available via `sudo -n true`.
///
/// Returns `false` on any error or non-zero exit. Only meaningful when
/// [`detect_is_root`] returned `false`.
pub async fn detect_sudo_available(session: &SshSession) -> Result<bool> {
    let result = session.exec_command("sudo -n true").await?;
    Ok(result.exit_code == 0)
}

// ── Sudoers content ────────────────────────────────────────────────────

/// A command entry with its resolved absolute path on the remote host.
pub struct ResolvedSudoCommand {
    /// Absolute path of the command on the remote host (e.g. `/usr/bin/apt-get`).
    pub command_path: String,
    /// Human-readable explanation, shown as a sudoers comment.
    pub explanation: String,
}

/// Describes what to write to the sudoers file.
pub enum SudoersContent {
    /// Write `NOPASSWD: ALL` — maximum permissions, legacy fallback.
    AllCommands,
    /// Write one entry per resolved command — minimal, specific permissions.
    SpecificCommands(Vec<ResolvedSudoCommand>),
}

/// Resolve a command name to its absolute path on the remote host via
/// `command -v <name>`.
///
/// Returns `None` if the command is not found or the session fails.
pub async fn resolve_command_path(
    session: &SshSession,
    command: &str,
) -> Result<Option<String>> {
    let escaped = shell_escape(command);
    let cmd = format!("command -v {escaped}");
    let result = session.exec_command(&cmd).await?;
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
/// # Regenerate: uptrakit-agent-ssh host update-sudoers <host>
/// alice ALL=(root) NOPASSWD: ALL
/// ```
///
/// For `SpecificCommands`:
/// ```text
/// # Managed by Uptrakit - DO NOT EDIT MANUALLY
/// # Regenerate: uptrakit-agent-ssh host update-sudoers <host>
/// # apt-get: Package installation and index refresh require root privileges
/// alice ALL=(root) NOPASSWD: /usr/bin/apt-get
/// ```
pub fn generate_sudoers_content(username: &str, content: &SudoersContent) -> String {
    let mut out = String::new();
    out.push_str("# Managed by Uptrakit - DO NOT EDIT MANUALLY\n");
    out.push_str("# Regenerate: uptrakit-agent-ssh host update-sudoers <host>\n");

    match content {
        SudoersContent::AllCommands => {
            out.push_str(&format!("{username} ALL=(root) NOPASSWD: ALL\n"));
        }
        SudoersContent::SpecificCommands(entries) => {
            for entry in entries {
                out.push_str(&format!("# {}: {}\n", entry.command_path, entry.explanation));
                out.push_str(&format!(
                    "{username} ALL=(root) NOPASSWD: {}\n",
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
    session: &SshSession,
    username: &str,
    content: &SudoersContent,
    privileged: bool,
) -> Result<()> {
    let sudoers_text = generate_sudoers_content(username, content);
    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{username}");
    let escaped_file = shell_escape(&sudoers_file);
    let escaped_content = shell_escape(&sudoers_text);
    let sudo_prefix = if privileged { "sudo " } else { "" };

    // Write the file and chmod 440.
    let write_cmd = format!(
        "echo {escaped_content} | {sudo_prefix}tee {escaped_file} > /dev/null && \
         {sudo_prefix}chmod 440 {escaped_file}"
    );
    let write_result = session.exec_command(&write_cmd).await?;
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
    let validate_result = session.exec_command(&validate_cmd).await?;
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
        assert!(text.contains("Regenerate: uptrakit-agent-ssh host update-sudoers"));
        assert!(text.contains("alice ALL=(root) NOPASSWD: ALL"));
        assert!(!text.contains("/usr/bin/"));
    }

    #[test]
    fn generate_sudoers_specific_commands() {
        let content = SudoersContent::SpecificCommands(vec![
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get".to_string(),
                explanation: "Package installation requires root".to_string(),
            },
        ]);
        let text = generate_sudoers_content("bob", &content);

        assert!(text.contains("# Managed by Uptrakit"));
        assert!(text.contains("# /usr/bin/apt-get: Package installation requires root"));
        assert!(text.contains("bob ALL=(root) NOPASSWD: /usr/bin/apt-get"));
        assert!(!text.contains("NOPASSWD: ALL\n"));
    }

    #[test]
    fn generate_sudoers_multiple_specific_commands() {
        let content = SudoersContent::SpecificCommands(vec![
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get".to_string(),
                explanation: "Package management".to_string(),
            },
            ResolvedSudoCommand {
                command_path: "/usr/sbin/service".to_string(),
                explanation: "Service management".to_string(),
            },
        ]);
        let text = generate_sudoers_content("deploy", &content);

        assert!(text.contains("deploy ALL=(root) NOPASSWD: /usr/bin/apt-get"));
        assert!(text.contains("deploy ALL=(root) NOPASSWD: /usr/sbin/service"));
    }

    #[test]
    fn generate_sudoers_header_present_in_all_variants() {
        let all = generate_sudoers_content("u", &SudoersContent::AllCommands);
        let specific = generate_sudoers_content(
            "u",
            &SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
                command_path: "/bin/true".to_string(),
                explanation: "test".to_string(),
            }]),
        );

        for text in [&all, &specific] {
            assert!(
                text.contains("# Managed by Uptrakit - DO NOT EDIT MANUALLY"),
                "header missing in: {text}"
            );
        }
    }
}
