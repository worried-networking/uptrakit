//! Update execution module for Uptrakit agents.
//!
//! Handles the complete update flow:
//! 1. Receive `ExecuteUpdate` message
//! 2. Detect current version (`from_version`)
//! 3. Run pre-update commands sequentially, streaming output
//! 4. Execute actual update (dispatched through Provider Registry), streaming output
//! 5. Run post-update commands sequentially, streaming output
//! 6. Detect to_version post-update
//! 7. Return `UpdateExecutionResult` with final status and accumulated output
//!
//! ## Shell Execution
//!
//! Commands are executed with fail-early shell settings:
//! - **Bash**: `set -euo pipefail` (exit on error, undefined vars, pipe failures)
//! - **Sh**: `set -eu` (exit on error, undefined vars)

use std::sync::Arc;

use rootcause::prelude::*;
use thiserror::Error as ThisError;
use tokio::sync::mpsc;
use uptrakit_command::{CommandExecutor, UpdateOutputLine};
use uptrakit_internal_wire::{
    ExecuteUpdatePayload, HookCommand, OutputStreamType, UpdateFinalStatus, UpdateResultPayload,
};
use uptrakit_plugin_registry::PluginRegistry;

use crate::error::AgentCoreError;

/// Maximum accumulated output size (10 MB) to prevent OOM from runaway commands.
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Marker appended when output is truncated at the limit.
const TRUNCATION_MARKER: &str = "\n... [output truncated at 10 MB] ...\n";

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
    #[error("install command failed: {0}")]
    InstallFailed(String),

    #[error("hook execution failed: {0}")]
    HookFailed(String),
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
/// `UpdateOutput` messages to the controller.
///
/// The `executor` parameter controls where commands run — `LocalCommandExecutor`
/// for the regular agent, `SshCommandExecutor` for the SSH agent.
pub async fn execute_update(
    payload: ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
) -> UpdateExecutionResult {
    tracing::info!(software_item = %payload.software_item_name, "starting update");
    let update_history_id = payload.update_history_id;

    // Detect current version (from_version)
    let from_version = detect_current_version(&payload, Arc::clone(&executor)).await;
    tracing::debug!(from_version = ?from_version, "detected current version before update");

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
                        return Err(AgentCoreError::PreUpdateHookFailed(e.to_string()));
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

        tracing::debug!("executing provider update");
        match execute_provider_update(&payload, &output_tx, Arc::clone(&executor)).await {
            Ok(output) => {
                tracing::debug!("provider update returned");
                append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES);
            }
            Err(e) => {
                return Err(AgentCoreError::UpdateExecution(e.to_string()));
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
                        return Err(AgentCoreError::PostUpdateHookFailed(e.to_string()));
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
            tracing::info!(software_item = %payload.software_item_name, "update completed");
            send_output(
                &output_tx,
                "[update] Update completed successfully",
                OutputStreamType::System,
            )
            .await;
            // Detect new version after update
            detect_current_version(&payload, Arc::clone(&executor)).await
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

/// Detect the current version of a software item by delegating to the provider registry.
async fn detect_current_version(
    payload: &ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
) -> Option<String> {
    // The connection context has already been merged into payload.provider_config
    // by the caller before the update task was spawned.
    let outcome = crate::version_check::check_version(
        payload.provider_type.clone(),
        &payload.provider_config,
        &payload.package_identifier,
        executor,
        &crate::connection_context::ConnectionContext::default(),
    )
    .await;
    if let Some(e) = outcome.error {
        tracing::warn!(
            software_item = %payload.software_item_name,
            error = %e,
            "failed to detect current version"
        );
    }
    tracing::debug!(version = ?outcome.installed_version, "current version detected");
    outcome.installed_version
}

/// Execute the provider-specific update logic by dispatching through the Provider Registry.
async fn execute_provider_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    executor: Arc<dyn CommandExecutor>,
) -> UpdateResult<String> {
    let provider = PluginRegistry::create_provider(
        payload.provider_type.clone(),
        &payload.provider_config,
        executor,
    )
    .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))?;

    // Bridge provider output (UpdateOutputLine) -> agent output (UpdateOutputMessage)
    let (provider_tx, mut provider_rx) = mpsc::channel::<UpdateOutputLine>(100);
    let bridge_output_tx = output_tx.clone();
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = provider_rx.recv().await {
            let _ = bridge_output_tx
                .send(UpdateOutputMessage {
                    output: line.text,
                    stream: line.stream,
                })
                .await;
        }
    });

    let result = provider
        .execute_update(
            &payload.package_identifier,
            &payload.to_version,
            payload.release_info.as_ref(),
            &provider_tx,
        )
        .await
        .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())));

    // Drop the sender so the bridge task finishes
    drop(provider_tx);
    let _ = bridge_handle.await;

    result
}

/// Execute a `HookCommand`, dispatching to shell or direct exec as appropriate.
async fn run_hook_command(
    hook_cmd: &HookCommand,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<(String, i32)> {
    tracing::debug!(hook = ?hook_cmd, "running update hook");
    // Bridge provider output -> agent output
    let (provider_tx, mut provider_rx) = mpsc::channel::<UpdateOutputLine>(100);
    let bridge_output_tx = output_tx.clone();
    let bridge_stream_type = stream_type;
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = provider_rx.recv().await {
            let stream = match line.stream {
                OutputStreamType::Stdout => bridge_stream_type,
                _ => OutputStreamType::Stderr,
            };
            let _ = bridge_output_tx
                .send(UpdateOutputMessage {
                    output: line.text,
                    stream,
                })
                .await;
        }
    });

    let result = match hook_cmd {
        HookCommand::Shell { command, shell } => {
            uptrakit_command::run_command_with_shell(command, *shell, &provider_tx).await
        }
        HookCommand::Exec {
            program,
            args,
            working_dir,
        } => {
            uptrakit_command::run_command_exec(program, args, working_dir.as_deref(), &provider_tx)
                .await
        }
    };

    // Drop the sender so the bridge task finishes
    drop(provider_tx);
    let _ = bridge_handle.await;

    let result = result.map_err(|e| report!(UpdateError::HookFailed(e.to_string())));
    if let Ok((_, exit_code)) = &result {
        tracing::debug!(exit_code, "hook completed");
    }
    result
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
    use uptrakit_command::LocalCommandExecutor;
    use uptrakit_internal_wire::{HookShell, PluginType};

    fn test_payload() -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            host_machine_id: String::new(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            package_identifier: "test-app".to_string(),
            to_version: "2.0.0".to_string(),
            provider_type: PluginType::GithubReleases,
            provider_config: serde_json::json!({}),
            pre_update_hooks: vec![],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: 60,
        }
    }

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
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
        assert_eq!(buf.len(), 100);
    }

    #[test]
    fn append_bounded_exact_fit() {
        let mut buf = String::new();
        append_bounded(&mut buf, "abc", 3);
        assert_eq!(buf, "abc");
        append_bounded(&mut buf, "d", 3);
        assert_eq!(buf, "abc");
    }

    // ── Hook execution tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_update_with_pre_hook() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hooks = vec![HookCommand::Shell {
            command: "echo 'pre-hook executed'".to_string(),
            shell: HookShell::Bash,
        }];
        payload.release_info = None;
        payload.provider_config = serde_json::json!({});

        let result = execute_update(payload, test_executor(), tx).await;

        assert_eq!(result.result.update_history_id, uuid::Uuid::nil());

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

        let result = execute_update(payload, test_executor(), tx).await;

        assert_eq!(result.result.status, UpdateFinalStatus::Failed);
        assert!(result.result.error.is_some(), "Expected error but got None");
        let error_msg = result.result.error.as_ref().unwrap();
        assert!(
            error_msg.contains("Pre-update hook failed"),
            "Expected error to contain 'Pre-update hook failed', got: {error_msg}"
        );

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

        let result = execute_update(payload, test_executor(), tx).await;

        assert_eq!(result.result.update_history_id, uuid::Uuid::nil());

        rx.close();
        let mut found_output = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("using sh shell") {
                found_output = true;
            }
        }
        assert!(found_output);
    }
}
