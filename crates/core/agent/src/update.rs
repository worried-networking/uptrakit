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
    ExecuteUpdatePayload, OutputStreamType, ProviderType, UpdateFinalStatus, UpdateResultPayload,
};

use crate::error::Error;

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

/// Default shell to use when not specified.
const DEFAULT_SHELL: &str = "bash";

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
    let shell = payload.shell.as_deref().unwrap_or(DEFAULT_SHELL);

    // Detect current version (from_version)
    let from_version = detect_current_version(&payload).await;

    let mut accumulated_output = String::new();
    let mut final_error: Option<String> = None;
    let mut final_status = UpdateFinalStatus::Completed;

    // Run with timeout
    let timeout_duration = std::time::Duration::from_secs(u64::from(payload.timeout_seconds));
    let execution_result = tokio::time::timeout(timeout_duration, async {
        // Run pre-update hooks
        if !payload.pre_update_commands.is_empty() {
            send_output(
                &output_tx,
                "[pre-hook] Starting pre-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for cmd in &payload.pre_update_commands {
                send_output(
                    &output_tx,
                    &format!("[pre-hook] Running: {cmd}"),
                    OutputStreamType::PreHook,
                )
                .await;

                match run_command_with_shell(cmd, shell, OutputStreamType::PreHook, &output_tx)
                    .await
                {
                    Ok((output, exit_code)) => {
                        accumulated_output.push_str(&output);
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
                accumulated_output.push_str(&output);
            }
            Err(e) => {
                return Err(Error::UpdateExecution(e.to_string()));
            }
        }

        // Run post-update hooks
        if !payload.post_update_commands.is_empty() {
            send_output(
                &output_tx,
                "[post-hook] Starting post-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for cmd in &payload.post_update_commands {
                send_output(
                    &output_tx,
                    &format!("[post-hook] Running: {cmd}"),
                    OutputStreamType::PostHook,
                )
                .await;

                match run_command_with_shell(cmd, shell, OutputStreamType::PostHook, &output_tx)
                    .await
                {
                    Ok((output, exit_code)) => {
                        accumulated_output.push_str(&output);
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
            // Substitute variables in the command
            let cmd = cmd_str
                .replace("{version}", &payload.to_version)
                .replace("{tag}", &release_info.tag)
                .replace("{package_identifier}", &payload.package_identifier);

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

    // Typically Proxmox helper scripts are run via bash
    let cmd = format!("bash -c \"$(curl -fsSL {script_url})\" -- --update");
    match run_command(&cmd, OutputStreamType::Stdout, output_tx).await {
        Ok(cmd_output) => {
            output.push_str(&cmd_output);
        }
        Err(e) => {
            return Err(report!(UpdateError::InstallFailed(e.to_string())));
        }
    }

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

    // Pull the new image
    let pull_cmd = format!("docker pull {image}:{tag}");
    match run_command(&pull_cmd, OutputStreamType::Stdout, output_tx).await {
        Ok(cmd_output) => {
            output.push_str(&cmd_output);
        }
        Err(e) => {
            return Err(report!(UpdateError::InstallFailed(e.to_string())));
        }
    }

    // Check for restart command in provider config
    if let Some(restart_cmd) = payload.provider_config.get("restart_command")
        && let Some(cmd_str) = restart_cmd.as_str()
    {
        let cmd = cmd_str
            .replace("{image}", image)
            .replace("{tag}", tag)
            .replace("{version}", &payload.to_version);

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
/// - **PowerShell** (future): `$ErrorActionPreference = 'Stop'`
fn wrap_command_for_shell(cmd: &str, shell: &str) -> String {
    match shell {
        "bash" => format!("set -euo pipefail\n{cmd}"),
        "sh" => format!("set -eu\n{cmd}"),
        "powershell" => format!("$ErrorActionPreference = 'Stop'\n{cmd}"),
        _ => cmd.to_string(),
    }
}

/// Get the shell executable and arguments for a given shell type.
fn get_shell_args(shell: &str) -> (&str, &str) {
    match shell {
        "bash" => ("bash", "-c"),
        "sh" => ("sh", "-c"),
        "powershell" => ("powershell", "-Command"),
        _ => ("sh", "-c"),
    }
}

/// Run a command with the specified shell and fail-early settings.
///
/// Returns the accumulated output and exit code on success.
async fn run_command_with_shell(
    cmd: &str,
    shell: &str,
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
            output.push_str(&line);
            output.push('\n');
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
            output.push_str(&line);
            output.push('\n');
        }
        output
    });

    let (stdout_output, stderr_output) = tokio::join!(stdout_handle, stderr_handle);

    accumulated.push_str(&stdout_output.unwrap_or_default());
    accumulated.push_str(&stderr_output.unwrap_or_default());

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
    let (output, _) = run_command_with_shell(cmd, DEFAULT_SHELL, stream_type, output_tx).await?;
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
            pre_update_commands: vec![],
            post_update_commands: vec![],
            release_info: None,
            timeout_seconds: 60,
            shell: None,
        }
    }

    // ── Shell wrapper tests ──────────────────────────────────────────────────

    #[test]
    fn wrap_command_for_bash() {
        let wrapped = wrap_command_for_shell("echo hello", "bash");
        assert!(wrapped.starts_with("set -euo pipefail\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn wrap_command_for_sh() {
        let wrapped = wrap_command_for_shell("echo hello", "sh");
        assert!(wrapped.starts_with("set -eu\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn wrap_command_for_powershell() {
        let wrapped = wrap_command_for_shell("echo hello", "powershell");
        assert!(wrapped.starts_with("$ErrorActionPreference = 'Stop'\n"));
        assert!(wrapped.ends_with("echo hello"));
    }

    #[test]
    fn wrap_command_for_unknown_shell() {
        let wrapped = wrap_command_for_shell("echo hello", "zsh");
        assert_eq!(wrapped, "echo hello");
    }

    #[test]
    fn get_shell_args_bash() {
        let (exec, arg) = get_shell_args("bash");
        assert_eq!(exec, "bash");
        assert_eq!(arg, "-c");
    }

    #[test]
    fn get_shell_args_sh() {
        let (exec, arg) = get_shell_args("sh");
        assert_eq!(exec, "sh");
        assert_eq!(arg, "-c");
    }

    #[test]
    fn get_shell_args_unknown() {
        let (exec, arg) = get_shell_args("unknown");
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
        payload.pre_update_commands = vec!["echo 'pre-hook executed'".to_string()];
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
        payload.pre_update_commands = vec!["exit 1".to_string()];

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
        payload.shell = Some("sh".to_string());
        payload.pre_update_commands = vec!["echo 'using sh shell'".to_string()];

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

        let result =
            run_command_with_shell("echo 'test'", "bash", OutputStreamType::Stdout, &tx).await;

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

        let result = run_command_with_shell("exit 42", "bash", OutputStreamType::Stdout, &tx).await;

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
}
