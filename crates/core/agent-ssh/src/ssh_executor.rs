//! SSH-backed [`CommandExecutor`] implementation.
//!
//! [`SshCommandExecutor`] runs commands on remote hosts via an
//! [`SshSession`], bridging the plugin command abstraction with
//! the SSH transport layer.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_command::{
    CommandError, CommandExecutor, CommandOutput, CommandSpec, StdioTunnel, UpdateOutputLine,
    shell_escape,
};

use crate::ssh_stdio_tunnel::SshStdioTunnel;
use crate::ssh_transport::SshSession;

/// Executes commands on a remote host via an SSH session.
pub struct SshCommandExecutor {
    session: Arc<SshSession>,
}

impl SshCommandExecutor {
    /// Create a new executor backed by the given SSH session.
    pub fn new(session: Arc<SshSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl CommandExecutor for SshCommandExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        let remote_cmd = build_remote_command_string(spec)?;

        let fut = self
            .session
            .exec_command_streaming(&remote_cmd, Some(output_tx));

        let result = if let Some(dur) = spec.timeout {
            tokio::time::timeout(dur, fut)
                .await
                .map_err(|_| {
                    tracing::warn!(timeout = ?dur, "SSH command timed out");
                    report!(CommandError::TimedOut)
                })?
                .map_err(|e| {
                    report!(CommandError::CommandSpawn(std::io::Error::other(
                        e.to_string()
                    )))
                })?
        } else {
            fut.await.map_err(|e| {
                report!(CommandError::CommandSpawn(std::io::Error::other(
                    e.to_string()
                )))
            })?
        };

        if result.exit_code != 0 {
            let exit_code = i32::try_from(result.exit_code).unwrap_or(-1);
            log_failed_command_output(exit_code, &result.stderr, &result.stdout);
            bail!(CommandError::CommandFailed(exit_code));
        }

        let mut output = result.stdout;
        output.push_str(&result.stderr);

        let exit_code = i32::try_from(result.exit_code).unwrap_or(0);
        Ok(CommandOutput { output, exit_code })
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        let remote_cmd = build_remote_command_string(spec)?;

        let fut = self
            .session
            .exec_command_streaming(&remote_cmd, None);

        let result = if let Some(dur) = spec.timeout {
            tokio::time::timeout(dur, fut)
                .await
                .map_err(|_| {
                    tracing::warn!(timeout = ?dur, "SSH command timed out");
                    report!(CommandError::TimedOut)
                })?
                .map_err(|e| {
                    report!(CommandError::CommandSpawn(std::io::Error::other(
                        e.to_string()
                    )))
                })?
        } else {
            fut.await.map_err(|e| {
                report!(CommandError::CommandSpawn(std::io::Error::other(
                    e.to_string()
                )))
            })?
        };

        if result.exit_code != 0 {
            let exit_code = i32::try_from(result.exit_code).unwrap_or(-1);
            // Do not log here: quiet execution is used for probing / compatibility
            // checks where a non-zero exit code is an expected, routine outcome
            // (e.g. `which brew` returning 1 when Homebrew is not installed).
            // The error is propagated to the caller, which decides how to handle it.
            bail!(CommandError::CommandFailed(exit_code));
        }

        let mut output = result.stdout;
        output.push_str(&result.stderr);

        let exit_code = i32::try_from(result.exit_code).unwrap_or(0);
        Ok(CommandOutput { output, exit_code })
    }

    fn supports_stdio_tunnel(&self) -> bool {
        true
    }

    async fn open_stdio_tunnel(
        &self,
        command: &str,
    ) -> uptrakit_command::Result<Box<dyn StdioTunnel>> {
        let channel = self
            .session
            .open_channel_for_command(command)
            .await
            .map_err(|e| {
                report!(CommandError::CommandSpawn(std::io::Error::other(
                    e.to_string()
                )))
            })?;
        Ok(Box::new(SshStdioTunnel::new(channel)))
    }
}

/// Log stderr (and stdout if stderr is empty) from a failed remote command.
///
/// When a remote command fails, the exit code alone is rarely enough to
/// diagnose the problem. This helper emits a WARN-level log with the
/// command's stderr output so that failures like "command not found",
/// "permission denied", or "No such file" are immediately visible in the
/// agent logs without having to cross-reference streamed output on the
/// controller.
fn log_failed_command_output(exit_code: i32, stderr: &str, stdout: &str) {
    let stderr = stderr.trim();
    let stdout = stdout.trim();
    if !stderr.is_empty() {
        tracing::warn!(exit_code, stderr = %stderr, "remote command failed");
    } else if !stdout.is_empty() {
        // Some programs (notably shell scripts) write errors to stdout.
        tracing::warn!(exit_code, stdout = %stdout, "remote command failed");
    }
}

/// Convert a [`CommandSpec`] to a single shell-safe command string
/// suitable for remote execution over SSH.
///
/// Uses `spec.resolve()` to obtain `(program, args)` for both Exec and
/// Shell modes, then shell-escapes each component. When `working_dir`
/// is set, prepends `cd '<dir>' &&`.
///
/// Returns [`CommandError::UnsupportedShell`] if the shell variant is
/// not recognized by this version of the agent.
fn build_remote_command_string(spec: &CommandSpec) -> uptrakit_command::Result<String> {
    let (program, args) = spec.resolve()?;

    let mut parts = Vec::with_capacity(1 + args.len());
    parts.push(shell_escape(&program));
    for arg in &args {
        parts.push(shell_escape(arg));
    }

    let command_str = parts.join(" ");

    Ok(match &spec.working_dir {
        Some(dir) => format!("cd {} && {}", shell_escape(dir), command_str),
        None => command_str,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_remote_command_string tests ─────────────────────────────

    #[test]
    fn exec_simple_command() {
        let spec = CommandSpec::exec("echo", ["hello".to_string()]);
        let result = build_remote_command_string(&spec).expect("exec mode always succeeds");
        assert_eq!(result, "'echo' 'hello'");
    }

    #[test]
    fn exec_with_spaces_in_args() {
        let spec = CommandSpec::exec(
            "docker",
            ["pull".to_string(), "my image:latest".to_string()],
        );
        let result = build_remote_command_string(&spec).expect("exec mode always succeeds");
        assert_eq!(result, "'docker' 'pull' 'my image:latest'");
    }

    #[test]
    fn exec_with_quotes_in_args() {
        let spec = CommandSpec::exec("echo", ["it's a test".to_string()]);
        let result = build_remote_command_string(&spec).expect("exec mode always succeeds");
        assert_eq!(result, "'echo' 'it'\\''s a test'");
    }

    #[test]
    fn exec_no_args() {
        let spec = CommandSpec::exec("whoami", Vec::<String>::new());
        let result = build_remote_command_string(&spec).expect("exec mode always succeeds");
        assert_eq!(result, "'whoami'");
    }

    #[test]
    fn shell_mode_wraps_with_fail_early() {
        let spec = CommandSpec::shell("echo hello && echo world");
        let result = build_remote_command_string(&spec).expect("bash is a supported shell");
        // Shell mode resolves to: ("bash", ["-c", "set -euo pipefail\n..."])
        assert!(result.starts_with("'bash' '-c'"));
        assert!(result.contains("set -euo pipefail"));
        assert!(result.contains("echo hello && echo world"));
    }

    #[test]
    fn with_working_dir() {
        let spec = CommandSpec::exec("ls", ["-la".to_string()]).with_working_dir("/opt/my app");
        let result = build_remote_command_string(&spec).expect("exec mode always succeeds");
        assert_eq!(result, "cd '/opt/my app' && 'ls' '-la'");
    }

    #[test]
    fn shell_injection_prevention() {
        let spec = CommandSpec::exec(
            "echo",
            [
                "$(whoami)".to_string(),
                "; rm -rf /".to_string(),
                "`id`".to_string(),
            ],
        );
        let result = build_remote_command_string(&spec).expect("exec mode always succeeds");
        // All special characters are safely wrapped in single quotes.
        assert_eq!(result, "'echo' '$(whoami)' '; rm -rf /' '`id`'");
    }

    #[test]
    fn working_dir_with_special_chars() {
        let spec = CommandSpec::exec("ls", Vec::<String>::new())
            .with_working_dir("/opt/dir with spaces; rm -rf /");
        let result = build_remote_command_string(&spec).expect("exec mode always succeeds");
        assert!(result.starts_with("cd '/opt/dir with spaces; rm -rf /'"));
    }
}
