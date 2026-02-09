//! Update execution module for the Uptrakit agent.
//!
//! Handles the complete update flow:
//! 1. Receive ExecuteUpdate message
//! 2. Send UpdateStarted (with detected from_version)
//! 3. Run pre-update commands sequentially, streaming output
//! 4. Execute actual update (dispatched through Provider Registry), streaming output
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

use rootcause::prelude::*;
use thiserror::Error as ThisError;
use tokio::sync::mpsc;
use uptrakit_internal_wire::{
    ExecuteUpdatePayload, HookCommand, HookShell, OutputStreamType, UpdateFinalStatus,
    UpdateResultPayload,
};
use uptrakit_provider_core::{ShellType, UpdateContext, UpdateOutputLine, UpdateOutputStream};
use uptrakit_provider_registry::ProviderRegistry;

use crate::error::Error;

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

/// Detect the current version of a software item by delegating to the provider registry.
async fn detect_current_version(payload: &ExecuteUpdatePayload) -> Option<String> {
    let (installed_version, error) = crate::version_check::check_version(
        payload.provider_type.clone(),
        &payload.package_identifier,
        &payload.provider_config,
    )
    .await;
    if let Some(e) = error {
        tracing::warn!(
            software_item = %payload.software_item_name,
            error = %e,
            "failed to detect current version"
        );
    }
    installed_version
}

/// Execute the provider-specific update logic by dispatching through the Provider Registry.
async fn execute_provider_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<String> {
    let provider = ProviderRegistry::create_local_provider(
        payload.provider_type.clone(),
        &payload.package_identifier,
        &payload.provider_config,
    )
    .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))?;

    let ctx = UpdateContext {
        to_version: payload.to_version.clone(),
        package_identifier: payload.package_identifier.clone(),
        provider_config: payload.provider_config.clone(),
        release_info: payload
            .release_info
            .as_ref()
            .map(|ri| uptrakit_provider_core::ReleaseInfo {
                tag: ri.tag.clone(),
                release_url: ri.release_url.clone(),
                assets: ri.assets.clone(),
            }),
    };

    // Bridge provider output (UpdateOutputLine) -> agent output (UpdateOutputMessage)
    let (provider_tx, mut provider_rx) = mpsc::channel::<UpdateOutputLine>(100);
    let bridge_output_tx = output_tx.clone();
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = provider_rx.recv().await {
            let stream = match line.stream {
                UpdateOutputStream::Stdout => OutputStreamType::Stdout,
                UpdateOutputStream::Stderr => OutputStreamType::Stderr,
            };
            let _ = bridge_output_tx
                .send(UpdateOutputMessage {
                    output: line.text,
                    stream,
                })
                .await;
        }
    });

    let result = provider
        .execute_update(&ctx, &provider_tx)
        .await
        .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())));

    // Drop the sender so the bridge task finishes
    drop(provider_tx);
    let _ = bridge_handle.await;

    result
}

/// Map a wire `HookShell` to a provider-core `ShellType`.
fn hook_shell_to_shell_type(shell: HookShell) -> ShellType {
    match shell {
        HookShell::Bash => ShellType::Bash,
        HookShell::Sh => ShellType::Sh,
        HookShell::PowerShell => ShellType::PowerShell,
    }
}

/// Execute a `HookCommand`, dispatching to shell or direct exec as appropriate.
async fn run_hook_command(
    hook_cmd: &HookCommand,
    stream_type: OutputStreamType,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> UpdateResult<(String, i32)> {
    // Bridge provider output -> agent output
    let (provider_tx, mut provider_rx) = mpsc::channel::<UpdateOutputLine>(100);
    let bridge_output_tx = output_tx.clone();
    let bridge_stream_type = stream_type;
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = provider_rx.recv().await {
            let stream = match line.stream {
                UpdateOutputStream::Stdout => bridge_stream_type,
                UpdateOutputStream::Stderr => OutputStreamType::Stderr,
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
            uptrakit_provider_core::command::run_command_with_shell(
                command,
                hook_shell_to_shell_type(*shell),
                &provider_tx,
            )
            .await
        }
        HookCommand::Exec {
            program,
            args,
            working_dir,
        } => {
            uptrakit_provider_core::command::run_command_exec(
                program,
                args,
                working_dir.as_deref(),
                &provider_tx,
            )
            .await
        }
    };

    // Drop the sender so the bridge task finishes
    drop(provider_tx);
    let _ = bridge_handle.await;

    result.map_err(|e| report!(UpdateError::HookFailed(e.to_string())))
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
    use uptrakit_internal_wire::ProviderType;

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
        payload.provider_config = json!({});

        let result = execute_update(payload, tx).await;

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

        let result = execute_update(payload, tx).await;

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

        let result = execute_update(payload, tx).await;

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

    // ── detect_current_version tests ────────────────────────────────────────

    #[tokio::test]
    async fn detect_current_version_delegates_to_provider() {
        let payload = ExecuteUpdatePayload {
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            package_identifier: "octocat/hello-world".to_string(),
            to_version: "2.0.0".to_string(),
            provider_type: ProviderType::GithubReleases,
            provider_config: json!({"owner": "octocat", "repo": "hello-world"}),
            pre_update_hooks: vec![],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: 60,
        };
        // The GitHub stub provider returns None for installed version.
        let result = detect_current_version(&payload).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_current_version_with_invalid_config_returns_none() {
        let payload = ExecuteUpdatePayload {
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            package_identifier: "test".to_string(),
            to_version: "2.0.0".to_string(),
            provider_type: ProviderType::GithubReleases,
            provider_config: json!({"invalid": "config"}),
            pre_update_hooks: vec![],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: 60,
        };
        // Invalid config should log a warning and return None.
        let result = detect_current_version(&payload).await;
        assert!(result.is_none());
    }

    // ── Hook shell mapping tests ────────────────────────────────────────────

    #[test]
    fn hook_shell_maps_correctly() {
        assert_eq!(hook_shell_to_shell_type(HookShell::Bash), ShellType::Bash);
        assert_eq!(hook_shell_to_shell_type(HookShell::Sh), ShellType::Sh);
        assert_eq!(
            hook_shell_to_shell_type(HookShell::PowerShell),
            ShellType::PowerShell
        );
    }
}
