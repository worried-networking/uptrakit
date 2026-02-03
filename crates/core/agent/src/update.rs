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

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use uptrakit_internal_wire::{
    ExecuteUpdatePayload, OutputStreamType, UpdateFinalStatus, UpdateResultPayload,
};

use crate::error::Error;

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
    let update_history_id = payload.update_history_id.clone();

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
                "[system] Running pre-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for cmd in &payload.pre_update_commands {
                match run_command(cmd, OutputStreamType::PreHook, &output_tx).await {
                    Ok(output) => {
                        accumulated_output.push_str(&output);
                    }
                    Err(e) => {
                        let error_msg = format!("Pre-update hook failed: {e}");
                        send_output(&output_tx, &error_msg, OutputStreamType::System).await;
                        return Err(Error::PreUpdateHookFailed(e));
                    }
                }
            }
        }

        // Execute actual update based on provider type
        send_output(
            &output_tx,
            &format!(
                "[system] Executing update to version {}...",
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
                return Err(Error::UpdateExecution(e));
            }
        }

        // Run post-update hooks
        if !payload.post_update_commands.is_empty() {
            send_output(
                &output_tx,
                "[system] Running post-update hooks...",
                OutputStreamType::System,
            )
            .await;

            for cmd in &payload.post_update_commands {
                match run_command(cmd, OutputStreamType::PostHook, &output_tx).await {
                    Ok(output) => {
                        accumulated_output.push_str(&output);
                    }
                    Err(e) => {
                        let error_msg = format!("Post-update hook failed: {e}");
                        send_output(&output_tx, &error_msg, OutputStreamType::System).await;
                        return Err(Error::PostUpdateHookFailed(e));
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
                "[system] Update completed successfully",
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
                    "[system] Update timed out after {} seconds",
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
) -> Result<String, String> {
    use uptrakit_internal_wire::UpdateProviderType;

    match payload.provider_type {
        UpdateProviderType::GithubReleases => {
            execute_github_releases_update(payload, output_tx).await
        }
        UpdateProviderType::ProxmoxHelperScripts => {
            execute_proxmox_helper_scripts_update(payload, output_tx).await
        }
        UpdateProviderType::DockerRegistry => {
            execute_docker_registry_update(payload, output_tx).await
        }
    }
}

/// Execute a GitHub Releases update.
async fn execute_github_releases_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> Result<String, String> {
    let mut output = String::new();

    // Extract release info
    let Some(release_info) = &payload.release_info else {
        return Err("No release info provided for GitHub Releases update".to_string());
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
                    return Err(format!("Install command failed: {e}"));
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
) -> Result<String, String> {
    let mut output = String::new();

    // Get the script URL from provider config
    let script_url = payload
        .provider_config
        .get("script_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No script_url in provider config".to_string())?;

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
            return Err(format!("Update script failed: {e}"));
        }
    }

    Ok(output)
}

/// Execute a Docker Registry update.
async fn execute_docker_registry_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> Result<String, String> {
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
            return Err(format!("Docker pull failed: {e}"));
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
                return Err(format!("Restart command failed: {e}"));
            }
        }
    }

    Ok(output)
}

/// Run a shell command and stream output.
async fn run_command(
    cmd: &str,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> Result<String, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn command: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture stderr".to_string())?;

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
        .map_err(|e| format!("Failed to wait for command: {e}"))?;

    if !status.success() {
        let exit_code = status.code().unwrap_or(-1);
        return Err(format!("Command exited with code {exit_code}"));
    }

    Ok(accumulated)
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
    use uptrakit_internal_wire::UpdateProviderType;

    fn test_payload() -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            update_history_id: "test-id".to_string(),
            software_item_id: "item-id".to_string(),
            software_item_name: "Test App".to_string(),
            package_identifier: "test-app".to_string(),
            to_version: "2.0.0".to_string(),
            provider_type: UpdateProviderType::GithubReleases,
            provider_config: json!({}),
            pre_update_commands: vec![],
            post_update_commands: vec![],
            release_info: None,
            timeout_seconds: 60,
        }
    }

    #[tokio::test]
    async fn test_run_command_success() {
        let (tx, mut rx) = mpsc::channel(100);

        let result = run_command("echo 'hello world'", OutputStreamType::Stdout, &tx).await;

        assert!(result.is_ok());
        let output = result.unwrap();
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
        assert!(err.contains("exited with code 1"));

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
        assert_eq!(result.result.update_history_id, "test-id");

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
}
