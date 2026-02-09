//! Update execution module for the Uptrakit agent.
//!
//! Handles the complete update flow:
//! 1. Receive ExecuteUpdate message
//! 2. Send UpdateStarted (with detected from_version)
//! 3. Run pre-update commands sequentially, streaming output
//! 4. Execute actual update (dispatch by provider_type), streaming output
//! 5. Run post-update commands sequentially, streaming output
//! 6. Detect to_version post-update
//! 7. Send UpdateResult with final status and accumulated output
//!
//! ## Shell Execution
//!
//! Commands are executed with fail-early shell settings:
//! - **Bash**: `set -euo pipefail` (exit on error, undefined vars, pipe failures)
//! - **Sh**: `set -eu` (exit on error, undefined vars)
//! - **PowerShell** (future): `$ErrorActionPreference = 'Stop'`

use std::process::Stdio;

use rootcause::prelude::*;
use thiserror::Error as ThisError;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use uptrakit_internal_wire::{
    ExecuteUpdatePayload, HookCommand, HookShell, OutputStreamType, ProviderType,
    UpdateFinalStatus, UpdateResultPayload,
};

use crate::error::Error;

/// Maximum accumulated output size (10 MB) to prevent OOM from runaway commands.
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Marker appended when output is truncated at the limit.
const TRUNCATION_MARKER: &str = "\n... [output truncated at 10 MB] ...\n";

/// Escape a string for safe embedding in a shell command.
///
/// Wraps the value in single quotes, escaping any embedded single quotes
/// with the `'\''` idiom (end quote, escaped literal quote, reopen quote).
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Append text to a bounded buffer. Once the buffer reaches `max` bytes,
/// further text is silently dropped and a single truncation marker is appended.
fn append_bounded(buffer: &mut String, text: &str, max: usize) {
    if buffer.len() >= max {
        return;
    }
    let remaining = max - buffer.len();
    if text.len() <= remaining {
        buffer.push_str(text);
    } else {
        buffer.push_str(&text[..remaining]);
        buffer.push_str(TRUNCATION_MARKER);
    }
}

/// Errors that can occur during update execution.
#[derive(Debug, ThisError)]
pub(crate) enum UpdateError {
    #[error("no release info provided")]
    MissingReleaseInfo,

    #[error("missing provider config field: {0}")]
    MissingConfig(String),

    #[error("command spawn failed: {0}")]
    CommandSpawn(#[source] std::io::Error),

    #[error("failed to capture {0}")]
    CaptureFailed(String),

    #[error("command exited with code {0}")]
    CommandFailed(i32),

    #[error("command execution failed: {0}")]
    CommandWait(#[source] std::io::Error),

    #[error("install command failed: {0}")]
    InstallFailed(String),
}

pub(crate) type UpdateResult<T> = std::result::Result<T, Report<UpdateError>>;

/// Result of an update execution.
pub struct UpdateExecutionResult {
    pub result: UpdateResultPayload,
}

/// Output message sent during update execution.
pub struct UpdateOutputMessage {
    pub output: String,
    pub stream: OutputStreamType,
}

/// Execute an update based on the payload.
///
/// This function runs the complete update flow and sends output lines through
/// the provided channel. The channel receiver should forward these as
/// UpdateOutput messages to the controller.
pub async fn execute_update(
    payload: ExecuteUpdatePayload,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
) -> UpdateExecutionResult {
    let update_history_id = payload.update_history_id;

    // Detect current version (from_version)
    let from_version = detect_current_version(&payload).await;

    let mut accumulated_output = String::new();
    let mut final_error: Option<String> = None;
    let mut final_status = UpdateFinalStatus::Completed;

    // Run with timeout
    let timeout_duration = std::time::Duration::from_secs(u64::from(payload.timeout_seconds));
    let execution_result = tokio::time::timeout(timeout_duration, async {
        // Run pre-update hooks
        if !payload.pre_update_hooks.is_empty() {
            send_output(
                &output_tx,
                "[pre-hook] Starting pre-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for hook_cmd in &payload.pre_update_hooks {
                send_output(
                    &output_tx,
                    &format!("[pre-hook] Running: {hook_cmd}"),
                    OutputStreamType::PreHook,
                )
                .await;

                match run_hook_command(hook_cmd, OutputStreamType::PreHook, &output_tx).await {
                    Ok((output, exit_code)) => {
                        append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES);
                        send_output(
                            &output_tx,
                            &format!("[pre-hook] (exit code {exit_code})"),
                            OutputStreamType::PreHook,
                        )
                        .await;
                    }
                    Err(e) => {
                        let error_msg = format!("[pre-hook] Failed: {e}");
                        send_output(&output_tx, &error_msg, OutputStreamType::System).await;
                        return Err(Error::PreUpdateHookFailed(e.to_string()));
                    }
                }
            }
        }

        // Execute actual update based on provider type
        send_output(
            &output_tx,
            &format!(
                "[update] Executing update to version {}...",
                payload.to_version
            ),
            OutputStreamType::System,
        )
        .await;

        match execute_provider_update(&payload, &output_tx).await {
            Ok(output) => {
                append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES);
            }
            Err(e) => {
                return Err(Error::UpdateExecution(e.to_string()));
            }
        }

        // Run post-update hooks
        if !payload.post_update_hooks.is_empty() {
            send_output(
                &output_tx,
                "[post-hook] Starting post-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for hook_cmd in &payload.post_update_hooks {
                send_output(
                    &output_tx,
                    &format!("[post-hook] Running: {hook_cmd}"),
                    OutputStreamType::PostHook,
                )
                .await;

                match run_hook_command(hook_cmd, OutputStreamType::PostHook, &output_tx).await {
                    Ok((output, exit_code)) => {
                        append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES);
                        send_output(
                            &output_tx,
                            &format!("[post-hook] (exit code {exit_code})"),
                            OutputStreamType::PostHook,
                        )
                        .await;
                    }
                    Err(e) => {
                        let error_msg = format!("[post-hook] Failed: {e}");
                        send_output(&output_tx, &error_msg, OutputStreamType::System).await;
                        return Err(Error::PostUpdateHookFailed(e.to_string()));
                    }
                }
            }
        }

        Ok(())
    })
    .await;

    // Handle timeout or execution result
    let to_version = match execution_result {
        Ok(Ok(())) => {
            send_output(
                &output_tx,
                "[update] Update completed successfully",
                OutputStreamType::System,
            )
            .await;
            // Detect new version after update
            detect_current_version(&payload).await
        }
        Ok(Err(e)) => {
            final_status = UpdateFinalStatus::Failed;
            final_error = Some(e.to_string());
            None
        }
        Err(_) => {
            final_status = UpdateFinalStatus::Failed;
            final_error = Some(format!(
                "Update timed out after {} seconds",
                payload.timeout_seconds
            ));
            send_output(
                &output_tx,
                &format!(
                    "[update] Update timed out after {} seconds",
                    payload.timeout_seconds
                ),
                OutputStreamType::System,
            )
            .await;
            None
        }
    };

    let result = UpdateResultPayload {
        update_history_id,
        status: final_status,
        from_version,
        to_version,
        output: accumulated_output,
        error: final_error,
    };

    UpdateExecutionResult { result }
}

/// Detect the current version of a software item.
///
/// This is a placeholder implementation. In a real system, this would:
/// - Query package managers (apt, yum, brew)
/// - Check version files
/// - Run version detection commands from the provider config
async fn detect_current_version(_payload: &ExecuteUpdatePayload) -> Option<String> {
    // TODO: Implement actual version detection based on provider_type
    // For now, return None to indicate unknown version
    None
}

/// Execute the provider-specific update logic.
async fn execute_provider_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<String> {
    match payload.provider_type {
        ProviderType::GithubReleases => execute_github_releases_update(payload, output_tx).await,
        ProviderType::ProxmoxHelperScripts => {
            execute_proxmox_helper_scripts_update(payload, output_tx).await
        }
        ProviderType::DockerRegistry => execute_docker_registry_update(payload, output_tx).await,
    }
}

/// Execute a GitHub Releases update.
async fn execute_github_releases_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<String> {
    let mut output = String::new();

    // Extract release info
    let Some(release_info) = &payload.release_info else {
        return Err(report!(UpdateError::MissingReleaseInfo));
    };

    send_output(
        output_tx,
        &format!(
            "Downloading release {} from {}",
            release_info.tag, release_info.release_url
        ),
        OutputStreamType::Stdout,
    )
    .await;
    output.push_str(&format!(
        "Downloading release {} from {}\n",
        release_info.tag, release_info.release_url
    ));

    // Check for install command in provider config
    if let Some(install_cmd) = payload.provider_config.get("install_command") {
        if let Some(cmd_str) = install_cmd.as_str() {
            // Substitute variables in the command with shell-escaped values
            // to prevent injection via untrusted wire-protocol fields.
            let cmd = cmd_str
                .replace("{version}", &shell_escape(&payload.to_version))
                .replace("{tag}", &shell_escape(&release_info.tag))
                .replace(
                    "{package_identifier}",
                    &shell_escape(&payload.package_identifier),
                );

            send_output(
                output_tx,
                &format!("Running install command: {cmd}"),
                OutputStreamType::Stdout,
            )
            .await;

            match run_command(&cmd, OutputStreamType::Stdout, output_tx).await {
                Ok(cmd_output) => {
                    output.push_str(&cmd_output);
                }
                Err(e) => {
                    return Err(report!(UpdateError::InstallFailed(e.to_string())));
                }
            }
        }
    } else {
        // Default behavior: just log that we would download and install
        send_output(
            output_tx,
            "No install_command configured, skipping automated installation",
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str("No install_command configured, skipping automated installation\n");
    }

    Ok(output)
}

/// Execute a Proxmox Helper Scripts update.
async fn execute_proxmox_helper_scripts_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<String> {
    let mut output = String::new();

    // Get the script URL from provider config
    let script_url = payload
        .provider_config
        .get("script_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| report!(UpdateError::MissingConfig("script_url".to_string())))?;

    send_output(
        output_tx,
        &format!("Running update script from {script_url}"),
        OutputStreamType::Stdout,
    )
    .await;
    output.push_str(&format!("Running update script from {script_url}\n"));

    // Run the helper script via bash, passing the URL as a positional argument
    // (`$1`) to avoid shell interpretation of the URL string.
    let (cmd_output, _exit_code) = run_command_exec(
        "bash",
        &[
            "-c".to_string(),
            "set -euo pipefail\ncurl -fsSL -- \"$1\" | bash -s -- --update".to_string(),
            "--".to_string(),
            script_url.to_string(),
        ],
        None,
        OutputStreamType::Stdout,
        output_tx,
    )
    .await
    .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))?;
    output.push_str(&cmd_output);

    Ok(output)
}

/// Execute a Docker Registry update.
async fn execute_docker_registry_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<String> {
    let mut output = String::new();

    let image = &payload.package_identifier;
    let tag = &payload.to_version;

    send_output(
        output_tx,
        &format!("Pulling Docker image {image}:{tag}"),
        OutputStreamType::Stdout,
    )
    .await;
    output.push_str(&format!("Pulling Docker image {image}:{tag}\n"));

    // Pull the new image using direct exec (no shell) to prevent injection
    // via crafted image names or tag values.
    let image_ref = format!("{image}:{tag}");
    let (cmd_output, _exit_code) = run_command_exec(
        "docker",
        &["pull".to_string(), image_ref],
        None,
        OutputStreamType::Stdout,
        output_tx,
    )
    .await
    .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))?;
    output.push_str(&cmd_output);

    // Check for restart command in provider config
    if let Some(restart_cmd) = payload.provider_config.get("restart_command")
        && let Some(cmd_str) = restart_cmd.as_str()
    {
        // Shell-escape all substituted values to prevent injection.
        let cmd = cmd_str
            .replace("{image}", &shell_escape(image))
            .replace("{tag}", &shell_escape(tag))
            .replace("{version}", &shell_escape(&payload.to_version));

        send_output(
            output_tx,
            &format!("Running restart command: {cmd}"),
            OutputStreamType::Stdout,
        )
        .await;

        match run_command(&cmd, OutputStreamType::Stdout, output_tx).await {
            Ok(cmd_output) => {
                output.push_str(&cmd_output);
            }
            Err(e) => {
                return Err(report!(UpdateError::InstallFailed(e.to_string())));
            }
        }
    }

    Ok(output)
}

/// Wrap a command with fail-early shell settings.
///
/// - **Bash**: `set -euo pipefail` (exit on error, undefined vars, pipe failures)
/// - **Sh**: `set -eu` (exit on error, undefined vars)
/// - **PowerShell**: `$ErrorActionPreference = 'Stop'`
fn wrap_command_for_shell(cmd: &str, shell: HookShell) -> String {
    match shell {
        HookShell::Bash => format!("set -euo pipefail\n{cmd}"),
        HookShell::Sh => format!("set -eu\n{cmd}"),
        HookShell::PowerShell => format!("$ErrorActionPreference = 'Stop'\n{cmd}"),
    }
}

/// Get the shell executable and arguments for a given shell type.
fn get_shell_args(shell: HookShell) -> (&'static str, &'static str) {
    match shell {
        HookShell::Bash => ("bash", "-c"),
        HookShell::Sh => ("sh", "-c"),
        HookShell::PowerShell => ("powershell", "-Command"),
    }
}

/// Execute a `HookCommand`, dispatching to shell or direct exec as appropriate.
async fn run_hook_command(
    hook_cmd: &HookCommand,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<(String, i32)> {
    match hook_cmd {
        HookCommand::Shell { command, shell } => {
            run_command_with_shell(command, *shell, stream_type, output_tx).await
        }
        HookCommand::Exec {
            program,
            args,
            working_dir,
        } => {
            run_command_exec(
                program,
                args,
                working_dir.as_deref(),
                stream_type,
                output_tx,
            )
            .await
        }
    }
}

/// Run a program directly with arguments (no shell interpretation).
///
/// Returns the accumulated output and exit code on success.
async fn run_command_exec(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<(String, i32)> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| report!(UpdateError::CommandSpawn(e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| report!(UpdateError::CaptureFailed("stdout".to_string())))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| report!(UpdateError::CaptureFailed("stderr".to_string())))?;

    let mut accumulated = String::new();

    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let output_tx_clone = output_tx.clone();
    let stdout_handle = tokio::spawn(async move {
        let mut lines = stdout_reader.lines();
        let mut output = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = output_tx_clone
                .send(UpdateOutputMessage {
                    output: line.clone(),
                    stream: stream_type,
                })
                .await;
            // Cap per-command buffer to prevent OOM from runaway output.
            if output.len() < MAX_OUTPUT_BYTES {
                output.push_str(&line);
                output.push('\n');
            }
        }
        output
    });

    let output_tx_clone = output_tx.clone();
    let stderr_handle = tokio::spawn(async move {
        let mut lines = stderr_reader.lines();
        let mut output = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = output_tx_clone
                .send(UpdateOutputMessage {
                    output: line.clone(),
                    stream: OutputStreamType::Stderr,
                })
                .await;
            if output.len() < MAX_OUTPUT_BYTES {
                output.push_str(&line);
                output.push('\n');
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
        .map_err(|e| report!(UpdateError::CommandWait(e)))?;

    let exit_code = status.code().unwrap_or(-1);

    if !status.success() {
        return Err(report!(UpdateError::CommandFailed(exit_code)));
    }

    Ok((accumulated, exit_code))
}

/// Run a command with the specified shell and fail-early settings.
///
/// Returns the accumulated output and exit code on success.
async fn run_command_with_shell(
    cmd: &str,
    shell: HookShell,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<(String, i32)> {
    let wrapped_cmd = wrap_command_for_shell(cmd, shell);
    let (shell_exec, shell_arg) = get_shell_args(shell);

    let mut child = Command::new(shell_exec)
        .arg(shell_arg)
        .arg(&wrapped_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| report!(UpdateError::CommandSpawn(e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| report!(UpdateError::CaptureFailed("stdout".to_string())))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| report!(UpdateError::CaptureFailed("stderr".to_string())))?;

    let mut accumulated = String::new();

    // Read stdout and stderr concurrently
    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let output_tx_clone = output_tx.clone();
    let stdout_handle = tokio::spawn(async move {
        let mut lines = stdout_reader.lines();
        let mut output = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = output_tx_clone
                .send(UpdateOutputMessage {
                    output: line.clone(),
                    stream: stream_type,
                })
                .await;
            if output.len() < MAX_OUTPUT_BYTES {
                output.push_str(&line);
                output.push('\n');
            }
        }
        output
    });

    let output_tx_clone = output_tx.clone();
    let stderr_handle = tokio::spawn(async move {
        let mut lines = stderr_reader.lines();
        let mut output = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = output_tx_clone
                .send(UpdateOutputMessage {
                    output: line.clone(),
                    stream: OutputStreamType::Stderr,
                })
                .await;
            if output.len() < MAX_OUTPUT_BYTES {
                output.push_str(&line);
                output.push('\n');
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
        .map_err(|e| report!(UpdateError::CommandWait(e)))?;

    let exit_code = status.code().unwrap_or(-1);

    if !status.success() {
        return Err(report!(UpdateError::CommandFailed(exit_code)));
    }

    Ok((accumulated, exit_code))
}

/// Run a shell command and stream output (legacy function for provider updates).
async fn run_command(
    cmd: &str,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<String> {
    let (output, _) =
        run_command_with_shell(cmd, HookShell::default(), stream_type, output_tx).await?;
    Ok(output)
}

/// Send an output message.
async fn send_output(
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    message: &str,
    stream: OutputStreamType,
) {
    let _ = output_tx
        .send(UpdateOutputMessage {
            output: message.to_string(),
            stream,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_payload() -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            package_identifier: "test-app".to_string(),
            to_version: "2.0.0".to_string(),
            provider_type: ProviderType::GithubReleases,
            provider_config: json!({}),
            pre_update_hooks: vec![],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: 60,
        }
    }

    // ── Shell wrapper tests ──────────────────────────────────────────────────

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

    #[tokio::test]
    async fn test_run_command_success() {
        let (tx, mut rx) = mpsc::channel(100);

        let result = run_command("echo 'hello world'", OutputStreamType::Stdout, &tx).await;

        assert!(result.is_ok());
        let output = result.expect("should succeed");
        assert!(output.contains("hello world"));

        // Drain the channel
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_run_command_failure() {
        let (tx, mut rx) = mpsc::channel(100);

        let result = run_command("exit 1", OutputStreamType::Stdout, &tx).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), UpdateError::CommandFailed(1)),
            "Expected CommandFailed(1), got: {err}"
        );

        // Drain the channel
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_execute_update_with_pre_hook() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hooks = vec![HookCommand::Shell {
            command: "echo 'pre-hook executed'".to_string(),
            shell: HookShell::Bash,
        }];
        payload.release_info = None;
        payload.provider_config = json!({});

        let result = execute_update(payload, tx).await;

        // Should complete (though the actual update may fail due to missing release_info)
        assert_eq!(result.result.update_history_id, uuid::Uuid::nil());

        // Drain the channel and check for pre-hook output
        rx.close();
        let mut found_pre_hook = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("pre-hook executed") {
                found_pre_hook = true;
            }
        }
        assert!(found_pre_hook);
    }

    #[tokio::test]
    async fn test_execute_update_pre_hook_failure() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hooks = vec![HookCommand::Shell {
            command: "exit 1".to_string(),
            shell: HookShell::Bash,
        }];

        let result = execute_update(payload, tx).await;

        assert_eq!(result.result.status, UpdateFinalStatus::Failed);
        assert!(result.result.error.is_some(), "Expected error but got None");
        let error_msg = result.result.error.as_ref().unwrap();
        assert!(
            error_msg.contains("Pre-update hook failed"),
            "Expected error to contain 'Pre-update hook failed', got: {error_msg}"
        );

        // Drain the channel
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_execute_update_with_sh_shell() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hooks = vec![HookCommand::Shell {
            command: "echo 'using sh shell'".to_string(),
            shell: HookShell::Sh,
        }];

        let result = execute_update(payload, tx).await;

        // Should complete (though the actual update may fail)
        assert_eq!(result.result.update_history_id, uuid::Uuid::nil());

        // Drain the channel and check for sh output
        rx.close();
        let mut found_output = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("using sh shell") {
                found_output = true;
            }
        }
        assert!(found_output);
    }

    #[tokio::test]
    async fn test_run_command_with_shell_success() {
        let (tx, mut rx) = mpsc::channel(100);

        let result = run_command_with_shell(
            "echo 'test'",
            HookShell::Bash,
            OutputStreamType::Stdout,
            &tx,
        )
        .await;

        assert!(result.is_ok());
        let (output, exit_code) = result.expect("should succeed");
        assert!(output.contains("test"));
        assert_eq!(exit_code, 0);

        // Drain the channel
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_run_command_with_shell_failure() {
        let (tx, mut rx) = mpsc::channel(100);

        let result =
            run_command_with_shell("exit 42", HookShell::Bash, OutputStreamType::Stdout, &tx).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), UpdateError::CommandFailed(42)),
            "Expected CommandFailed(42), got: {err}"
        );

        // Drain the channel
        rx.close();
        while rx.recv().await.is_some() {}
    }

    // ── Shell escape tests ──────────────────────────────────────────────────

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
    /// Uses `; echo MARKER` as payload — if interpreted, MARKER appears on a
    /// separate line. With proper escaping, the entire string is a single
    /// literal argument and MARKER only appears as part of it.
    #[tokio::test]
    async fn shell_escape_prevents_injection_in_bash() {
        let (tx, mut rx) = mpsc::channel(100);
        // This malicious value would produce two commands if unescaped:
        //   printf '%s' 2.0.0; echo MARKER
        // With proper escaping printf receives the literal string.
        let malicious = "2.0.0'; echo 'MARKER";
        let cmd = format!("printf '%s' {}", shell_escape(malicious));
        let result = run_command(&cmd, OutputStreamType::Stdout, &tx).await;
        assert!(result.is_ok());
        let output = result.expect("should succeed");
        // Output should be the literal malicious string (no separate MARKER line).
        assert_eq!(output.trim(), malicious);
        rx.close();
        while rx.recv().await.is_some() {}
    }

    // ── Bounded output tests ────────────────────────────────────────────────

    #[test]
    fn append_bounded_below_limit() {
        let mut buf = String::new();
        append_bounded(&mut buf, "hello", 100);
        assert_eq!(buf, "hello");
    }

    #[test]
    fn append_bounded_at_limit_truncates() {
        let mut buf = String::new();
        append_bounded(&mut buf, "abcde", 3);
        assert!(buf.starts_with("abc"));
        assert!(buf.contains(TRUNCATION_MARKER));
    }

    #[test]
    fn append_bounded_already_full_drops() {
        let mut buf = "x".repeat(100);
        append_bounded(&mut buf, "more data", 100);
        // Should not grow beyond 100
        assert_eq!(buf.len(), 100);
    }

    #[test]
    fn append_bounded_exact_fit() {
        let mut buf = String::new();
        append_bounded(&mut buf, "abc", 3);
        assert_eq!(buf, "abc");
        // Now it's full, next append should be a no-op
        append_bounded(&mut buf, "d", 3);
        assert_eq!(buf, "abc");
    }

    // ── Direct exec tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn run_command_exec_success() {
        let (tx, mut rx) = mpsc::channel(100);
        let result = run_command_exec(
            "echo",
            &["hello exec".to_string()],
            None,
            OutputStreamType::Stdout,
            &tx,
        )
        .await;
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
        let result = run_command_exec("false", &[], None, OutputStreamType::Stdout, &tx).await;
        assert!(result.is_err());
        rx.close();
        while rx.recv().await.is_some() {}
    }
}
