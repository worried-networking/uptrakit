//! Shell command execution utilities for update operations.
//!
//! Provides safe command execution with output streaming, shell escaping,
//! and fail-early shell settings.
//!
//! Each public function comes in two flavours:
//! - **streaming** (`run_command_exec`, `run_command_with_shell`, `run_command`)
//!   — requires an `&mpsc::Sender<UpdateOutputLine>` for real-time output.
//! - **quiet** (`run_command_exec_quiet`, `run_command_with_shell_quiet`,
//!   `run_command_quiet`) — no channel needed; output is accumulated and
//!   returned as a `String`.

use std::process::Stdio;

use rootcause::prelude::*;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::error::CommandError;
use uptrakit_shared_types::{HookShell, OutputStreamType};

use crate::types::UpdateOutputLine;

/// Maximum accumulated output size (10 MB) to prevent OOM from runaway commands.
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Escape a string for safe embedding in a shell command.
///
/// Wraps the value in single quotes, escaping any embedded single quotes
/// with the `'\''` idiom (end quote, escaped literal quote, reopen quote).
pub fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Send an output line to the channel.
pub async fn send_output(
    output_tx: &mpsc::Sender<UpdateOutputLine>,
    text: &str,
    stream: OutputStreamType,
) {
    let _ = output_tx
        .send(UpdateOutputLine {
            text: text.to_string(),
            stream,
        })
        .await;
}

/// Core implementation shared by streaming and quiet variants.
///
/// When `output_tx` is `Some`, each line is sent to the channel.
/// When `None`, lines are still accumulated but not streamed.
pub(crate) async fn run_command_exec_impl(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    output_tx: Option<&mpsc::Sender<UpdateOutputLine>>,
) -> crate::Result<(String, i32)> {
    tracing::debug!(program, args = ?args, working_dir = ?working_dir, "spawning command");

    let mut cmd = Command::new(program);
    // kill_on_drop(true) ensures the child is sent SIGKILL when the Child handle is
    // dropped (e.g. on timeout), preventing orphaned processes from holding package
    // manager locks or consuming resources after the executor returns.
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| report!(CommandError::CommandSpawn(e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| report!(CommandError::CaptureFailed("stdout".to_string())))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| report!(CommandError::CaptureFailed("stderr".to_string())))?;

    let mut accumulated = String::new();

    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let stdout_tx = output_tx.cloned();
    let stdout_handle = tokio::spawn(async move {
        let mut lines = stdout_reader.lines();
        let mut output = String::new();
        let mut truncated = false;
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::trace!(line = %line, stream = "stdout", "command output");
            if let Some(ref tx) = stdout_tx {
                let _ = tx
                    .send(UpdateOutputLine {
                        text: line.clone(),
                        stream: OutputStreamType::Stdout,
                    })
                    .await;
            }
            if output.len() < MAX_OUTPUT_BYTES {
                output.push_str(&line);
                output.push('\n');
            } else if !truncated {
                truncated = true;
                tracing::warn!(
                    "stdout output exceeded {MAX_OUTPUT_BYTES} bytes, truncating accumulation"
                );
                output.push_str("\n[output truncated at 10 MB]\n");
            }
        }
        output
    });

    let stderr_tx = output_tx.cloned();
    let stderr_handle = tokio::spawn(async move {
        let mut lines = stderr_reader.lines();
        let mut output = String::new();
        let mut truncated = false;
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::trace!(line = %line, stream = "stderr", "command output");
            if let Some(ref tx) = stderr_tx {
                let _ = tx
                    .send(UpdateOutputLine {
                        text: line.clone(),
                        stream: OutputStreamType::Stderr,
                    })
                    .await;
            }
            if output.len() < MAX_OUTPUT_BYTES {
                output.push_str(&line);
                output.push('\n');
            } else if !truncated {
                truncated = true;
                tracing::warn!(
                    "stderr output exceeded {MAX_OUTPUT_BYTES} bytes, truncating accumulation"
                );
                output.push_str("\n[output truncated at 10 MB]\n");
            }
        }
        output
    });

    let (stdout_output, stderr_output) = tokio::join!(stdout_handle, stderr_handle);

    match stdout_output {
        Ok(out) => accumulated.push_str(&out),
        Err(e) => {
            tracing::error!(error = %e, "stdout reader task failed");
            accumulated.push_str("[stdout reader failed]\n");
        }
    }
    match stderr_output {
        Ok(out) => accumulated.push_str(&out),
        Err(e) => {
            tracing::error!(error = %e, "stderr reader task failed");
            accumulated.push_str("[stderr reader failed]\n");
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| report!(CommandError::CommandWait(e)))?;

    let exit_code = status.code().unwrap_or(-1);
    tracing::debug!(exit_code, "command exited");

    if !status.success() {
        bail!(CommandError::CommandFailed(exit_code));
    }

    Ok((accumulated, exit_code))
}

/// Run a program directly with arguments (no shell interpretation).
///
/// Returns the accumulated output and exit code on success.
pub async fn run_command_exec(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<(String, i32)> {
    run_command_exec_impl(program, args, working_dir, Some(output_tx)).await
}

/// Run a program directly with arguments, without streaming output.
///
/// Equivalent to [`run_command_exec`] but does not require a channel.
/// Output is still accumulated and returned.
pub async fn run_command_exec_quiet(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> crate::Result<(String, i32)> {
    run_command_exec_impl(program, args, working_dir, None).await
}

/// Wrap a command with fail-early shell settings.
///
/// - **Bash**: `set -euo pipefail` (exit on error, undefined vars, pipe failures)
/// - **Sh**: `set -eu` (exit on error, undefined vars)
/// - **PowerShell**: `$ErrorActionPreference = 'Stop'`
pub(crate) fn wrap_command_for_shell(cmd: &str, shell: HookShell) -> String {
    match shell {
        HookShell::Bash => format!("set -euo pipefail\n{cmd}"),
        HookShell::Sh => format!("set -eu\n{cmd}"),
        HookShell::PowerShell => format!("$ErrorActionPreference = 'Stop'\n{cmd}"),
        _ => unimplemented!("wrap_command_for_shell: unsupported HookShell variant"),
    }
}

/// Get the shell executable and arguments for a given shell type.
///
/// Uses [`HookShell::local_executable`] to select the correct binary for the
/// local machine's OS. On Linux/macOS, PowerShell Core (`pwsh`) is used instead
/// of the Windows-only `powershell` binary.
pub(crate) fn get_shell_args(shell: HookShell) -> (&'static str, &'static str) {
    (shell.local_executable(), shell.flag())
}

/// Run a command with the specified shell and fail-early settings.
///
/// Returns the accumulated output and exit code on success.
pub async fn run_command_with_shell(
    cmd: &str,
    shell: HookShell,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<(String, i32)> {
    tracing::trace!(cmd = %cmd, "running shell command");
    let wrapped_cmd = wrap_command_for_shell(cmd, shell);
    let (shell_exec, shell_arg) = get_shell_args(shell);

    run_command_exec(
        shell_exec,
        &[shell_arg.to_string(), wrapped_cmd],
        None,
        output_tx,
    )
    .await
}

/// Run a command with the specified shell, without streaming output.
///
/// Equivalent to [`run_command_with_shell`] but does not require a channel.
pub async fn run_command_with_shell_quiet(
    cmd: &str,
    shell: HookShell,
) -> crate::Result<(String, i32)> {
    tracing::trace!(cmd = %cmd, "running shell command (quiet)");
    let wrapped_cmd = wrap_command_for_shell(cmd, shell);
    let (shell_exec, shell_arg) = get_shell_args(shell);

    run_command_exec_quiet(shell_exec, &[shell_arg.to_string(), wrapped_cmd], None).await
}

/// Run a shell command via bash and stream output (convenience wrapper).
///
/// Returns the accumulated output on success.
pub async fn run_command(
    cmd: &str,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<String> {
    let (output, _) = run_command_with_shell(cmd, HookShell::Bash, output_tx).await?;
    Ok(output)
}

/// Run a shell command via bash, without streaming output.
///
/// Equivalent to [`run_command`] but does not require a channel.
/// Returns the accumulated output on success.
pub async fn run_command_quiet(cmd: &str) -> crate::Result<String> {
    let (output, _) = run_command_with_shell_quiet(cmd, HookShell::Bash).await?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Shell escape tests --

    #[test]
    fn shell_escape_plain_string() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_with_single_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_escape_with_semicolon() {
        assert_eq!(shell_escape("2.0.0; rm -rf /"), "'2.0.0; rm -rf /'");
    }

    #[test]
    fn shell_escape_with_backticks() {
        assert_eq!(shell_escape("`whoami`"), "'`whoami`'");
    }

    #[test]
    fn shell_escape_with_dollar_subshell() {
        assert_eq!(shell_escape("$(id)"), "'$(id)'");
    }

    #[test]
    fn shell_escape_empty_string() {
        assert_eq!(shell_escape(""), "''");
    }

    /// Verify that shell-escaped values prevent command injection.
    #[tokio::test]
    async fn shell_escape_prevents_injection_in_bash() {
        let (tx, mut rx) = mpsc::channel(100);
        let malicious = "2.0.0'; echo 'MARKER";
        let cmd = format!("printf '%s' {}", shell_escape(malicious));
        let result = run_command(&cmd, &tx).await;
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert_eq!(output.trim(), malicious);
        rx.close();
        while rx.recv().await.is_some() {}
    }

    // -- Shell wrapper tests --

    #[test]
    fn wrap_command_for_bash() {
        let wrapped = wrap_command_for_shell("echo hello", HookShell::Bash);
        assert!(wrapped.starts_with("set -euo pipefail\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn wrap_command_for_sh() {
        let wrapped = wrap_command_for_shell("echo hello", HookShell::Sh);
        assert!(wrapped.starts_with("set -eu\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn wrap_command_for_powershell() {
        let wrapped = wrap_command_for_shell("echo hello", HookShell::PowerShell);
        assert!(wrapped.starts_with("$ErrorActionPreference = 'Stop'\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn get_shell_args_bash() {
        let (exec, arg) = get_shell_args(HookShell::Bash);
        assert_eq!(exec, "bash");
        assert_eq!(arg, "-c");
    }

    #[test]
    fn get_shell_args_sh() {
        let (exec, arg) = get_shell_args(HookShell::Sh);
        assert_eq!(exec, "sh");
        assert_eq!(arg, "-c");
    }

    #[test]
    fn get_shell_args_powershell_local() {
        let (exec, flag) = get_shell_args(HookShell::PowerShell);
        #[cfg(target_os = "windows")]
        assert_eq!(exec, "powershell");
        #[cfg(not(target_os = "windows"))]
        assert_eq!(exec, "pwsh");
        assert_eq!(flag, "-Command");
    }

    // -- Run command tests --

    #[tokio::test]
    async fn test_run_command_success() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command("echo 'hello world'", &tx).await;
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert!(output.contains("hello world"));
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_run_command_failure() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command("exit 1", &tx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), CommandError::CommandFailed(1)),
            "Expected CommandFailed(1), got: {err}"
        );
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_run_command_with_shell_success() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command_with_shell("echo 'test'", HookShell::Bash, &tx).await;
        assert!(result.is_ok());
        let (output, exit_code) = result.expect("should succeed");
        assert!(output.contains("test"));
        assert_eq!(exit_code, 0);
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_run_command_with_shell_failure() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command_with_shell("exit 42", HookShell::Bash, &tx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), CommandError::CommandFailed(42)),
            "Expected CommandFailed(42), got: {err}"
        );
        rx.close();
        while rx.recv().await.is_some() {}
    }

    // -- Direct exec tests --

    #[tokio::test]
    async fn run_command_exec_success() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command_exec("echo", &["hello exec".to_string()], None, &tx).await;
        assert!(result.is_ok());
        let (output, exit_code) = result.expect("should succeed");
        assert!(output.contains("hello exec"));
        assert_eq!(exit_code, 0);
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn run_command_exec_failure() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command_exec("false", &[], None, &tx).await;
        assert!(result.is_err());
        rx.close();
        while rx.recv().await.is_some() {}
    }

    // -- Quiet variant tests --

    #[tokio::test]
    async fn run_command_exec_quiet_success() {
        let result = run_command_exec_quiet("echo", &["hello quiet".to_string()], None).await;
        assert!(result.is_ok());
        let (output, exit_code) = result.expect("should succeed");
        assert!(output.contains("hello quiet"));
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn run_command_exec_quiet_failure() {
        let result = run_command_exec_quiet("false", &[], None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err.current_context(),
            CommandError::CommandFailed(_)
        ));
    }

    #[tokio::test]
    async fn run_command_with_shell_quiet_success() {
        let result = run_command_with_shell_quiet("echo 'quiet shell'", HookShell::Bash).await;
        assert!(result.is_ok());
        let (output, exit_code) = result.expect("should succeed");
        assert!(output.contains("quiet shell"));
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn run_command_with_shell_quiet_failure() {
        let result = run_command_with_shell_quiet("exit 7", HookShell::Bash).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), CommandError::CommandFailed(7)),
            "Expected CommandFailed(7), got: {err}"
        );
    }

    #[tokio::test]
    async fn run_command_quiet_success() {
        let result = run_command_quiet("echo 'quiet run'").await;
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert!(output.contains("quiet run"));
    }

    #[tokio::test]
    async fn run_command_quiet_failure() {
        let result = run_command_quiet("exit 3").await;
        assert!(result.is_err());
    }

    // -- Working directory tests --

    #[tokio::test]
    async fn run_command_exec_with_working_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command_exec("pwd", &[], Some(temp.path().to_str().unwrap()), &tx).await;
        assert!(result.is_ok());
        let (output, _) = result.expect("should succeed");
        // On macOS, /tmp is symlinked to /private/tmp.
        let canonical = temp.path().canonicalize().expect("canonicalize");
        assert!(
            output.trim().contains(canonical.to_str().unwrap()),
            "output should contain the working directory, got: {output}"
        );
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn run_command_exec_invalid_working_dir() {
        let result = run_command_exec_quiet(
            "echo",
            &["hello".to_string()],
            Some("/nonexistent/path/that/does/not/exist"),
        )
        .await;
        assert!(result.is_err());
    }

    // -- Spawn failure tests --

    #[tokio::test]
    async fn run_command_exec_nonexistent_program() {
        let result = run_command_exec_quiet("/nonexistent/binary/xyz123", &[], None).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            CommandError::CommandSpawn(_)
        ));
    }

    // -- Stderr output tests --

    #[tokio::test]
    async fn run_command_captures_stderr() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command("echo 'stderr msg' >&2", &tx).await;
        // Command with set -euo pipefail: echo to stderr succeeds with exit 0.
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert!(
            output.contains("stderr msg"),
            "accumulated output should include stderr"
        );
        rx.close();
        while rx.recv().await.is_some() {}
    }

    // -- Multi-line output test --

    #[tokio::test]
    async fn run_command_multiline_output() {
        let result = run_command_quiet("printf 'line1\\nline2\\nline3'").await;
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
    }
}
