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

use crate::types::{AbandonmentPolicy, DEFAULT_COMMAND_TIMEOUT, UpdateOutputLine};

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
#[expect(
    clippy::let_underscore_must_use,
    reason = "channel send failure (receiver dropped) is intentionally ignored — caller cannot do anything useful with it"
)]
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

/// Cleans up the spawned pipe-reader tasks when `run_command_exec_impl` is
/// abandoned — its future dropped by external cancellation (an op-level
/// deadline, a dropped update task) or an early `bail!`. Disarmed on the
/// normal completion path by taking `handles` out.
///
/// Known ceiling: a `DrainOnAbandon` child abandoned by external
/// cancellation is **not killed** — nothing bounds the child itself, only
/// the drain. When the drain bound expires the readers abort, the pipes
/// close, and a child that writes afterwards takes `SIGPIPE` (default-fatal
/// for most tools) — an approximate, not guaranteed, backstop. The
/// controller-side stalled-update reaper (Plan 1 Task 3/4) marks the
/// corresponding row. The outer pipeline timeout (`update.rs:241-251`) is
/// NOT retired by this change — no milestone removes it (M1.9 covers the
/// SSH stream-loop deadline only), so external cancellation stays a
/// standing abandonment source and the drain window is load-bearing
/// indefinitely. A sudo-elevated child is also root-owned, so the
/// unprivileged agent's `start_kill()` (deadline path) and post-drain
/// reader abort are best-effort against it — SIGPIPE-on-write is the only
/// remaining backstop; the sudoers model accepts this
/// (docs/security/sudoers-management.md).
struct ReaderAbandonGuard {
    handles: Option<(
        tokio::task::JoinHandle<String>,
        tokio::task::JoinHandle<String>,
    )>,
    policy: AbandonmentPolicy,
    /// Bound for `DrainOnAbandon` draining — one full command budget,
    /// measured from the moment of abandonment (a *fresh* window, not the
    /// residual of the command's own deadline). Abandonment routes fire at
    /// or after an op deadline (the outer pipeline wrap `update.rs:241`, the
    /// batch pipeline wrap `client.rs:534`), where the residual is ~0 — a
    /// residual bound would abort the readers immediately and hand the
    /// still-running child `EPIPE` mid-write, exactly what `DrainOnAbandon`
    /// exists to prevent (M1.1 drain assertion: the child does NOT receive
    /// EPIPE).
    // ponytail: the bound is one command budget, not the (larger) op
    // deadline — op constants live in uptrakit-shared-types and importing
    // them here would invert the dependency; the command budget is the
    // tighter bound and is always available locally.
    budget: std::time::Duration,
}

impl Drop for ReaderAbandonGuard {
    fn drop(&mut self) {
        let Some((stdout_handle, stderr_handle)) = self.handles.take() else {
            return;
        };
        match self.policy {
            AbandonmentPolicy::CloseOnAbandon => {
                stdout_handle.abort();
                stderr_handle.abort();
            }
            AbandonmentPolicy::DrainOnAbandon => {
                // Keep draining the pipes so the still-running child never
                // blocks on a full pipe buffer or takes EPIPE mid-write.
                let drain_budget = self.budget;
                let stdout_abort = stdout_handle.abort_handle();
                let stderr_abort = stderr_handle.abort_handle();
                let drain = async move {
                    drop(stdout_handle.await);
                    drop(stderr_handle.await);
                };
                if let Ok(rt) = tokio::runtime::Handle::try_current() {
                    drop(rt.spawn(async move {
                        if tokio::time::timeout(drain_budget, drain).await.is_err() {
                            stdout_abort.abort();
                            stderr_abort.abort();
                        }
                    }));
                } else {
                    // No runtime to drain on (process shutting down): fall
                    // back to aborting the readers.
                    stdout_abort.abort();
                    stderr_abort.abort();
                }
            }
        }
    }
}

/// Core implementation shared by streaming and quiet variants.
///
/// When `output_tx` is `Some`, each line is sent to the channel.
/// When `None`, lines are still accumulated but not streamed.
#[expect(
    clippy::let_underscore_must_use,
    reason = "channel send failures inside spawned tasks are intentionally ignored — receiver may have been dropped"
)]
pub(crate) async fn run_command_exec_impl(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    envs: &[(String, String)],
    output_tx: Option<&mpsc::Sender<UpdateOutputLine>>,
    timeout: std::time::Duration,
    abandonment: AbandonmentPolicy,
) -> crate::Result<(String, i32)> {
    tracing::debug!(program, args = ?args, working_dir = ?working_dir, "spawning command");

    let mut cmd = Command::new(program);
    // CloseOnAbandon: kill_on_drop(true) SIGKILLs the child when its handle
    // drops (timeout expiry or external cancellation), preventing orphans
    // from holding package-manager locks. DrainOnAbandon: the child must
    // survive external cancellation (mutating update command), so the handle
    // drop must not kill it; the deadline path kills explicitly via
    // start_kill(), and tokio's orphan reaper collects the exit status.
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(matches!(abandonment, AbandonmentPolicy::CloseOnAbandon));

    for (name, value) in envs {
        cmd.env(name, value);
    }

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
                        text: format!("{line}\n"),
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
                        text: format!("{line}\n"),
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

    let started = tokio::time::Instant::now();
    let warn_at = started + timeout.mul_f64(0.8);
    let deadline = started + timeout;

    let mut guard = ReaderAbandonGuard {
        handles: Some((stdout_handle, stderr_handle)),
        policy: abandonment,
        budget: timeout,
    };

    // Bound child.wait() by the deadline, warning once at 80% of the budget.
    // The pinned `wait` future holds the &mut child borrow, so the borrow is
    // scoped: the block returns None on deadline expiry, releasing the
    // borrow before start_kill() below.
    let wait_result = {
        let wait = child.wait();
        tokio::pin!(wait);
        let before_warn = tokio::select! {
            res = &mut wait => Some(res),
            () = tokio::time::sleep_until(warn_at) => {
                tracing::warn!(
                    program,
                    timeout_secs = timeout.as_secs(),
                    "command has consumed 80% of its timeout budget"
                );
                None
            }
        };
        match before_warn {
            Some(res) => Some(res),
            None => tokio::select! {
                res = &mut wait => Some(res),
                () = tokio::time::sleep_until(deadline) => None,
            },
        }
    };

    let Some(wait_status) = wait_result else {
        tracing::warn!(
            program,
            timeout_secs = timeout.as_secs(),
            "command timed out"
        );
        // Kill explicitly (not via drop): DrainOnAbandon children are spawned
        // without kill_on_drop, and the budget is the hard bound regardless
        // of abandonment policy. tokio's orphan reaper collects the child
        // after SIGKILL — no bounded reap loop needed here.
        if let Err(e) = child.start_kill() {
            tracing::warn!(program, error = %e, "failed to kill timed-out command");
        }
        // `guard` drops here → readers aborted (Close) or detached to a
        // bounded drain (Drain).
        bail!(CommandError::TimedOut);
    };
    let status = wait_status.map_err(|e| report!(CommandError::CommandWait(e)))?;

    let Some((stdout_handle, stderr_handle)) = guard.handles.take() else {
        // Unreachable: the guard's handles are taken exactly once.
        bail!(CommandError::CaptureFailed(
            "reader task handles".to_string()
        ));
    };
    let (stdout_output, stderr_output) = tokio::join!(stdout_handle, stderr_handle);

    let mut accumulated = String::new();
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

    let exit_code = status.code().unwrap_or(-1);
    tracing::debug!(exit_code, "command exited");

    if !status.success() {
        bail!(CommandError::CommandFailed(exit_code));
    }

    Ok((accumulated, exit_code))
}

/// Run a program directly with arguments (no shell interpretation).
///
/// Returns the accumulated output and exit code on success. Bounded by
/// [`DEFAULT_COMMAND_TIMEOUT`] with [`AbandonmentPolicy::CloseOnAbandon`] —
/// callers wanting a custom timeout or abandonment policy go through a
/// [`crate::types::CommandSpec`] and an executor instead.
pub async fn run_command_exec(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<(String, i32)> {
    run_command_exec_impl(
        program,
        args,
        working_dir,
        &[],
        Some(output_tx),
        DEFAULT_COMMAND_TIMEOUT,
        AbandonmentPolicy::CloseOnAbandon,
    )
    .await
}

/// Run a program directly with arguments, without streaming output.
///
/// Equivalent to [`run_command_exec`] but does not require a channel.
/// Output is still accumulated and returned. Bounded by
/// [`DEFAULT_COMMAND_TIMEOUT`] with [`AbandonmentPolicy::CloseOnAbandon`].
pub async fn run_command_exec_quiet(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> crate::Result<(String, i32)> {
    run_command_exec_impl(
        program,
        args,
        working_dir,
        &[],
        None,
        DEFAULT_COMMAND_TIMEOUT,
        AbandonmentPolicy::CloseOnAbandon,
    )
    .await
}

/// Wrap a command with fail-early shell settings.
///
/// - **Bash**: `set -euo pipefail` (exit on error, undefined vars, pipe failures)
/// - **Sh**: `set -eu` (exit on error, undefined vars)
/// - **PowerShell**: `$ErrorActionPreference = 'Stop'`
///
/// Returns [`CommandError::UnsupportedShell`] if the shell variant is not
/// recognized by this version of the agent.
pub(crate) fn wrap_command_for_shell(cmd: &str, shell: HookShell) -> crate::Result<String> {
    match shell {
        HookShell::Bash => Ok(format!("set -euo pipefail\n{cmd}")),
        HookShell::Sh => Ok(format!("set -eu\n{cmd}")),
        HookShell::PowerShell => Ok(format!("$ErrorActionPreference = 'Stop'\n{cmd}")),
        _ => bail!(crate::error::CommandError::UnsupportedShell(format!(
            "{shell:?}"
        ))),
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
/// Returns the accumulated output and exit code on success. Bounded by
/// [`DEFAULT_COMMAND_TIMEOUT`] (via [`run_command_exec`]).
pub async fn run_command_with_shell(
    cmd: &str,
    shell: HookShell,
    output_tx: &mpsc::Sender<UpdateOutputLine>,
) -> crate::Result<(String, i32)> {
    tracing::trace!(cmd = %cmd, "running shell command");
    let wrapped_cmd = wrap_command_for_shell(cmd, shell)?;
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
/// Bounded by [`DEFAULT_COMMAND_TIMEOUT`] (via [`run_command_exec_quiet`]).
pub async fn run_command_with_shell_quiet(
    cmd: &str,
    shell: HookShell,
) -> crate::Result<(String, i32)> {
    tracing::trace!(cmd = %cmd, "running shell command (quiet)");
    let wrapped_cmd = wrap_command_for_shell(cmd, shell)?;
    let (shell_exec, shell_arg) = get_shell_args(shell);

    run_command_exec_quiet(shell_exec, &[shell_arg.to_string(), wrapped_cmd], None).await
}

/// Run a shell command via bash and stream output (convenience wrapper).
///
/// Returns the accumulated output on success. Bounded by
/// [`DEFAULT_COMMAND_TIMEOUT`] (via [`run_command_with_shell`]).
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
/// Returns the accumulated output on success. Bounded by
/// [`DEFAULT_COMMAND_TIMEOUT`] (via [`run_command_with_shell_quiet`]).
pub async fn run_command_quiet(cmd: &str) -> crate::Result<String> {
    let (output, _) = run_command_with_shell_quiet(cmd, HookShell::Bash).await?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_err()) are idiomatic in tests"
    )]

    use super::*;
    use crate::CommandExecutor;

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
        let wrapped = wrap_command_for_shell("echo hello", HookShell::Bash)
            .expect("Bash is a supported shell");
        assert!(wrapped.starts_with("set -euo pipefail\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn wrap_command_for_sh() {
        let wrapped =
            wrap_command_for_shell("echo hello", HookShell::Sh).expect("Sh is a supported shell");
        assert!(wrapped.starts_with("set -eu\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn wrap_command_for_powershell() {
        let wrapped = wrap_command_for_shell("echo hello", HookShell::PowerShell)
            .expect("PowerShell is a supported shell");
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

    // -- Stdin null tests --

    /// Verify that spawned commands have stdin closed (null) so interactive
    /// prompts like `read` receive EOF immediately and do not block.
    #[tokio::test]
    async fn run_command_exec_quiet_null_stdin_completes() {
        // `read var` gets EOF from /dev/null and returns non-zero, but `|| true`
        // prevents the shell from failing. The printf should always execute.
        let result = run_command_exec_quiet(
            "bash",
            &[
                "-c".to_string(),
                "read var || true; printf 'eof\n'".to_string(),
            ],
            None,
        )
        .await;
        let (output, _exit_code) = result.expect("command should complete without blocking");
        assert!(
            output.contains("eof"),
            "expected 'eof' in output, got: {output:?}"
        );
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

    // -- Environment variable tests --

    #[tokio::test]
    async fn run_command_exec_impl_sets_env_vars() {
        let envs = vec![("MY_TEST_VAR".to_string(), "hello_env".to_string())];
        // Use printenv to echo the env var value.
        let result = run_command_exec_impl(
            "printenv",
            &["MY_TEST_VAR".to_string()],
            None,
            &envs,
            None,
            DEFAULT_COMMAND_TIMEOUT,
            AbandonmentPolicy::CloseOnAbandon,
        )
        .await;
        let (output, exit_code) = result.expect("printenv should succeed");
        assert_eq!(exit_code, 0);
        assert!(
            output.trim() == "hello_env",
            "expected 'hello_env', got: {output:?}"
        );
    }

    // -- Deadline / abandonment kill-path tests --
    //
    // All three use real children and real short timeouts (kill-path
    // exception, docs/development/testing.md — a paused clock auto-advances
    // while the runtime waits on real process I/O).

    /// Kill-path test: real child + real short timeout (documented exception
    /// in docs/development/testing.md — a paused clock auto-advances while
    /// the runtime waits on real process I/O).
    #[tokio::test]
    async fn timeout_expiry_kills_child_and_returns_timed_out() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("marker");
        let script = format!("sleep 5; touch {}", marker.display());
        let spec =
            crate::CommandSpec::shell(script).with_timeout(std::time::Duration::from_millis(300));
        let started = std::time::Instant::now();
        let result = crate::LocalCommandExecutor.execute_quiet(&spec).await;
        let err = result.expect_err("command must time out");
        assert!(matches!(
            err.current_context(),
            crate::CommandError::TimedOut
        ));
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        // Give a killed child no chance to have finished the script.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(!marker.exists(), "child must have been killed before touch");
    }

    /// Kill-path test: real child + real time (documented exception).
    #[tokio::test]
    async fn close_on_abandon_kills_child_when_future_is_dropped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("marker");
        let script = format!("sleep 1; touch {}", marker.display());
        let spec =
            crate::CommandSpec::shell(script).with_timeout(std::time::Duration::from_secs(30));
        {
            let fut = crate::LocalCommandExecutor.execute_quiet(&spec);
            // Poll once so the child is spawned, then drop the future.
            let mut fut = Box::pin(fut);
            let poll = futures::poll!(fut.as_mut());
            assert!(poll.is_pending());
        } // future dropped here → kill_on_drop SIGKILLs the child
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert!(!marker.exists(), "abandoned child must have been killed");
    }

    /// Kill-path test: real child + real time (documented exception).
    #[tokio::test]
    async fn drain_on_abandon_lets_child_finish_when_future_is_dropped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("marker");
        // The child emits output before and after the drop point; DrainOnAbandon
        // must keep the pipes drained so it never takes EPIPE, and must not
        // kill it — the marker file proves it ran to completion.
        let script = format!(
            "echo before; sleep 1; echo after; touch {}",
            marker.display()
        );
        let spec = crate::CommandSpec::shell(script)
            .with_timeout(std::time::Duration::from_secs(30))
            .drain_on_abandon();
        {
            let fut = crate::LocalCommandExecutor.execute_quiet(&spec);
            let mut fut = Box::pin(fut);
            let poll = futures::poll!(fut.as_mut());
            assert!(poll.is_pending());
        } // future dropped here → child survives, readers drain detached
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert!(
            marker.exists(),
            "drain-on-abandon child must run to completion"
        );
    }
}
