//! Command execution abstraction.
//!
//! Defines [`CommandExecutor`] for decoupling command preparation from execution,
//! and [`LocalCommandExecutor`] for running commands on the local machine.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::command::run_command_exec_impl;
use crate::error::CommandError;
use rootcause::prelude::*;

use crate::types::UpdateOutputLine;

// Re-export types that were previously defined here, so that existing
// `use crate::executor::{CommandOutput, CommandSpec, …}` paths keep working.
#[cfg(feature = "interactive")]
pub use crate::types::InteractiveHandle;
pub use crate::types::{CommandMode, CommandOutput, CommandSpec};

/// A bidirectional byte stream connected to a remote command's stdin/stdout.
///
/// Implementations wrap transport-specific channel types (e.g. russh
/// `ChannelStream`) and expose them as a unified `AsyncRead + AsyncWrite`
/// interface. The Docker plugin uses this to run `docker system dial-stdio`
/// over an SSH channel without spawning a second SSH connection.
pub trait StdioTunnel: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin {}

/// Trait for executing commands.
///
/// Plugins call methods on this trait instead of spawning processes directly.
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

    /// Whether this executor supports opening stdio tunnels.
    ///
    /// When `true`, [`open_stdio_tunnel`](Self::open_stdio_tunnel) can be
    /// called to obtain a bidirectional byte stream connected to a remote
    /// command's stdin/stdout. The default implementation returns `false`.
    fn supports_stdio_tunnel(&self) -> bool {
        false
    }

    /// Open a bidirectional stdio tunnel to the given command.
    ///
    /// The returned [`StdioTunnel`] connects the caller's reads/writes to the
    /// remote command's stdout/stdin respectively. This is used by the Docker
    /// plugin to run `docker system dial-stdio` over an existing SSH session
    /// without spawning a second SSH connection.
    ///
    /// The default implementation returns
    /// [`CommandError::UnsupportedOperation`].
    async fn open_stdio_tunnel(&self, _command: &str) -> crate::Result<Box<dyn StdioTunnel>> {
        bail!(CommandError::UnsupportedOperation(
            "stdio tunnel not supported by this executor".into()
        ))
    }

    /// Whether this executor supports interactive (PTY-backed) execution.
    ///
    /// When `true`, [`execute_interactive`](Self::execute_interactive) can be
    /// called to run a command with a real terminal, stdin forwarding, and
    /// signal delivery. The default implementation returns `false`.
    #[cfg(feature = "interactive")]
    fn supports_interactive(&self) -> bool {
        false
    }

    /// Execute a command interactively with PTY allocation.
    ///
    /// The returned [`InteractiveHandle`] provides channels for stdin/signal
    /// forwarding and a completion future. Output is streamed via `output_tx`.
    ///
    /// The default implementation returns [`CommandError::UnsupportedOperation`].
    #[cfg(feature = "interactive")]
    async fn execute_interactive(
        &self,
        _spec: &CommandSpec,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> crate::Result<InteractiveHandle> {
        bail!(CommandError::UnsupportedOperation(
            "interactive execution not supported by this executor".into()
        ))
    }
}

/// Convert a [`CommandSpec`] to a single shell-safe command string
/// suitable for remote execution.
///
/// Uses `spec.resolve()` to obtain `(program, args)` for both Exec and
/// Shell modes, then shell-escapes each component. When `working_dir`
/// is set, prepends `cd '<dir>' &&`.
///
/// Returns [`CommandError::UnsupportedShell`] if the shell variant is
/// not recognized.
pub fn build_remote_command_string(spec: &CommandSpec) -> crate::Result<String> {
    let (program, args) = spec.resolve()?;

    let mut parts = Vec::with_capacity(spec.envs.len() + 1 + args.len());
    for (name, value) in &spec.envs {
        parts.push(format!("{name}={}", crate::shell_escape(value)));
    }
    parts.push(crate::shell_escape(&program));
    for arg in &args {
        parts.push(crate::shell_escape(arg));
    }

    let command_str = parts.join(" ");

    Ok(match &spec.working_dir {
        Some(dir) => format!("cd {} && {}", crate::shell_escape(dir), command_str),
        None => command_str,
    })
}

/// Apply an optional timeout to a command execution future.
///
/// If `timeout` is `Some(dur)`, the future is wrapped with
/// [`tokio::time::timeout`]. On expiry a [`CommandError::TimedOut`] is
/// returned. If `timeout` is `None` the future is awaited directly.
#[expect(
    clippy::map_err_ignore,
    reason = "tokio::time::error::Elapsed carries no additional context beyond the fact that the timeout expired"
)]
async fn apply_timeout(
    fut: impl std::future::Future<Output = crate::Result<(String, i32)>>,
    timeout: Option<std::time::Duration>,
) -> crate::Result<(String, i32)> {
    if let Some(dur) = timeout {
        tokio::time::timeout(dur, fut).await.map_err(|_| {
            tracing::warn!(timeout = ?dur, "command timed out");
            report!(CommandError::TimedOut)
        })?
    } else {
        fut.await
    }
}

/// A [`CommandExecutor`] that returns an error on use.
///
/// The controller process never executes local commands directly for
/// API-based plugins (GitHub, Docker, npm registry). This struct satisfies
/// the `Arc<dyn CommandExecutor>` requirement of plugin construction without
/// pulling in a real executor.
///
/// Calling either method is a bug in the calling code; it returns
/// [`CommandError::UnsupportedOperation`] with a diagnostic message instead
/// of panicking.
pub struct NoopCommandExecutor;

#[async_trait]
impl CommandExecutor for NoopCommandExecutor {
    async fn execute(
        &self,
        _spec: &CommandSpec,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> crate::Result<CommandOutput> {
        bail!(CommandError::UnsupportedOperation(
            "NoopCommandExecutor::execute called on the controller — this is a bug".into()
        ))
    }

    async fn execute_quiet(&self, _spec: &CommandSpec) -> crate::Result<CommandOutput> {
        bail!(CommandError::UnsupportedOperation(
            "NoopCommandExecutor::execute_quiet called on the controller — this is a bug".into()
        ))
    }
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
        tracing::debug!(timeout = ?spec.timeout, "executing command");
        let (program, args) = spec.resolve()?;
        let fut = run_command_exec_impl(
            &program,
            &args,
            spec.working_dir.as_deref(),
            &spec.envs,
            Some(output_tx),
        );
        let (output, exit_code) = apply_timeout(fut, spec.timeout).await?;
        tracing::debug!(exit_code, "command completed");
        Ok(CommandOutput { output, exit_code })
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> crate::Result<CommandOutput> {
        tracing::debug!("executing command (quiet)");
        let (program, args) = spec.resolve()?;
        let fut = run_command_exec_impl(
            &program,
            &args,
            spec.working_dir.as_deref(),
            &spec.envs,
            None,
        );
        let (output, exit_code) = apply_timeout(fut, spec.timeout).await?;
        tracing::debug!(exit_code, "command completed");
        Ok(CommandOutput { output, exit_code })
    }

    #[cfg(feature = "interactive")]
    fn supports_interactive(&self) -> bool {
        true
    }

    #[cfg(feature = "interactive")]
    async fn execute_interactive(
        &self,
        spec: &CommandSpec,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> crate::Result<InteractiveHandle> {
        crate::interactive::run_interactive(spec, output_tx).await
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err checks are idiomatic in tests"
    )]
    use super::*;
    use std::time::Duration;
    use uptrakit_shared_types::HookShell;

    // ── CommandSpec constructor tests ──────────────────────────────────

    #[test]
    fn exec_constructor() {
        let spec = CommandSpec::exec("echo", ["hello".to_string()]);
        assert!(matches!(&spec.mode, CommandMode::Exec { program, args }
            if program == "echo" && args == &["hello".to_string()]));
        assert!(spec.working_dir.is_none());
        assert!(spec.timeout.is_none());
        assert!(!spec.privileged);
        assert!(spec.envs.is_empty());
    }

    #[test]
    fn with_env_builder_single() {
        let spec = CommandSpec::exec("apt-get", Vec::<String>::new())
            .with_env("DEBIAN_FRONTEND", "noninteractive");
        assert_eq!(
            spec.envs,
            vec![("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string())]
        );
    }

    #[test]
    fn with_env_builder_chained() {
        let spec = CommandSpec::exec("env", Vec::<String>::new())
            .with_env("FOO", "bar")
            .with_env("BAZ", "qux");
        assert_eq!(spec.envs.len(), 2);
        assert_eq!(spec.envs[0], ("FOO".to_string(), "bar".to_string()));
        assert_eq!(spec.envs[1], ("BAZ".to_string(), "qux".to_string()));
    }

    #[test]
    fn privileged_builder() {
        let spec = CommandSpec::exec("apt-get", Vec::<String>::new()).privileged();
        assert!(spec.privileged);
    }

    #[test]
    fn shell_constructor_not_privileged_by_default() {
        let spec = CommandSpec::shell("echo hello");
        assert!(!spec.privileged);
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
        let (program, args) = spec.resolve().expect("exec mode always succeeds");
        assert_eq!(program, "docker");
        assert_eq!(args, vec!["pull", "nginx"]);
    }

    #[test]
    fn resolve_shell_wraps_with_fail_early() {
        let spec = CommandSpec::shell("echo hello");
        let (program, args) = spec.resolve().expect("bash is a supported shell");
        assert_eq!(program, "bash");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-c");
        assert!(args[1].starts_with("set -euo pipefail\n"));
        assert!(args[1].ends_with("echo hello"));
    }

    #[test]
    fn resolve_shell_sh() {
        let spec = CommandSpec::shell_with("echo hello", HookShell::Sh);
        let (program, args) = spec.resolve().expect("sh is a supported shell");
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

    // ── NoopCommandExecutor tests ────────────────────────────────────

    #[tokio::test]
    async fn noop_executor_returns_error_on_execute() {
        let executor = NoopCommandExecutor;
        let spec = CommandSpec::exec("echo", ["hello".to_string()]);
        let (tx, _rx) = mpsc::channel(1);
        let result = executor.execute(&spec, &tx).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            CommandError::UnsupportedOperation(_)
        ));
    }

    #[tokio::test]
    async fn noop_executor_returns_error_on_execute_quiet() {
        let executor = NoopCommandExecutor;
        let spec = CommandSpec::exec("echo", ["hello".to_string()]);
        let result = executor.execute_quiet(&spec).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            CommandError::UnsupportedOperation(_)
        ));
    }
}
