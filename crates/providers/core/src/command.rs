//! Shell command execution utilities for provider update operations.
//!
//! Thin wrappers around `uptrakit_command` that convert `CommandError` into
//! `ProviderError` so that callers within the provider ecosystem see the same
//! error type as before.

use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_command::UpdateOutputLine;

use crate::error::ProviderError;
use uptrakit_command::ShellType;

// Direct re-exports (no error conversion needed)
pub use uptrakit_command::{send_output, shell_escape};

// Error conversion: CommandError -> ProviderError
uptrakit_shared_macros::impl_report_conversion!(
    uptrakit_command::CommandError => ProviderError, |e| {
        match e {
            uptrakit_command::CommandError::CommandSpawn(io) => ProviderError::CommandSpawn(io),
            uptrakit_command::CommandError::CaptureFailed(s) => ProviderError::CaptureFailed(s),
            uptrakit_command::CommandError::CommandFailed(code) => ProviderError::CommandFailed(code),
            uptrakit_command::CommandError::CommandWait(io) => ProviderError::CommandWait(io),
        }
    }
);

/// Run a program directly with arguments (no shell interpretation).
///
/// Returns the accumulated output and exit code on success.
pub async fn run_command_exec(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<(String, i32)> {
    uptrakit_command::run_command_exec(program, args, working_dir, output_tx)
        .await
        .context_to()
}

/// Run a command with the specified shell and fail-early settings.
///
/// Returns the accumulated output and exit code on success.
pub async fn run_command_with_shell(
    cmd: &str,
    shell: ShellType,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<(String, i32)> {
    uptrakit_command::run_command_with_shell(cmd, shell, output_tx)
        .await
        .context_to()
}

/// Run a shell command via bash and stream output (convenience wrapper).
///
/// Returns the accumulated output on success.
pub async fn run_command(
    cmd: &str,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<String> {
    uptrakit_command::run_command(cmd, output_tx)
        .await
        .context_to()
}

/// Run a program directly with arguments, without streaming output.
///
/// Equivalent to [`run_command_exec`] but does not require a channel.
pub async fn run_command_exec_quiet(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> crate::Result<(String, i32)> {
    uptrakit_command::run_command_exec_quiet(program, args, working_dir)
        .await
        .context_to()
}

/// Run a command with the specified shell, without streaming output.
///
/// Equivalent to [`run_command_with_shell`] but does not require a channel.
pub async fn run_command_with_shell_quiet(
    cmd: &str,
    shell: ShellType,
) -> crate::Result<(String, i32)> {
    uptrakit_command::run_command_with_shell_quiet(cmd, shell)
        .await
        .context_to()
}

/// Run a shell command via bash, without streaming output.
///
/// Equivalent to [`run_command`] but does not require a channel.
pub async fn run_command_quiet(cmd: &str) -> crate::Result<String> {
    uptrakit_command::run_command_quiet(cmd).await.context_to()
}
