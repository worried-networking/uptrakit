//! Command execution abstraction.
//!
//! Defines [`CommandExecutor`] for decoupling command preparation from execution,
//! and [`LocalCommandExecutor`] for running commands on the local machine.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::command::{get_shell_args, run_command_exec_impl, wrap_command_for_shell};
use crate::error::CommandError;
use rootcause::prelude::*;
use uptrakit_shared_types::HookShell;

use crate::types::UpdateOutputLine;

/// Specification for a command to execute.
///
/// Providers build a `CommandSpec` describing *what* to run, and the injected
/// [`CommandExecutor`] decides *how* to run it (locally, over SSH, etc.).
#[derive(Clone, Debug)]
pub struct CommandSpec {
    /// How the command should be invoked.
    pub mode: CommandMode,
    /// Optional working directory for the command.
    pub working_dir: Option<String>,
    /// Maximum time to wait for the command. `None` means no timeout is applied.
    pub timeout: Option<std::time::Duration>,
}

/// How a command is invoked.
#[derive(Clone, Debug)]
pub enum CommandMode {
    /// Direct program execution (no shell interpretation).
    Exec {
        /// The program to run.
        program: String,
        /// Arguments to pass to the program.
        args: Vec<String>,
    },
    /// Shell-interpreted command with fail-early settings.
    Shell {
        /// The shell command string.
        command: String,
        /// Which shell to use.
        shell: HookShell,
    },
}

/// Output captured from a command execution.
#[derive(Clone, Debug)]
pub struct CommandOutput {
    /// The accumulated stdout followed by stderr output.
    ///
    /// Stdout content always precedes stderr content, regardless of the actual
    /// temporal interleaving of the two streams. This is a fundamental limitation
    /// of reading from separate pipes.
    pub output: String,
    /// The process exit code.
    pub exit_code: i32,
}

impl CommandSpec {
    /// Create a spec for direct program execution (no shell).
    pub fn exec(program: impl Into<String>, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            mode: CommandMode::Exec {
                program: program.into(),
                args: args.into_iter().collect(),
            },
            working_dir: None,
            timeout: None,
        }
    }

    /// Create a spec for a shell command using Bash.
    pub fn shell(command: impl Into<String>) -> Self {
        Self {
            mode: CommandMode::Shell {
                command: command.into(),
                shell: HookShell::Bash,
            },
            working_dir: None,
            timeout: None,
        }
    }

    /// Create a spec for a shell command using the specified shell.
    pub fn shell_with(command: impl Into<String>, shell: HookShell) -> Self {
        Self {
            mode: CommandMode::Shell {
                command: command.into(),
                shell,
            },
            working_dir: None,
            timeout: None,
        }
    }

    /// Set the working directory (builder pattern).
    #[must_use]
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Set a maximum execution time for the command (builder pattern).
    ///
    /// When the deadline is reached, the executor returns
    /// [`CommandError::TimedOut`]. The child process's stdio pipes are closed,
    /// but the orphaned process is not killed; a follow-up task can add
    /// `child.start_kill()` once the executor is restructured to retain the
    /// child handle outside the completion future.
    #[must_use]
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Resolve to (program, args) for process execution.
    ///
    /// - **Exec** mode: returns `(program, args)` unchanged.
    /// - **Shell** mode: wraps the command with fail-early settings and returns
    ///   `(shell_executable, [flag, wrapped_command])`.
    pub fn resolve(&self) -> (String, Vec<String>) {
        match &self.mode {
            CommandMode::Exec { program, args } => (program.clone(), args.clone()),
            CommandMode::Shell { command, shell } => {
                let wrapped = wrap_command_for_shell(command, *shell);
                let (shell_exec, shell_arg) = get_shell_args(*shell);
                (shell_exec.to_string(), vec![shell_arg.to_string(), wrapped])
            }
        }
    }
}

/// Trait for executing commands.
///
/// Providers call methods on this trait instead of spawning processes directly.
/// The default implementation ([`LocalCommandExecutor`]) runs commands on the
/// local machine; future implementations (e.g., SSH) can run them remotely.
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute a command, streaming output through `output_tx`.
    ///
    /// Returns the accumulated output and exit code on success.
    async fn execute(
        &self,
        spec: &CommandSpec,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> crate::Result<CommandOutput>;

    /// Execute a command without streaming output.
    ///
    /// Output is still accumulated and returned.
    async fn execute_quiet(&self, spec: &CommandSpec) -> crate::Result<CommandOutput>;
}

/// Executes commands on the local machine via `tokio::process::Command`.
pub struct LocalCommandExecutor;

#[async_trait]
impl CommandExecutor for LocalCommandExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> crate::Result<CommandOutput> {
        let (program, args) = spec.resolve();
        let fut = run_command_exec_impl(
            &program,
            &args,
            spec.working_dir.as_deref(),
            Some(output_tx),
        );
        let (output, exit_code) = if let Some(dur) = spec.timeout {
            tokio::time::timeout(dur, fut)
                .await
                .map_err(|_| report!(CommandError::TimedOut))??
        } else {
            fut.await?
        };
        Ok(CommandOutput { output, exit_code })
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> crate::Result<CommandOutput> {
        let (program, args) = spec.resolve();
        let fut = run_command_exec_impl(&program, &args, spec.working_dir.as_deref(), None);
        let (output, exit_code) = if let Some(dur) = spec.timeout {
            tokio::time::timeout(dur, fut)
                .await
                .map_err(|_| report!(CommandError::TimedOut))??
        } else {
            fut.await?
        };
        Ok(CommandOutput { output, exit_code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── CommandSpec constructor tests ──────────────────────────────────

    #[test]
    fn exec_constructor() {
        let spec = CommandSpec::exec("echo", ["hello".to_string()]);
        assert!(matches!(&spec.mode, CommandMode::Exec { program, args }
            if program == "echo" && args == &["hello".to_string()]));
        assert!(spec.working_dir.is_none());
        assert!(spec.timeout.is_none());
    }

    #[test]
    fn shell_constructor_defaults_to_bash() {
        let spec = CommandSpec::shell("echo hello");
        assert!(matches!(&spec.mode, CommandMode::Shell { command, shell }
            if command == "echo hello" && *shell == HookShell::Bash));
        assert!(spec.working_dir.is_none());
    }

    #[test]
    fn shell_with_specific_shell() {
        let spec = CommandSpec::shell_with("echo hello", HookShell::Sh);
        assert!(matches!(&spec.mode, CommandMode::Shell { command, shell }
            if command == "echo hello" && *shell == HookShell::Sh));
    }

    #[test]
    fn with_working_dir_builder() {
        let spec = CommandSpec::exec("ls", Vec::<String>::new()).with_working_dir("/tmp");
        assert_eq!(spec.working_dir.as_deref(), Some("/tmp"));
    }

    // ── CommandSpec::resolve tests ────────────────────────────────────

    #[test]
    fn resolve_exec_passthrough() {
        let spec = CommandSpec::exec("docker", ["pull".to_string(), "nginx".to_string()]);
        let (program, args) = spec.resolve();
        assert_eq!(program, "docker");
        assert_eq!(args, vec!["pull", "nginx"]);
    }

    #[test]
    fn resolve_shell_wraps_with_fail_early() {
        let spec = CommandSpec::shell("echo hello");
        let (program, args) = spec.resolve();
        assert_eq!(program, "bash");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-c");
        assert!(args[1].starts_with("set -euo pipefail\n"));
        assert!(args[1].ends_with("echo hello"));
    }

    #[test]
    fn resolve_shell_sh() {
        let spec = CommandSpec::shell_with("echo hello", HookShell::Sh);
        let (program, args) = spec.resolve();
        assert_eq!(program, "sh");
        assert_eq!(args[0], "-c");
        assert!(args[1].starts_with("set -eu\n"));
    }

    // ── LocalCommandExecutor tests ────────────────────────────────────

    #[tokio::test]
    async fn execute_streams_output() {
        let executor = LocalCommandExecutor;
        let (tx, mut rx) = mpsc::channel(100);
        let spec = CommandSpec::shell("echo hello");
        let result = executor.execute(&spec, &tx).await;
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert!(output.output.contains("hello"));
        assert_eq!(output.exit_code, 0);
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_failure() {
        let executor = LocalCommandExecutor;
        let (tx, mut rx) = mpsc::channel(100);
        let spec = CommandSpec::shell("exit 1");
        let result = executor.execute(&spec, &tx).await;
        assert!(result.is_err());
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_quiet_success() {
        let executor = LocalCommandExecutor;
        let spec = CommandSpec::exec("echo", ["quiet test".to_string()]);
        let result = executor.execute_quiet(&spec).await;
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert!(output.output.contains("quiet test"));
        assert_eq!(output.exit_code, 0);
    }

    #[tokio::test]
    async fn execute_quiet_failure() {
        let executor = LocalCommandExecutor;
        let spec = CommandSpec::exec("false", Vec::<String>::new());
        let result = executor.execute_quiet(&spec).await;
        assert!(result.is_err());
    }

    // ── Timeout tests ─────────────────────────────────────────────────

    #[test]
    fn command_spec_no_timeout_by_default() {
        let spec = CommandSpec::exec("echo", Vec::<String>::new());
        assert!(spec.timeout.is_none());
    }

    #[test]
    fn command_spec_with_timeout_builder() {
        let spec =
            CommandSpec::exec("echo", Vec::<String>::new()).with_timeout(Duration::from_secs(5));
        assert_eq!(spec.timeout, Some(Duration::from_secs(5)));
    }

    #[tokio::test(start_paused = true)]
    async fn execute_quiet_timeout_fires() {
        let spec =
            CommandSpec::exec("sleep", ["100".to_string()]).with_timeout(Duration::from_secs(5));
        let executor = LocalCommandExecutor;
        let handle = tokio::spawn(async move { executor.execute_quiet(&spec).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(10)).await;
        let result = handle.await.expect("join");
        assert!(
            matches!(
                result.unwrap_err().current_context(),
                CommandError::TimedOut
            ),
            "expected TimedOut error"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn execute_timeout_fires() {
        let spec =
            CommandSpec::exec("sleep", ["100".to_string()]).with_timeout(Duration::from_secs(5));
        let executor = LocalCommandExecutor;
        let (tx, mut rx) = mpsc::channel(100);
        let handle = tokio::spawn(async move { executor.execute(&spec, &tx).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(10)).await;
        let result = handle.await.expect("join");
        assert!(
            matches!(
                result.unwrap_err().current_context(),
                CommandError::TimedOut
            ),
            "expected TimedOut error"
        );
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_quiet_no_timeout_succeeds() {
        let spec = CommandSpec::exec("echo", ["no timeout".to_string()]);
        let executor = LocalCommandExecutor;
        let result = executor.execute_quiet(&spec).await;
        assert!(result.is_ok());
        assert!(result.unwrap().output.contains("no timeout"));
    }
}
