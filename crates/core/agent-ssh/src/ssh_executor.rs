//! SSH-backed [`CommandExecutor`] implementation.
//!
//! [`SshCommandExecutor`] runs commands on remote hosts via an
//! [`SshSession`], bridging the provider command abstraction with
//! the SSH transport layer.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_command::{
    CommandError, CommandExecutor, CommandOutput, CommandSpec, UpdateOutputLine, shell_escape,
};

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
        let remote_cmd = build_remote_command_string(spec);

        let result = self
            .session
            .exec_command_streaming(&remote_cmd, Some(output_tx))
            .await
            .map_err(|e| {
                report!(CommandError::CommandSpawn(std::io::Error::other(
                    e.to_string()
                )))
            })?;

        if result.exit_code != 0 {
            let exit_code = i32::try_from(result.exit_code).unwrap_or(-1);
            bail!(CommandError::CommandFailed(exit_code));
        }

        let mut output = result.stdout;
        output.push_str(&result.stderr);

        let exit_code = i32::try_from(result.exit_code).unwrap_or(0);
        Ok(CommandOutput { output, exit_code })
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        let remote_cmd = build_remote_command_string(spec);

        let result = self
            .session
            .exec_command_streaming(&remote_cmd, None)
            .await
            .map_err(|e| {
                report!(CommandError::CommandSpawn(std::io::Error::other(
                    e.to_string()
                )))
            })?;

        if result.exit_code != 0 {
            let exit_code = i32::try_from(result.exit_code).unwrap_or(-1);
            bail!(CommandError::CommandFailed(exit_code));
        }

        let mut output = result.stdout;
        output.push_str(&result.stderr);

        let exit_code = i32::try_from(result.exit_code).unwrap_or(0);
        Ok(CommandOutput { output, exit_code })
    }
}

/// Convert a [`CommandSpec`] to a single shell-safe command string
/// suitable for remote execution over SSH.
///
/// Uses `spec.resolve()` to obtain `(program, args)` for both Exec and
/// Shell modes, then shell-escapes each component. When `working_dir`
/// is set, prepends `cd '<dir>' && `.
fn build_remote_command_string(spec: &CommandSpec) -> String {
    let (program, args) = spec.resolve();

    let mut parts = Vec::with_capacity(1 + args.len());
    parts.push(shell_escape(&program));
    for arg in &args {
        parts.push(shell_escape(arg));
    }

    let command_str = parts.join(" ");

    match &spec.working_dir {
        Some(dir) => format!("cd {} && {}", shell_escape(dir), command_str),
        None => command_str,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_remote_command_string tests ─────────────────────────────

    #[test]
    fn exec_simple_command() {
        let spec = CommandSpec::exec("echo", ["hello".to_string()]);
        let result = build_remote_command_string(&spec);
        assert_eq!(result, "'echo' 'hello'");
    }

    #[test]
    fn exec_with_spaces_in_args() {
        let spec = CommandSpec::exec(
            "docker",
            ["pull".to_string(), "my image:latest".to_string()],
        );
        let result = build_remote_command_string(&spec);
        assert_eq!(result, "'docker' 'pull' 'my image:latest'");
    }

    #[test]
    fn exec_with_quotes_in_args() {
        let spec = CommandSpec::exec("echo", ["it's a test".to_string()]);
        let result = build_remote_command_string(&spec);
        assert_eq!(result, "'echo' 'it'\\''s a test'");
    }

    #[test]
    fn exec_no_args() {
        let spec = CommandSpec::exec("whoami", Vec::<String>::new());
        let result = build_remote_command_string(&spec);
        assert_eq!(result, "'whoami'");
    }

    #[test]
    fn shell_mode_wraps_with_fail_early() {
        let spec = CommandSpec::shell("echo hello && echo world");
        let result = build_remote_command_string(&spec);
        // Shell mode resolves to: ("bash", ["-c", "set -euo pipefail\n..."])
        assert!(result.starts_with("'bash' '-c'"));
        assert!(result.contains("set -euo pipefail"));
        assert!(result.contains("echo hello && echo world"));
    }

    #[test]
    fn with_working_dir() {
        let spec = CommandSpec::exec("ls", ["-la".to_string()]).with_working_dir("/opt/my app");
        let result = build_remote_command_string(&spec);
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
        let result = build_remote_command_string(&spec);
        // All special characters are safely wrapped in single quotes.
        assert_eq!(result, "'echo' '$(whoami)' '; rm -rf /' '`id`'");
    }

    #[test]
    fn working_dir_with_special_chars() {
        let spec = CommandSpec::exec("ls", Vec::<String>::new())
            .with_working_dir("/opt/dir with spaces; rm -rf /");
        let result = build_remote_command_string(&spec);
        assert!(result.starts_with("cd '/opt/dir with spaces; rm -rf /'"));
    }
}
