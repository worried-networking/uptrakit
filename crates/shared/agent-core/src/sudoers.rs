//! Sudoers management helpers shared by every writer of the drop-in file.
//!
//! This module handles sudo detection, sudoers file generation, and writing the
//! sudoers drop-in file through a [`RemoteExecutor`], so it serves both writers:
//! the SSH host sync flow (`uptrakit-agent-ssh` bootstrap and `sync`) over an SSH
//! executor, and the local `bootstrap-host` subcommand over a local executor. See
//! `docs/security/sudoers-management.md`.

use rootcause::prelude::*;
use uptrakit_command::{RemoteExecutor, shell_escape};
use uptrakit_plugin_infrastructure_registry::SudoHelperScript;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors from host sudoers/helper provisioning.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SudoersError {
    /// A provisioning command failed on the host.
    #[error("sudoers provisioning command failed: {0}")]
    Command(String),
}

/// Result alias covering every function in this module.
pub type Result<T> = std::result::Result<T, rootcause::Report<SudoersError>>;

impl_report_conversion!(uptrakit_command::CommandError => SudoersError, |e| SudoersError::Command(e.to_string()));

// ── Detection ──────────────────────────────────────────────────────────

/// Detect whether the current SSH user is root by running `id -u`.
///
/// Returns `false` on any error so callers can proceed conservatively.
pub async fn detect_is_root(executor: &dyn RemoteExecutor) -> Result<bool> {
    let result = executor
        .exec_command("id -u")
        .await
        .context_to::<SudoersError>()?;
    Ok(result.exit_code == 0 && result.stdout.trim() == "0")
}

/// Detect whether the current SSH user has at least one passwordless sudo
/// entry via `sudo -n -l`.
///
/// Returns `false` on any error or non-zero exit. Only meaningful when
/// [`detect_is_root`] returned `false`.
pub async fn detect_sudo_available(executor: &dyn RemoteExecutor) -> Result<bool> {
    let result = executor
        .exec_command("sudo -n -l")
        .await
        .context_to::<SudoersError>()?;
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
    let result = executor
        .exec_command(&cmd)
        .await
        .context_to::<SudoersError>()?;
    if result.exit_code != 0 {
        bail!(SudoersError::Command(format!(
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
    let result = executor
        .exec_command(&cmd)
        .await
        .context_to::<SudoersError>()?;
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
/// # Regenerated by uptrakit: SSH host sync or 'bootstrap-host'
/// alice ALL=(root) NOPASSWD: ALL
/// ```
///
/// For `SpecificCommands`:
/// ```text
/// # Managed by Uptrakit - DO NOT EDIT MANUALLY
/// # Regenerated by uptrakit: SSH host sync or 'bootstrap-host'
/// # /usr/bin/apt-get: Install or upgrade APT packages
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
///
/// ## Command argument escaping
///
/// Plugin declarations and infrastructure sync results provide command specs in
/// normal command-token form (for example `apt-get -o KEY=value upgrade *`).
/// Before writing sudoers lines, this module escapes the subset of characters
/// that `sudoers(5)` treats specially inside command arguments: `,`, `:`, `=`,
/// `\`, and a leading `^`. Wildcard characters remain unescaped so existing
/// sudoers matching semantics are preserved, including wildcard tokens that
/// appear in the middle of the argument list.
fn escape_sudoers_arg_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for (idx, ch) in token.chars().enumerate() {
        let needs_escape = matches!(ch, ',' | ':' | '=' | '\\') || (idx == 0 && ch == '^');
        if needs_escape {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn render_sudoers_command_spec(command_spec: &str) -> String {
    command_spec
        .split_whitespace()
        .map(escape_sudoers_arg_token)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn generate_sudoers_content(username: &str, content: &SudoersContent) -> String {
    let mut out = String::new();
    out.push_str("# Managed by Uptrakit - DO NOT EDIT MANUALLY\n");
    out.push_str("# Regenerated by uptrakit: SSH host sync or 'bootstrap-host'\n");

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
                let rendered_command = render_sudoers_command_spec(&entry.command_path);
                out.push_str(&format!(
                    "{username} ALL=(root) NOPASSWD: {setenv}{}\n",
                    rendered_command
                ));
            }
        }
    }

    out
}

/// Ensure the managed user is a member of the `docker` group when Docker is present.
///
/// Steps:
/// 1. Run `getent group docker` (read-only, no sudo required) to detect Docker.
/// 2. If the group is absent, log a debug message and return `Ok(())` — non-fatal.
/// 3. Run `[sudo] usermod -aG docker <username>`.
/// 4. If `usermod` fails, emit a `tracing::warn!` with the stderr and return
///    `Ok(())` — also non-fatal (the host may already be a member, or the
///    command may be missing).
///
/// `privileged` mirrors the same flag used by [`write_sudoers_file`]:
/// `true` when the auth user is non-root and has passwordless sudo, `false`
/// when the auth user is root.
pub async fn ensure_docker_group_membership(
    executor: &dyn RemoteExecutor,
    username: &str,
    privileged: bool,
) -> Result<()> {
    let check = executor
        .exec_command("getent group docker")
        .await
        .context_to::<SudoersError>()?;
    if check.exit_code != 0 {
        tracing::debug!("docker group not found, skipping group membership configuration");
        return Ok(());
    }

    let sudo_prefix = if privileged { "sudo " } else { "" };
    let escaped_username = shell_escape(username);
    let cmd = format!("{sudo_prefix}usermod -aG docker {escaped_username}");
    let result = executor
        .exec_command(&cmd)
        .await
        .context_to::<SudoersError>()?;
    if result.exit_code != 0 {
        tracing::warn!(
            stderr = result.stderr.trim(),
            user = username,
            "failed to add user to docker group (non-fatal)"
        );
        return Ok(());
    }

    tracing::info!(user = username, "added user to docker group");
    Ok(())
}

/// Write the sudoers drop-in for `username`, validating before activation:
/// content goes to `<file>.tmp.<pid>`, is `chmod 0440`ed and checked with
/// `visudo -cf`, and only then moved into place. On any failure the temp file
/// is removed and an existing drop-in stays untouched. Idempotent.
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
    let sudoers_content = generate_sudoers_content(username, content);
    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{username}");
    // The `.` in the temp name is load-bearing: sudo's `#includedir` skips
    // filenames containing a dot, so an unvalidated temp file is inert even
    // while it sits in /etc/sudoers.d/. The pid suffix keeps concurrent
    // writers (SSH sync vs bootstrap-host) from clobbering each other's temp.
    let tmp_file = format!("{sudoers_file}.tmp.{}", std::process::id());
    let sudo_prefix = if privileged { "sudo " } else { "" };

    // `visudo` commonly lives in /usr/sbin, outside non-root PATHs.
    let visudo = resolve_command_path(executor, "visudo")
        .await?
        .unwrap_or_else(|| "/usr/sbin/visudo".to_string());

    let command = format!(
        "printf '%s' {} | {sudo_prefix}tee {} > /dev/null && {sudo_prefix}chmod 440 {} && {sudo_prefix}{} -cf {} && {sudo_prefix}mv {} {} || {{ {sudo_prefix}rm -f {}; echo 'uptrakit sudoers install failed (write/validate/activate pipeline)' >&2; exit 1; }}",
        shell_escape(&sudoers_content),
        shell_escape(&tmp_file),
        shell_escape(&tmp_file),
        shell_escape(&visudo),
        shell_escape(&tmp_file),
        shell_escape(&tmp_file),
        shell_escape(&sudoers_file),
        shell_escape(&tmp_file),
    );
    let result = executor
        .exec_command(&command)
        .await
        .context_to::<SudoersError>()?;
    if result.exit_code != 0 {
        bail!(SudoersError::Command(format!(
            "failed to install sudoers file '{sudoers_file}': {}",
            result.stderr.trim()
        )));
    }
    tracing::debug!(file = %sudoers_file, "sudoers file installed and validated");
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test code: `assert!(r.is_ok())` is idiomatic in tests where the success value is not inspected"
    )]

    use super::*;

    // ── generate_sudoers_content ─────────────────────────────────────────

    #[test]
    fn generate_sudoers_all_commands() {
        let content = SudoersContent::AllCommands;
        let text = generate_sudoers_content("alice", &content);

        assert!(text.contains("# Managed by Uptrakit"));
        assert!(text.contains("Regenerated by uptrakit: SSH host sync or 'bootstrap-host'"));
        assert!(text.contains("alice ALL=(root) NOPASSWD: ALL"));
        assert!(!text.contains("/usr/bin/"));
    }

    #[test]
    fn generate_sudoers_specific_commands_with_setenv() {
        let content = SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
            command_path: "/usr/bin/apt-get".to_string(),
            explanation: "Install or upgrade an APT package".to_string(),
            needs_setenv: true,
        }]);
        let text = generate_sudoers_content("bob", &content);

        assert!(text.contains("# Managed by Uptrakit"));
        assert!(text.contains("# /usr/bin/apt-get: Install or upgrade an APT package"));
        // SETENV: is emitted when needs_setenv is true.
        assert!(text.contains("bob ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get"));
        assert!(!text.contains("NOPASSWD: ALL\n"));
    }

    #[test]
    fn generate_sudoers_specific_commands_without_setenv() {
        let content = SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
            command_path: "/usr/bin/npm".to_string(),
            explanation: "Install or upgrade a global npm package".to_string(),
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
            explanation: "Refresh the APT package index".to_string(),
            needs_setenv: true,
        }]);
        let text = generate_sudoers_content("alice", &content);

        assert!(
            text.contains("alice ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get update *"),
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
                explanation: "Refresh the APT package index".to_string(),
                needs_setenv: true,
            },
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get install *".to_string(),
                explanation: "Install or upgrade an APT package".to_string(),
                needs_setenv: true,
            },
            ResolvedSudoCommand {
                command_path: "/usr/bin/apt-get -o Dir::Etc::Preferences=/tmp/uptrakit-apt-batch.pref upgrade *"
                    .to_string(),
                explanation: "Upgrade packages using a pinned preferences file (batch update)".to_string(),
                needs_setenv: true,
            },
        ]);
        let text = generate_sudoers_content("uptrakit", &content);

        assert!(text.contains("uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get update *"));
        assert!(text.contains("uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get install *"));
        assert!(text.contains(
            "uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get -o Dir\\:\\:Etc\\:\\:Preferences\\=/tmp/uptrakit-apt-batch.pref upgrade *"
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

    #[test]
    fn escape_sudoers_arg_token_escapes_sudoers_special_chars() {
        assert_eq!(
            escape_sudoers_arg_token("Dir::Etc::Preferences=/tmp/uptrakit-apt-batch.pref"),
            "Dir\\:\\:Etc\\:\\:Preferences\\=/tmp/uptrakit-apt-batch.pref"
        );
        assert_eq!(escape_sudoers_arg_token("^caret"), "\\^caret");
        assert_eq!(escape_sudoers_arg_token(r"foo\bar"), r"foo\\bar");
    }

    #[test]
    fn escape_sudoers_arg_token_leaves_wildcards_untouched() {
        assert_eq!(escape_sudoers_arg_token("*"), "*");
        assert_eq!(escape_sudoers_arg_token("stop"), "stop");
        assert_eq!(escape_sudoers_arg_token("/etc/*.conf"), "/etc/*.conf");
    }

    #[test]
    fn render_sudoers_command_spec_preserves_wildcards_in_middle() {
        assert_eq!(
            render_sudoers_command_spec("/usr/sbin/qm guest cmd * network-get-interfaces"),
            "/usr/sbin/qm guest cmd * network-get-interfaces"
        );
        assert_eq!(
            render_sudoers_command_spec("/usr/sbin/pct exec *"),
            "/usr/sbin/pct exec *"
        );
    }

    #[test]
    fn generate_sudoers_specific_commands_escapes_literal_tokens() {
        let content = SudoersContent::SpecificCommands(vec![ResolvedSudoCommand {
            command_path:
                "/usr/bin/apt-get -o Dir::Etc::Preferences=/tmp/uptrakit-apt-batch.pref upgrade *"
                    .to_string(),
            explanation: "Upgrade packages using a pinned preferences file (batch update)"
                .to_string(),
            needs_setenv: true,
        }]);
        let text = generate_sudoers_content("uptrakit", &content);

        assert!(text.contains(
            "uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get -o Dir\\:\\:Etc\\:\\:Preferences\\=/tmp/uptrakit-apt-batch.pref upgrade *"
        ));
    }

    // ── ensure_docker_group_membership ──────────────────────────────────

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use uptrakit_command::RemoteCommandResult;

    /// Scripted mock executor: returns pre-programmed [`RemoteCommandResult`]
    /// values in FIFO order for each `exec_command` call.
    struct ScriptedRemoteExecutor {
        results: Mutex<VecDeque<RemoteCommandResult>>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedRemoteExecutor {
        fn new(results: impl IntoIterator<Item = RemoteCommandResult>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn recorded_calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl RemoteExecutor for ScriptedRemoteExecutor {
        async fn exec_command(
            &self,
            command: &str,
        ) -> uptrakit_command::Result<RemoteCommandResult> {
            self.calls.lock().unwrap().push(command.to_string());
            let result = self
                .results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(RemoteCommandResult {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                });
            Ok(result)
        }
    }

    fn ok_result() -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn err_result(stderr: &str) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code: 1,
        }
    }

    fn stdout_result(stdout: &str) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    // ── write_sudoers_file ──────────────────────────────────────────────

    #[tokio::test]
    async fn write_sudoers_validates_before_activation() {
        let executor = ScriptedRemoteExecutor::new(vec![
            stdout_result("/usr/sbin/visudo\n"), // command -v visudo
            ok_result(),                         // combined write+validate+move
        ]);
        write_sudoers_file(&executor, "uptrakit", &SudoersContent::AllCommands, false)
            .await
            .expect("write");
        let calls = executor.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].contains("command -v"));
        let cmd = &calls[1];
        // Ordering inside the single compound command: write → chmod → validate → activate.
        let tee = cmd.find("tee ").expect("tee present");
        let chmod = cmd.find("chmod 440").expect("chmod present");
        let visudo = cmd.find("visudo").expect("visudo present");
        let mv = cmd.find(" mv ").expect("mv present");
        assert!(
            tee < chmod && chmod < visudo && visudo < mv,
            "order write→chmod→validate→activate: {cmd}"
        );
        assert!(cmd.contains(".tmp"), "writes to temp file: {cmd}");
        assert!(cmd.contains("rm -f"), "cleans up on failure: {cmd}");
        // NOTE: cannot assert `!cmd.contains("sudo ")` — "visudo " matches that substring.
        assert!(
            !cmd.contains("sudo tee") && !cmd.contains("sudo mv"),
            "non-privileged has no sudo prefix: {cmd}"
        );
    }

    #[tokio::test]
    async fn write_sudoers_privileged_prefixes_sudo() {
        let executor =
            ScriptedRemoteExecutor::new(vec![stdout_result("/usr/sbin/visudo\n"), ok_result()]);
        write_sudoers_file(&executor, "uptrakit", &SudoersContent::AllCommands, true)
            .await
            .expect("write");
        let cmd = &executor.recorded_calls()[1];
        assert!(cmd.contains("sudo tee"), "{cmd}");
        assert!(cmd.contains("sudo mv"), "{cmd}");
    }

    #[tokio::test]
    async fn write_sudoers_falls_back_to_default_visudo_path() {
        let executor = ScriptedRemoteExecutor::new(vec![
            err_result("not found"), // command -v visudo fails
            ok_result(),
        ]);
        write_sudoers_file(&executor, "uptrakit", &SudoersContent::AllCommands, false)
            .await
            .expect("write");
        let cmd = &executor.recorded_calls()[1];
        assert!(cmd.contains("/usr/sbin/visudo"), "{cmd}");
    }

    #[tokio::test]
    async fn write_sudoers_failure_returns_error() {
        let executor = ScriptedRemoteExecutor::new(vec![
            stdout_result("/usr/sbin/visudo\n"),
            err_result("syntax error near line 3"),
        ]);
        let err = write_sudoers_file(&executor, "uptrakit", &SudoersContent::AllCommands, false)
            .await
            .expect_err("must fail");
        assert!(format!("{err:?}").contains("syntax error"), "{err:?}");
    }

    #[tokio::test]
    async fn detect_sudo_available_uses_sudo_list_probe() {
        let exec = ScriptedRemoteExecutor::new([ok_result()]);

        let result = detect_sudo_available(&exec).await;

        assert!(result.expect("sudo detection should succeed"));
        let calls = exec.recorded_calls();
        assert_eq!(calls, vec!["sudo -n -l".to_string()]);
    }

    #[tokio::test]
    async fn docker_group_absent_returns_ok_no_usermod() {
        // getent exits non-zero → group absent
        let exec = ScriptedRemoteExecutor::new([err_result("docker: no such group")]);

        let result = ensure_docker_group_membership(&exec, "uptrakit", false).await;

        assert!(result.is_ok());
        let calls = exec.recorded_calls();
        assert_eq!(calls.len(), 1, "only getent should be called");
        assert!(calls[0].contains("getent group docker"));
    }

    #[tokio::test]
    async fn docker_group_present_usermod_succeeds() {
        // getent succeeds, usermod succeeds
        let exec = ScriptedRemoteExecutor::new([ok_result(), ok_result()]);

        let result = ensure_docker_group_membership(&exec, "uptrakit", false).await;

        assert!(result.is_ok());
        let calls = exec.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].contains("getent group docker"));
        assert!(calls[1].contains("usermod -aG docker"));
        assert!(calls[1].contains("uptrakit"));
        // non-privileged: no sudo prefix
        assert!(!calls[1].starts_with("sudo "));
    }

    #[tokio::test]
    async fn docker_group_present_usermod_succeeds_privileged() {
        // getent succeeds, sudo usermod succeeds
        let exec = ScriptedRemoteExecutor::new([ok_result(), ok_result()]);

        let result = ensure_docker_group_membership(&exec, "uptrakit", true).await;

        assert!(result.is_ok());
        let calls = exec.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[1].starts_with("sudo "),
            "expected sudo prefix: {}",
            calls[1]
        );
        assert!(calls[1].contains("usermod -aG docker"));
    }

    #[tokio::test]
    async fn docker_group_present_usermod_fails_returns_ok() {
        // getent succeeds, usermod fails — must still return Ok (warn-only)
        let exec = ScriptedRemoteExecutor::new([
            ok_result(),
            err_result("usermod: group 'docker' does not exist"),
        ]);

        let result = ensure_docker_group_membership(&exec, "uptrakit", false).await;

        assert!(result.is_ok(), "usermod failure must be non-fatal");
        let calls = exec.recorded_calls();
        assert_eq!(calls.len(), 2);
    }
}
