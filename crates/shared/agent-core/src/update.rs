//! Update execution module for Uptrakit agents.
//!
//! Handles the complete update flow:
//! 1. Receive `ExecuteUpdate` message
//! 2. Detect current version (`from_version`)
//! 3. Run pre-update lifecycle hook plugins (ordered by assignment)
//! 4. Execute actual update (dispatched through Plugin Registry)
//! 5. Run post-update lifecycle hook plugins (always, even on failure)
//! 6. Detect to_version post-update
//! 7. Return `UpdateExecutionResult` with final status and accumulated output

use std::sync::Arc;

use rootcause::prelude::*;
use thiserror::Error as ThisError;
use tokio::sync::mpsc;
use uptrakit_command::{CommandExecutor, UpdateOutputLine};
use uptrakit_internal_wire::{
    AttestationStatus, ExecuteUpdatePayload, OutputStreamType, PluginAssignment, ReleaseInfo,
    UpdateFinalStatus, UpdateResultPayload,
};
use uptrakit_plugin_infrastructure_core::{
    HostCapabilities, UpdateLifecycleContext, construct_host_runtime,
};
use uptrakit_plugin_infrastructure_registry::get_descriptor;

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

/// Run the update pipeline: pre-hook plugins → plugin execution → post-hook plugins.
///
/// The caller wraps this in [`tokio::time::timeout`] so cancellation
/// (`drop`) releases the `&mut accumulated_output` borrow cleanly.
async fn execute_update_pipeline(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    executor: Arc<dyn CommandExecutor>,
    accumulated_output: &mut String,
) -> std::result::Result<(), AgentCoreError> {
    // Run pre-update lifecycle hook plugins
    let lifecycle_ctx = UpdateLifecycleContext::for_pre_hook(
        &payload.execute_update_plugin.package_identifier,
        &payload.to_version,
        None, // from_version not yet available at this stage
        payload.release_info.clone(),
    );
    run_pre_hook_plugins(
        &payload.pre_update_hook_plugins,
        &lifecycle_ctx,
        Arc::clone(&executor),
        output_tx,
        accumulated_output,
    )
    .await?;

    // Attestation gate — abort if policy requires a verified attestation
    // and none was found.
    tracing::debug!("checking attestation gate");
    check_attestation_gate(payload.release_info.as_ref(), output_tx).await?;
    tracing::debug!("attestation gate passed");

    // Execute actual update based on plugin type
    send_output(
        output_tx,
        &format!(
            "[update] Executing update to version {}...",
            payload.to_version
        ),
        OutputStreamType::System,
    )
    .await;

    tracing::debug!("executing plugin update");
    let update_succeeded =
        match execute_plugin_update(payload, output_tx, Arc::clone(&executor)).await {
            Ok(output) => {
                tracing::debug!("plugin update returned successfully");
                append_bounded(accumulated_output, &output, MAX_OUTPUT_BYTES);
                true
            }
            Err(e) => {
                let error_msg = e.to_string();
                tracing::warn!(
                    software_item = %payload.software_item_name,
                    error = %error_msg,
                    "plugin update command failed"
                );
                let formatted = format!("[error] {error_msg}\n");
                send_output(
                    output_tx,
                    &format!("[error] {error_msg}"),
                    OutputStreamType::Stderr,
                )
                .await;
                append_bounded(accumulated_output, &formatted, MAX_OUTPUT_BYTES);
                false
            }
        };

    // Run post-update lifecycle hook plugins (always, even on failure)
    let post_ctx = UpdateLifecycleContext::for_post_hook(
        &payload.execute_update_plugin.package_identifier,
        &payload.to_version,
        None,
        payload.release_info.clone(),
        update_succeeded,
    );
    run_post_hook_plugins(
        &payload.post_update_hook_plugins,
        &post_ctx,
        executor,
        output_tx,
        accumulated_output,
    )
    .await;

    if !update_succeeded {
        return Err(AgentCoreError::UpdateExecution(
            "plugin update command failed".to_string(),
        ));
    }

    Ok(())
}

/// Execute an update based on the payload.
///
/// This function runs the complete update flow and sends output lines through
/// the provided channel. The channel receiver should forward these as
/// `UpdateOutput` messages to the controller.
///
/// The `executor` parameter controls where commands run — `LocalCommandExecutor`
/// for the regular agent, `SshCommandExecutor` for the SSH agent.
#[tracing::instrument(skip_all, fields(software_item = %payload.software_item_name, update_history_id = %payload.update_history_id))]
pub async fn execute_update(
    payload: ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
) -> UpdateExecutionResult {
    tracing::info!(software_item = %payload.software_item_name, "starting update");
    let update_history_id = payload.update_history_id;
    // Copy before the payload reference is used inside the timeout future.
    let timeout_duration = payload.timeout;

    // Detect current version (from_version)
    let from_version = detect_current_version(&payload, Arc::clone(&executor)).await;
    tracing::debug!(from_version = ?from_version, "detected current version before update");

    let mut accumulated_output = String::new();
    let mut final_error: Option<String> = None;
    let mut final_status = UpdateFinalStatus::Completed;

    // Run with timeout — the pipeline borrows `accumulated_output` mutably;
    // on cancellation (timeout) the borrow is released before the match below.
    let execution_result = tokio::time::timeout(
        timeout_duration,
        execute_update_pipeline(
            &payload,
            &output_tx,
            Arc::clone(&executor),
            &mut accumulated_output,
        ),
    )
    .await;

    // Handle timeout or execution result
    let to_version = match execution_result {
        Ok(Ok(())) => {
            tracing::info!(software_item = %payload.software_item_name, "update completed successfully");
            send_output(
                &output_tx,
                "[update] Update completed successfully",
                OutputStreamType::System,
            )
            .await;
            // Detect new version after update
            let to_version = detect_current_version(&payload, executor).await;
            tracing::debug!(to_version = ?to_version, "post-update version detected");
            to_version
        }
        Ok(Err(e)) => {
            // The error was already logged and appended to accumulated_output in the
            // execute_update_pipeline error arm above; here we only set the final state.
            final_status = UpdateFinalStatus::Failed;
            final_error = Some(e.to_string());
            None
        }
        Err(_) => {
            let timeout_msg = format!("Update timed out after {}s", timeout_duration.as_secs());
            tracing::warn!(
                software_item = %payload.software_item_name,
                timeout = ?timeout_duration,
                "update timed out"
            );
            let formatted = format!("[error] {timeout_msg}\n");
            send_output(
                &output_tx,
                &format!("[update] {timeout_msg}"),
                OutputStreamType::System,
            )
            .await;
            append_bounded(&mut accumulated_output, &formatted, MAX_OUTPUT_BYTES);
            final_status = UpdateFinalStatus::Failed;
            final_error = Some(timeout_msg);
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

/// Detect the current version of a software item by delegating to the plugin registry.
///
/// Uses the `detect_version_plugin` from the payload if available.
#[tracing::instrument(skip_all, fields(software_item = %payload.software_item_name))]
async fn detect_current_version(
    payload: &ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
) -> Option<String> {
    // The connection context has already been merged into the plugin config
    // by the caller before the update task was spawned.
    let detect_assignment = payload.detect_version_plugin.as_ref();
    let outcome = crate::version_check::check_version(
        detect_assignment,
        None,
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

/// Execute the plugin-specific update logic.
///
/// The plugin is obtained via descriptor-based creation from `get_descriptor()`.
/// The `UpdateExecutor` role slot is used to create the update executor directly.
#[tracing::instrument(skip_all, fields(plugin_type = %payload.execute_update_plugin.plugin_type))]
async fn execute_plugin_update(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    executor: Arc<dyn CommandExecutor>,
) -> UpdateResult<String> {
    let eu = &payload.execute_update_plugin;
    let runtime = construct_host_runtime(executor, HostCapabilities::default());

    let desc = get_descriptor(eu.plugin_type.as_str()).ok_or_else(|| {
        report!(UpdateError::InstallFailed(format!(
            "unknown plugin type: {}",
            eu.plugin_type
        )))
    })?;

    let slot = desc.roles.update_executor.as_ref().ok_or_else(|| {
        report!(UpdateError::InstallFailed(format!(
            "plugin {} does not implement UpdateExecutorPlugin",
            eu.plugin_type
        )))
    })?;

    let update_executor = (slot.create)(&eu.config, runtime)
        .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))?;

    let (plugin_tx, bridge_handle) = make_output_bridge(output_tx);
    let update_result = update_executor
        .execute_update(
            &eu.package_identifier,
            &payload.to_version,
            payload.release_info.as_ref(),
            &plugin_tx,
        )
        .await
        .map_err(|e| {
            use uptrakit_plugin_infrastructure_core::PluginError;
            let msg = match e.current_context() {
                PluginError::InstallFailed(s) => s.clone(),
                other => other.to_string(),
            };
            report!(UpdateError::InstallFailed(msg))
        });
    drop(plugin_tx);
    let _ = bridge_handle.await;

    update_result
}

/// Bridge plugin output (`UpdateOutputLine`) to agent output (`UpdateOutputMessage`).
///
/// Returns the sender for the plugin side and a join handle for the bridge task.
fn make_output_bridge(
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> (mpsc::Sender<UpdateOutputLine>, tokio::task::JoinHandle<()>) {
    let (plugin_tx, mut plugin_rx) = mpsc::channel::<UpdateOutputLine>(100);
    let bridge_output_tx = output_tx.clone();
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = plugin_rx.recv().await {
            let _ = bridge_output_tx
                .send(UpdateOutputMessage {
                    output: line.text,
                    stream: line.stream,
                })
                .await;
        }
    });
    (plugin_tx, bridge_handle)
}

/// Run pre-update lifecycle hook plugins in order.
///
/// Each plugin is instantiated, and its `execute_pre_hook()` is called.
/// If any hook returns `should_proceed = false`, the update is aborted.
async fn run_pre_hook_plugins(
    plugins: &[PluginAssignment],
    ctx: &UpdateLifecycleContext,
    executor: Arc<dyn CommandExecutor>,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    accumulated_output: &mut String,
) -> Result<(), AgentCoreError> {
    if plugins.is_empty() {
        return Ok(());
    }
    tracing::info!(
        hook_count = plugins.len(),
        "executing pre-update lifecycle hook plugins"
    );
    send_output(
        output_tx,
        "[pre-hook] Starting pre-update hook plugins...",
        OutputStreamType::System,
    )
    .await;

    for assignment in plugins {
        tracing::info!(
            plugin_type = %assignment.plugin_type,
            "running pre-update hook plugin"
        );
        send_output(
            output_tx,
            &format!("[pre-hook] Running plugin: {}", assignment.plugin_type),
            OutputStreamType::PreHook,
        )
        .await;

        let runtime = construct_host_runtime(Arc::clone(&executor), HostCapabilities::default());

        let desc = get_descriptor(assignment.plugin_type.as_str()).ok_or_else(|| {
            AgentCoreError::PreUpdateHookFailed(format!(
                "unknown plugin type: {}",
                assignment.plugin_type
            ))
        })?;

        let slot = desc.roles.lifecycle_hook.as_ref().ok_or_else(|| {
            AgentCoreError::PreUpdateHookFailed(format!(
                "plugin {} does not implement UpdateLifecyclePlugin",
                assignment.plugin_type
            ))
        })?;

        let lifecycle = (slot.create)(&assignment.config, runtime).map_err(|e| {
            AgentCoreError::PreUpdateHookFailed(format!(
                "failed to create hook plugin {}: {e}",
                assignment.plugin_type
            ))
        })?;

        let (plugin_tx, bridge_handle) = make_output_bridge(output_tx);
        let result = lifecycle.execute_pre_hook(ctx, &plugin_tx).await;
        drop(plugin_tx);
        let _ = bridge_handle.await;

        match result {
            Ok(pre_result) => {
                if !pre_result.should_proceed {
                    let reason = pre_result.abort_reason.unwrap_or_else(|| {
                        format!(
                            "pre-update hook plugin {} aborted the update",
                            assignment.plugin_type
                        )
                    });
                    tracing::warn!(reason, "pre-update hook plugin aborted the update");
                    let msg = format!("[pre-hook] Aborted: {reason}");
                    send_output(output_tx, &msg, OutputStreamType::PreHook).await;
                    append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
                    return Err(AgentCoreError::PreUpdateHookFailed(reason));
                }
                send_output(
                    output_tx,
                    &format!("[pre-hook] Plugin {} completed", assignment.plugin_type),
                    OutputStreamType::PreHook,
                )
                .await;
            }
            Err(e) => {
                let error_msg = format!(
                    "pre-update hook plugin {} failed: {e}",
                    assignment.plugin_type
                );
                send_output(
                    output_tx,
                    &format!("[pre-hook] Failed: {error_msg}"),
                    OutputStreamType::PreHook,
                )
                .await;
                return Err(AgentCoreError::PreUpdateHookFailed(error_msg));
            }
        }
    }

    Ok(())
}

/// Run post-update lifecycle hook plugins in order.
///
/// Each plugin is instantiated, and its `execute_post_hook()` is called.
/// Errors are logged but non-fatal — all plugins are always attempted.
async fn run_post_hook_plugins(
    plugins: &[PluginAssignment],
    ctx: &UpdateLifecycleContext,
    executor: Arc<dyn CommandExecutor>,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    accumulated_output: &mut String,
) {
    if plugins.is_empty() {
        return;
    }
    tracing::info!(
        hook_count = plugins.len(),
        "executing post-update lifecycle hook plugins"
    );
    send_output(
        output_tx,
        "[post-hook] Starting post-update hook plugins...",
        OutputStreamType::System,
    )
    .await;

    for assignment in plugins {
        tracing::info!(
            plugin_type = %assignment.plugin_type,
            "running post-update hook plugin"
        );
        send_output(
            output_tx,
            &format!("[post-hook] Running plugin: {}", assignment.plugin_type),
            OutputStreamType::PostHook,
        )
        .await;

        let runtime = construct_host_runtime(Arc::clone(&executor), HostCapabilities::default());

        let Some(desc) = get_descriptor(assignment.plugin_type.as_str()) else {
            let msg = format!("unknown plugin type: {}", assignment.plugin_type);
            tracing::warn!(msg, "skipping post-update hook plugin");
            send_output(
                output_tx,
                &format!("[post-hook] Skipped (failed to create): {msg}"),
                OutputStreamType::PostHook,
            )
            .await;
            append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
            continue;
        };

        let Some(slot) = desc.roles.lifecycle_hook.as_ref() else {
            tracing::warn!(
                plugin_type = %assignment.plugin_type,
                "plugin does not implement UpdateLifecyclePlugin; skipping"
            );
            continue;
        };

        let lifecycle = match (slot.create)(&assignment.config, runtime) {
            Ok(lc) => lc,
            Err(e) => {
                let msg = format!(
                    "failed to create post-hook plugin {}: {e}",
                    assignment.plugin_type
                );
                tracing::warn!(msg, "skipping post-update hook plugin");
                send_output(
                    output_tx,
                    &format!("[post-hook] Skipped (failed to create): {msg}"),
                    OutputStreamType::PostHook,
                )
                .await;
                append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
                continue;
            }
        };

        let (plugin_tx, bridge_handle) = make_output_bridge(output_tx);
        let result = lifecycle.execute_post_hook(ctx, &plugin_tx).await;
        drop(plugin_tx);
        let _ = bridge_handle.await;

        match result {
            Ok(()) => {
                send_output(
                    output_tx,
                    &format!("[post-hook] Plugin {} completed", assignment.plugin_type),
                    OutputStreamType::PostHook,
                )
                .await;
            }
            Err(e) => {
                let msg = format!(
                    "post-update hook plugin {} failed (non-fatal): {e}",
                    assignment.plugin_type
                );
                tracing::warn!(msg);
                send_output(
                    output_tx,
                    &format!("[post-hook] Warning: {msg}"),
                    OutputStreamType::PostHook,
                )
                .await;
                append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
            }
        }
    }
}

/// Run pre-update lifecycle hook plugins for batch operations.
///
/// Unlike single-update hooks, batch hooks do not stream output.
/// Returns error on first failure (aborts the batch).
pub(crate) async fn run_batch_pre_hook_plugins(
    plugins: &[PluginAssignment],
    ctx: &UpdateLifecycleContext,
    executor: Arc<dyn CommandExecutor>,
) -> UpdateResult<()> {
    for assignment in plugins {
        let runtime = construct_host_runtime(Arc::clone(&executor), HostCapabilities::default());

        let desc = get_descriptor(assignment.plugin_type.as_str()).ok_or_else(|| {
            report!(UpdateError::HookFailed(format!(
                "unknown plugin type: {}",
                assignment.plugin_type
            )))
        })?;

        let slot = desc.roles.lifecycle_hook.as_ref().ok_or_else(|| {
            report!(UpdateError::HookFailed(format!(
                "plugin {} does not implement UpdateLifecyclePlugin",
                assignment.plugin_type
            )))
        })?;

        let lifecycle = (slot.create)(&assignment.config, runtime).map_err(|e| {
            report!(UpdateError::HookFailed(format!(
                "failed to create hook plugin {}: {e}",
                assignment.plugin_type
            )))
        })?;

        let (plugin_tx, mut plugin_rx) = mpsc::channel::<UpdateOutputLine>(100);
        let drain_handle = tokio::spawn(async move { while plugin_rx.recv().await.is_some() {} });

        let result = lifecycle.execute_pre_hook(ctx, &plugin_tx).await;
        drop(plugin_tx);
        let _ = drain_handle.await;

        let pre_result = result.map_err(|e| {
            report!(UpdateError::HookFailed(format!(
                "pre-update hook plugin {} failed: {e}",
                assignment.plugin_type
            )))
        })?;

        if !pre_result.should_proceed {
            let reason = pre_result.abort_reason.unwrap_or_else(|| {
                format!(
                    "pre-update hook plugin {} aborted the batch",
                    assignment.plugin_type
                )
            });
            return Err(report!(UpdateError::HookFailed(reason)));
        }
    }
    Ok(())
}

/// Run post-update lifecycle hook plugins for batch operations.
///
/// Errors are logged but non-fatal.
pub(crate) async fn run_batch_post_hook_plugins(
    plugins: &[PluginAssignment],
    ctx: &UpdateLifecycleContext,
    executor: Arc<dyn CommandExecutor>,
) {
    for assignment in plugins {
        let runtime = construct_host_runtime(Arc::clone(&executor), HostCapabilities::default());

        let Some(desc) = get_descriptor(assignment.plugin_type.as_str()) else {
            tracing::warn!(
                plugin_type = %assignment.plugin_type,
                "unknown plugin type for post-hook; skipping"
            );
            continue;
        };

        let Some(slot) = desc.roles.lifecycle_hook.as_ref() else {
            continue;
        };

        let lifecycle = match (slot.create)(&assignment.config, runtime) {
            Ok(lc) => lc,
            Err(e) => {
                tracing::warn!(
                    plugin_type = %assignment.plugin_type,
                    error = %e,
                    "failed to create post-hook plugin for batch; skipping"
                );
                continue;
            }
        };

        let (plugin_tx, mut plugin_rx) = mpsc::channel::<UpdateOutputLine>(100);
        let drain_handle = tokio::spawn(async move { while plugin_rx.recv().await.is_some() {} });

        let result = lifecycle.execute_post_hook(ctx, &plugin_tx).await;
        drop(plugin_tx);
        let _ = drain_handle.await;

        if let Err(e) = result {
            tracing::warn!(
                plugin_type = %assignment.plugin_type,
                error = %e,
                "batch post-update hook plugin failed (non-fatal)"
            );
        }
    }
}

/// Send an output message.
async fn send_output(
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    message: &str,
    stream: OutputStreamType,
) {
    let _ = output_tx
        .send(UpdateOutputMessage {
            output: format!("{message}\n"),
            stream,
        })
        .await;
}

// ---------------------------------------------------------------------------
// Attestation gate
// ---------------------------------------------------------------------------

/// Parse `owner` and `repo` from a GitHub HTML release URL.
///
/// Accepts `https://github.com/{owner}/{repo}/releases/...` and returns
/// `Some((owner, repo))`, or `None` for any other URL format.
fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let path = url.strip_prefix("https://github.com/")?;
    let mut parts = path.splitn(3, '/');
    let owner = parts.next().filter(|s| !s.is_empty())?.to_string();
    let repo = parts.next().filter(|s| !s.is_empty())?.to_string();
    Some((owner, repo))
}

/// Query the GitHub Attestations API for the given asset SHA-256 digest.
///
/// Returns:
/// - [`AttestationStatus::Verified`] if one or more attestations are found.
/// - [`AttestationStatus::NotFound`] if the API returns 404 or an empty list.
/// - [`AttestationStatus::Unverified`] on any network or parse error.
#[tracing::instrument(skip_all, fields(%owner, %repo))]
async fn independently_verify_attestation(
    owner: &str,
    repo: &str,
    digest_hex: &str,
) -> AttestationStatus {
    #[derive(serde::Deserialize)]
    struct ApiResponse {
        #[serde(default)]
        attestations: Vec<serde_json::Value>,
    }

    let url =
        format!("https://api.github.com/repos/{owner}/{repo}/attestations/sha256:{digest_hex}");

    let client = match reqwest::Client::builder()
        .user_agent(concat!("uptrakit-agent/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to build reqwest client for attestation check");
            return AttestationStatus::Unverified;
        }
    };

    let response = match client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "attestation API request failed");
            return AttestationStatus::Unverified;
        }
    };

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return AttestationStatus::NotFound;
    }

    if !response.status().is_success() {
        tracing::warn!(
            status = %response.status(),
            "attestation API returned unexpected status"
        );
        return AttestationStatus::Unverified;
    }

    match response.json::<ApiResponse>().await {
        Ok(body) if !body.attestations.is_empty() => AttestationStatus::Verified,
        Ok(_) => AttestationStatus::NotFound,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse attestation API response");
            AttestationStatus::Unverified
        }
    }
}

/// Pre-install attestation gate.
///
/// Checks the GitHub Actions attestation for the release before allowing the
/// update to proceed. The gate is skipped when:
///
/// - `release_info` is `None` (no GitHub release source).
/// - The release URL does not parse as a `https://github.com/{owner}/{repo}/…`
///   URL (non-GitHub update paths are unaffected).
/// - No asset with a known `sha256_digest` is available for re-verification.
///
/// When the controller already set `attestation_status = Verified`, the agent
/// trusts that verdict and returns `Ok` immediately without a live API call.
///
/// Otherwise, the agent independently re-queries the GitHub Attestations API
/// using the first asset's `sha256_digest`. The result governs whether to
/// block (`require_attestation = true`) or warn.
async fn check_attestation_gate(
    release_info: Option<&ReleaseInfo>,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
) -> std::result::Result<(), AgentCoreError> {
    let Some(ri) = release_info else {
        return Ok(());
    };

    let Some((owner, repo)) = parse_github_owner_repo(&ri.release_url) else {
        return Ok(());
    };

    // Trust the controller's Verified verdict — already confirmed at fetch time.
    if ri.attestation_status == Some(AttestationStatus::Verified) {
        send_output(
            output_tx,
            &format!("[attestation] GitHub Actions attestation verified for {owner}/{repo}"),
            OutputStreamType::System,
        )
        .await;
        return Ok(());
    }

    // Independent re-verify using an asset SHA-256 digest.
    let digest = ri.assets.iter().find_map(|a| a.sha256_digest.as_deref());
    let Some(digest) = digest else {
        send_output(
            output_tx,
            "[attestation] No asset digest available for independent attestation check; proceeding",
            OutputStreamType::System,
        )
        .await;
        return Ok(());
    };

    let verified_status = independently_verify_attestation(&owner, &repo, digest).await;

    match verified_status {
        AttestationStatus::Verified => {
            send_output(
                output_tx,
                &format!(
                    "[attestation] GitHub Actions attestation independently verified for \
                     {owner}/{repo}"
                ),
                OutputStreamType::System,
            )
            .await;
            Ok(())
        }
        AttestationStatus::NotFound => {
            if ri.require_attestation {
                let msg = format!(
                    "no GitHub Actions attestation found for {owner}/{repo}; \
                     update blocked by require_attestation policy"
                );
                send_output(
                    output_tx,
                    &format!("[attestation] [error] {msg}"),
                    OutputStreamType::Stderr,
                )
                .await;
                Err(AgentCoreError::AttestationFailed(msg))
            } else {
                send_output(
                    output_tx,
                    &format!(
                        "[attestation] [warning] No GitHub Actions attestation found for \
                         {owner}/{repo}. Proceeding (require_attestation is false)."
                    ),
                    OutputStreamType::System,
                )
                .await;
                Ok(())
            }
        }
        AttestationStatus::Unverified => {
            send_output(
                output_tx,
                &format!(
                    "[attestation] Attestation check inconclusive for {owner}/{repo}; \
                     proceeding with install"
                ),
                OutputStreamType::System,
            )
            .await;
            Ok(())
        }
        _ => {
            // Forward-compatible: treat unknown statuses as Unverified.
            send_output(
                output_tx,
                "[attestation] Attestation check returned unknown status; proceeding with install",
                OutputStreamType::System,
            )
            .await;
            Ok(())
        }
    }
}

// ── Interactive update execution ──────────────────────────────────────

/// Handle returned from [`execute_update_interactive`], containing the
/// spawned task and channels for stdin/signal/attention forwarding.
#[cfg(feature = "interactive")]
pub struct InteractiveUpdateHandle {
    pub handle: tokio::task::JoinHandle<UpdateExecutionResult>,
    pub stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    pub signal_tx: Option<mpsc::Sender<i32>>,
    pub attention_rx: Option<mpsc::Receiver<()>>,
}

/// Tuple type for channels delivered from [`ForwardingInteractiveExecutor`]
/// to [`execute_update_interactive`] via oneshot.
#[cfg(feature = "interactive")]
type InteractiveChannels = (mpsc::Sender<Vec<u8>>, mpsc::Sender<i32>, mpsc::Receiver<()>);

/// A [`CommandExecutor`] wrapper that intercepts the first `execute()` call
/// and promotes it to `execute_interactive()`.
///
/// This allows the generic update pipeline (which calls `plugin.execute_update`
/// → `executor.execute()`) to transparently use a PTY without requiring any
/// changes to the `Plugin` trait.
///
/// On the first `execute()` call:
/// - Calls the inner executor's `execute_interactive()`.
/// - Sends the interactive channels (`stdin_tx`, `signal_tx`, `attention_rx`)
///   to the waiting [`execute_update_interactive`] function via a oneshot.
/// - Drives the PTY to completion and returns the resulting `CommandOutput`.
///
/// If `execute_interactive()` fails (executor doesn't support it or setup
/// errors), drops the oneshot sender and falls back to non-interactive
/// `execute()`. Subsequent `execute()` calls always pass through to the
/// inner executor.
#[cfg(feature = "interactive")]
struct ForwardingInteractiveExecutor {
    inner: Arc<dyn CommandExecutor>,
    channels_tx: parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<InteractiveChannels>>>,
}

#[cfg(feature = "interactive")]
#[async_trait::async_trait]
impl CommandExecutor for ForwardingInteractiveExecutor {
    async fn execute(
        &self,
        spec: &uptrakit_command::CommandSpec,
        output_tx: &mpsc::Sender<uptrakit_command::UpdateOutputLine>,
    ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
        // Take the sender on the first execute() call to attempt interactive
        // promotion. Subsequent calls pass through directly.
        let maybe_tx = self.channels_tx.lock().take();
        if let Some(tx) = maybe_tx {
            match self.inner.execute_interactive(spec, output_tx).await {
                Ok(handle) => {
                    // Deliver channels to execute_update_interactive.
                    // Ignore send errors — the receiver may have been dropped if
                    // the outer function timed out (shouldn't happen in practice).
                    let _ = tx.send((handle.stdin_tx, handle.signal_tx, handle.attention_rx));
                    // Drive the PTY process to completion.
                    handle.completion.await.map_err(|e| {
                        rootcause::report!(uptrakit_command::CommandError::UnsupportedOperation(
                            format!("interactive task panicked: {e}")
                        ))
                    })?
                }
                Err(e) => {
                    // Interactive not supported or setup failed — drop tx to
                    // unblock execute_update_interactive with None channels,
                    // then fall back to non-interactive execution.
                    drop(tx);
                    tracing::warn!(
                        error = %e,
                        "interactive execution unavailable, falling back to non-interactive"
                    );
                    self.inner.execute(spec, output_tx).await
                }
            }
        } else {
            self.inner.execute(spec, output_tx).await
        }
    }

    async fn execute_quiet(
        &self,
        spec: &uptrakit_command::CommandSpec,
    ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
        self.inner.execute_quiet(spec).await
    }

    fn supports_interactive(&self) -> bool {
        self.inner.supports_interactive()
    }

    async fn execute_interactive(
        &self,
        spec: &uptrakit_command::CommandSpec,
        output_tx: &mpsc::Sender<uptrakit_command::UpdateOutputLine>,
    ) -> uptrakit_command::Result<uptrakit_command::InteractiveHandle> {
        self.inner.execute_interactive(spec, output_tx).await
    }
}

/// Execute an update interactively with PTY support.
///
/// Runs the same update pipeline as [`execute_update`] but wraps the executor
/// in a [`ForwardingInteractiveExecutor`] so that the plugin's first
/// `execute()` call is promoted to `execute_interactive()`. This allocates a
/// real PTY for the update command, making `/dev/tty` available and keeping
/// stdin open for forwarding — without any changes to the `Plugin` trait.
///
/// Pre/post hooks and version detection still run non-interactively via the
/// inner executor's `execute()` / `execute_quiet()` paths. Only the plugin's
/// primary `execute_update` call gets the PTY.
///
/// Returns an [`InteractiveUpdateHandle`] whose `stdin_tx`, `signal_tx`, and
/// `attention_rx` are `Some(...)` when the executor supports interactive mode
/// and the update reaches the plugin execution step. If the executor falls back
/// to non-interactive (e.g. SSH executor without PTY support, or the update
/// fails before reaching the plugin step), all three fields are `None` and the
/// update still runs to completion via `handle`.
#[cfg(feature = "interactive")]
pub async fn execute_update_interactive(
    payload: ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
) -> InteractiveUpdateHandle {
    let (channels_tx, channels_rx) = tokio::sync::oneshot::channel::<InteractiveChannels>();

    let forwarding = Arc::new(ForwardingInteractiveExecutor {
        inner: executor,
        channels_tx: parking_lot::Mutex::new(Some(channels_tx)),
    });

    let handle = tokio::spawn(async move { execute_update(payload, forwarding, output_tx).await });

    // Await delivery of the interactive channels from
    // ForwardingInteractiveExecutor::execute(). This resolves when the plugin's
    // first execute() call is intercepted and promoted to interactive mode.
    //
    // If the oneshot sender is dropped before sending (the update fails before
    // reaching the plugin's execute step, or the executor falls back to
    // non-interactive), channels_rx returns Err and we return None channels —
    // the update still runs to completion via `handle`.
    match channels_rx.await {
        Ok((stdin_tx, signal_tx, attention_rx)) => InteractiveUpdateHandle {
            handle,
            stdin_tx: Some(stdin_tx),
            signal_tx: Some(signal_tx),
            attention_rx: Some(attention_rx),
        },
        Err(_) => InteractiveUpdateHandle {
            handle,
            stdin_tx: None,
            signal_tx: None,
            attention_rx: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::LocalCommandExecutor;
    use uptrakit_internal_wire::PluginType;

    fn test_payload() -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            host_machine_id: String::new(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            to_version: "2.0.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: uptrakit_internal_wire::PluginAssignment {
                plugin_type: PluginType::ReleasesGithub,
                package_identifier: "test-app".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            release_info: None,
            timeout: std::time::Duration::from_secs(60),
            interactive: false,
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

    // ── Lifecycle hook plugin tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_update_with_shell_pre_hook_plugin() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hook_plugins = vec![uptrakit_internal_wire::PluginAssignment {
            plugin_type: PluginType::HookShell,
            package_identifier: String::new(),
            config: serde_json::json!({"pre_command": "echo 'pre-hook executed'"}),
        }];
        payload.release_info = None;

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
    async fn test_execute_update_pre_hook_plugin_failure() {
        let (tx, mut rx) = mpsc::channel(100);

        let mut payload = test_payload();
        payload.pre_update_hook_plugins = vec![uptrakit_internal_wire::PluginAssignment {
            plugin_type: PluginType::HookShell,
            package_identifier: String::new(),
            config: serde_json::json!({"pre_command": "exit 1"}),
        }];

        let result = execute_update(payload, test_executor(), tx).await;

        assert_eq!(result.result.status, UpdateFinalStatus::Failed);
        assert!(result.result.error.is_some(), "Expected error but got None");

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn test_run_pre_hook_plugins_empty_is_noop() {
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_pre_hook("pkg", "1.0", None, None);
        let mut output = String::new();
        let result = run_pre_hook_plugins(&[], &ctx, test_executor(), &tx, &mut output).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_post_hook_plugins_empty_is_noop() {
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_post_hook("pkg", "1.0", None, None, true);
        let mut output = String::new();
        run_post_hook_plugins(&[], &ctx, test_executor(), &tx, &mut output).await;
        // No panic or error — noop
    }

    #[tokio::test]
    async fn test_run_batch_pre_hook_plugins_empty_is_noop() {
        let ctx = UpdateLifecycleContext::for_pre_hook("", "", None, None);
        let result = run_batch_pre_hook_plugins(&[], &ctx, test_executor()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_batch_post_hook_plugins_empty_is_noop() {
        let ctx = UpdateLifecycleContext::for_post_hook("", "", None, None, true);
        run_batch_post_hook_plugins(&[], &ctx, test_executor()).await;
        // No panic or error — noop
    }

    // ── Attestation gate tests ───────────────────────────────────────────

    fn make_release_info_with_status(
        status: Option<AttestationStatus>,
        require: bool,
    ) -> ReleaseInfo {
        ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://github.com/owner/repo/releases/tag/v1.0.0".to_string(),
            assets: vec![uptrakit_internal_wire::ReleaseAsset {
                name: "app.tar.gz".to_string(),
                download_url: "https://github.com/owner/repo/releases/download/v1.0.0/app.tar.gz"
                    .to_string(),
                size: None,
                content_type: None,
                sha256_digest: None,
            }],
            attestation_status: status,
            require_attestation: require,
        }
    }

    #[test]
    fn parse_github_owner_repo_valid() {
        let r = parse_github_owner_repo("https://github.com/owner/repo/releases/tag/v1.0.0");
        assert_eq!(r, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn parse_github_owner_repo_non_github() {
        assert!(
            parse_github_owner_repo("https://gitlab.com/owner/repo/releases/tag/v1.0.0").is_none()
        );
    }

    #[test]
    fn parse_github_owner_repo_missing_repo() {
        assert!(parse_github_owner_repo("https://github.com/owner/").is_none());
    }

    #[test]
    fn parse_github_owner_repo_empty_owner() {
        assert!(parse_github_owner_repo("https://github.com//repo/releases").is_none());
    }

    #[tokio::test]
    async fn check_attestation_gate_none_release_info_ok() {
        let (tx, _rx) = mpsc::channel(10);
        let result = check_attestation_gate(None, &tx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_attestation_gate_already_verified_ok() {
        let (tx, mut rx) = mpsc::channel(10);
        let ri = make_release_info_with_status(Some(AttestationStatus::Verified), false);
        let result = check_attestation_gate(Some(&ri), &tx).await;
        assert!(result.is_ok());
        rx.close();
        let mut found = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("verified") {
                found = true;
            }
        }
        assert!(found);
    }

    #[tokio::test]
    async fn check_attestation_gate_non_github_url_ok() {
        let (tx, _rx) = mpsc::channel(10);
        let mut ri = make_release_info_with_status(None, true);
        ri.release_url = "https://example.com/releases/v1.0.0".to_string();
        let result = check_attestation_gate(Some(&ri), &tx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_attestation_gate_no_digest_ok() {
        let (tx, mut rx) = mpsc::channel(10);
        // No sha256_digest on any asset → skip independent verify.
        let ri = make_release_info_with_status(None, false);
        let result = check_attestation_gate(Some(&ri), &tx).await;
        assert!(result.is_ok());
        rx.close();
        let mut found = false;
        while let Some(msg) = rx.recv().await {
            if msg.output.contains("No asset digest") {
                found = true;
            }
        }
        assert!(found);
    }
}
