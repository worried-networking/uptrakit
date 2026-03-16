//! Command execution utilities for plugin operations.
//!
//! Re-exports the [`CommandExecutor`] abstraction from `uptrakit_command` so
//! that plugin crates access everything through `uptrakit_plugin_infrastructure_core`.
//!
//! # Shared helper
//!
//! [`execute_and_capture`] is the canonical way for package-manager plugins to run
//! a subprocess and obtain its stdout as a `String`.  It handles the repetitive
//! `map_err` / `warn!` / `PluginError` conversion that previously appeared at every
//! call site.

use rootcause::prelude::*;

use crate::error::PluginError;

// Direct re-exports (no error conversion needed)
pub use uptrakit_command::{send_output, shell_escape};

// Executor abstraction re-exports
pub use uptrakit_command::{
    CommandExecutor, CommandMode, CommandOutput, CommandSpec, LocalCommandExecutor, StdioTunnel,
};

// Error conversion: CommandError -> PluginError
uptrakit_shared_macros::impl_report_conversion!(
    uptrakit_command::CommandError => PluginError, |e| {
        match e {
            uptrakit_command::CommandError::CommandSpawn(io) => PluginError::CommandSpawn(io),
            uptrakit_command::CommandError::CaptureFailed(s) => PluginError::CaptureFailed(s),
            uptrakit_command::CommandError::CommandFailed(code) => PluginError::CommandFailed(code),
            uptrakit_command::CommandError::CommandWait(io) => PluginError::CommandWait(io),
            uptrakit_command::CommandError::TimedOut => PluginError::TimedOut,
            uptrakit_command::CommandError::UnsupportedShell(s) => PluginError::UnsupportedShell(s),
            uptrakit_command::CommandError::UnsupportedOperation(s) => PluginError::UnsupportedOperation(s),
            uptrakit_command::CommandError::PtyAllocationFailed(io) => PluginError::CommandSpawn(io),
        }
    }
);

/// Execute a command quietly and capture its stdout as a [`String`].
///
/// This is the canonical helper for package-manager plugins that need to run a
/// subprocess and inspect its output.  It encapsulates the repetitive
/// `execute_quiet` → `map_err` → `warn!` → `PluginError` pattern that would
/// otherwise appear at every call site.
///
/// # Behaviour
///
/// 1. Calls [`CommandExecutor::execute_quiet`] with `cmd`.
/// 2. On a process-level error (spawn failure, I/O error, timeout, …), logs a
///    warning and propagates a [`PluginError::PluginInternal`] that includes
///    `context` and the underlying error message.
/// 3. On a non-zero exit code, propagates [`PluginError::CommandFailed`] with
///    that exit code.
/// 4. On success (`exit_code == 0`) returns the captured stdout as a `String`.
///
/// # Parameters
///
/// * `executor` – the [`CommandExecutor`] to drive.
/// * `cmd` – the [`CommandSpec`] describing the subprocess.
/// * `context` – a short human-readable label used in log messages and error
///   strings (e.g. `"dpkg-query"`, `"apt-cache madison"`).
///
/// # Errors
///
/// Returns `Err` wrapped in [`rootcause::Report`] for any of the failure cases
/// described above.
///
/// # Examples
///
/// ```ignore
/// use uptrakit_plugin_infrastructure_core::command::{execute_and_capture, CommandSpec};
///
/// let output = execute_and_capture(
///     executor.as_ref(),
///     CommandSpec::exec("dpkg-query", ["--show", "--showformat=${Version}\\n", "nginx"]),
///     "dpkg-query",
/// )
/// .await?;
/// ```
pub async fn execute_and_capture(
    executor: &dyn CommandExecutor,
    cmd: CommandSpec,
    context: &str,
) -> crate::error::Result<String> {
    let cmd_output = executor.execute_quiet(&cmd).await.map_err(|e| {
        tracing::warn!(context, error = ?e, "command failed");
        report!(PluginError::PluginInternal(format!(
            "{context} failed: {e}"
        )))
    })?;

    if cmd_output.exit_code != 0 {
        tracing::warn!(
            context,
            exit_code = cmd_output.exit_code,
            "command exited with non-zero status"
        );
        bail!(PluginError::CommandFailed(cmd_output.exit_code));
    }

    Ok(cmd_output.output)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use rootcause::prelude::*;

    use super::*;

    /// Minimal inline executor for unit tests in this module.
    ///
    /// `execute_quiet` returns `Ok(CommandOutput { output, exit_code })` for
    /// exit_code 0 and `Err(CommandError::CommandFailed(code))` for non-zero
    /// codes — matching the contract that [`execute_and_capture`] must handle.
    struct StubExecutor {
        output: &'static str,
        exit_code: i32,
    }

    #[async_trait]
    impl CommandExecutor for StubExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<crate::UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.to_string(),
                exit_code: self.exit_code,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.to_string(),
                exit_code: self.exit_code,
            })
        }
    }

    // ── execute_and_capture ────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_and_capture_success_returns_stdout() {
        let executor = StubExecutor {
            output: "hello world\n",
            exit_code: 0,
        };
        let result = execute_and_capture(
            &executor,
            CommandSpec::exec("echo", ["hello world".to_string()]),
            "echo",
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello world\n");
    }

    #[tokio::test]
    async fn execute_and_capture_nonzero_exit_propagates_command_failed() {
        let executor = StubExecutor {
            output: "",
            exit_code: 2,
        };
        let err = execute_and_capture(
            &executor,
            CommandSpec::exec("false", [] as [String; 0]),
            "false",
        )
        .await
        .unwrap_err();
        let kind = err.current_context();
        assert!(matches!(kind, PluginError::CommandFailed(2)));
    }

    #[tokio::test]
    async fn execute_and_capture_process_error_propagates_plugin_internal() {
        /// Executor that always returns a process-level spawn error.
        struct AlwaysFailExecutor;

        #[async_trait]
        impl CommandExecutor for AlwaysFailExecutor {
            async fn execute(
                &self,
                _spec: &CommandSpec,
                _output_tx: &tokio::sync::mpsc::Sender<crate::UpdateOutputLine>,
            ) -> uptrakit_command::Result<CommandOutput> {
                bail!(uptrakit_command::CommandError::CommandSpawn(
                    std::io::Error::new(std::io::ErrorKind::NotFound, "not found")
                ))
            }

            async fn execute_quiet(
                &self,
                _spec: &CommandSpec,
            ) -> uptrakit_command::Result<CommandOutput> {
                bail!(uptrakit_command::CommandError::CommandSpawn(
                    std::io::Error::new(std::io::ErrorKind::NotFound, "not found")
                ))
            }
        }

        let err = execute_and_capture(
            &AlwaysFailExecutor,
            CommandSpec::exec("missing-binary", [] as [String; 0]),
            "missing-binary",
        )
        .await
        .unwrap_err();
        let kind = err.current_context();
        assert!(matches!(kind, PluginError::PluginInternal(_)));
    }
}
