#![expect(
    clippy::let_underscore_must_use,
    clippy::string_slice,
    reason = "let _ = used for intentional fire-and-forget of channel sends and JoinHandles; string_slice used with pre-validated byte positions"
)]
//! Update execution module for Uptrakit agents.
//!
//! Handles the complete update flow:
//! 1. Receive `ExecuteUpdate` message
//! 2. Detect current version (`from_version`)
//! 3. Run pre-update lifecycle hook plugins (ordered by assignment)
//! 4. Execute actual update (dispatched through Plugin Registry)
//! 5. Run post-update lifecycle hook plugins (inline for non-resumable; spawned fire-and-forget for resumable)
//! 6. Detect to_version post-update
//! 7. Return `UpdateExecutionResult` with final status and accumulated output

use std::sync::Arc;

use rootcause::prelude::*;
use thiserror::Error as ThisError;
use tokio::sync::mpsc;
use uptrakit_command::{CommandExecutor, UpdateOutputLine};
use uptrakit_plugin_infrastructure_registry::{
    ExecuteUpdateResult, HostRuntime, PluginError, UpdateLifecycleContext, get_descriptor,
};
use uptrakit_wire::{
    AttestationStatus, ExecuteUpdatePayload, OutputStreamType, PluginAssignment, ReleaseInfo,
    UpdateFinalStatus, UpdateResultPayload,
};

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
    /// `true` when the plugin signalled the update is mid-restart (e.g.
    /// self-update or APT phased reboot) and the post-update state should be
    /// observed on the next reconnect rather than reported now.
    pub resumable: bool,
}

/// Outcome of [`execute_update_pipeline`].
///
/// The pipeline runs pre-hooks, the attestation gate, and the plugin update,
/// but no longer runs post-hooks itself — the outer [`execute_update`]
/// function handles those, since post-hook scheduling depends on whether the
/// update was resumable (run inline vs spawned for fire-and-forget).
struct PipelineResult {
    /// `true` when pre-hooks, attestation, and the plugin update all
    /// completed without error.
    succeeded: bool,
    /// `true` when the plugin's [`ExecuteUpdateResult::resumable`] flag was
    /// set, meaning a restart is imminent and post-update observations must
    /// happen after reconnect. Only meaningful when `succeeded` is `true`.
    resumable: bool,
}

/// Output message sent during update execution.
pub struct UpdateOutputMessage {
    pub output: String,
    pub stream: OutputStreamType,
}

/// Run the update pipeline: pre-hook plugins → attestation gate → plugin execution.
///
/// Post-hook plugins are NOT run here — the outer [`execute_update`] function
/// runs them inline for non-resumable updates and spawns them as a
/// fire-and-forget task for resumable ones (so the host can restart without
/// waiting for them).
///
/// The caller wraps this in [`tokio::time::timeout`] so cancellation
/// (`drop`) releases the `&mut accumulated_output` borrow cleanly.
async fn execute_update_pipeline(
    payload: &ExecuteUpdatePayload,
    output_tx: &mpsc::Sender<UpdateOutputMessage>,
    runtime: Arc<dyn HostRuntime>,
    accumulated_output: &mut String,
    // Runtime override used ONLY for the update command itself
    // (`execute_plugin_update`). Hooks and version detection always run on
    // `runtime`. The interactive path passes the forwarding runtime here so
    // PTY promotion targets exactly the update command; `None` for the
    // non-interactive path.
    update_exec_runtime: Option<Arc<dyn HostRuntime>>,
) -> PipelineResult {
    // Run pre-update lifecycle hook plugins
    let lifecycle_ctx = UpdateLifecycleContext::for_pre_hook(
        &payload.execute_update_plugin.package_identifier,
        &payload.to_version,
        None, // from_version not yet available at this stage
        payload.release_info.clone(),
    );
    if let Err(e) = run_pre_hook_plugins(
        &payload.pre_update_hook_plugins,
        &lifecycle_ctx,
        Arc::clone(&runtime),
        output_tx,
        accumulated_output,
    )
    .await
    {
        tracing::warn!(error = %e, "pre-update hook failed; aborting pipeline");
        return PipelineResult {
            succeeded: false,
            resumable: false,
        };
    }

    // Attestation gate — abort if policy requires a verified attestation
    // and none was found.
    tracing::debug!("checking attestation gate");
    if let Err(e) = check_attestation_gate(payload.release_info.as_ref(), output_tx).await {
        tracing::warn!(error = %e, "attestation gate failed; aborting pipeline");
        let formatted = format!("[error] {e}\n");
        append_bounded(accumulated_output, &formatted, MAX_OUTPUT_BYTES);
        return PipelineResult {
            succeeded: false,
            resumable: false,
        };
    }
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
    let exec_runtime = update_exec_runtime.unwrap_or_else(|| Arc::clone(&runtime));
    match execute_plugin_update(payload, output_tx, exec_runtime).await {
        Ok(exec_result) => {
            tracing::debug!(
                resumable = exec_result.resumable,
                "plugin update returned successfully"
            );
            append_bounded(accumulated_output, &exec_result.output, MAX_OUTPUT_BYTES);
            PipelineResult {
                succeeded: true,
                resumable: exec_result.resumable,
            }
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
            PipelineResult {
                succeeded: false,
                resumable: false,
            }
        }
    }
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
    runtime: Arc<dyn HostRuntime>,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
    early_result_tx: tokio::sync::mpsc::UnboundedSender<UpdateResultPayload>,
    // Runtime override used ONLY for the update command itself
    // (`execute_plugin_update`). Hooks and version detection always run on
    // `runtime`. The interactive path passes the forwarding runtime here so
    // PTY promotion targets exactly the update command; `None` for the
    // non-interactive path.
    update_exec_runtime: Option<Arc<dyn HostRuntime>>,
) -> UpdateExecutionResult {
    tracing::info!(software_item = %payload.software_item_name, "starting update");
    let update_history_id = payload.update_history_id;
    // Copy before the payload reference is used inside the timeout future.
    let timeout_duration = payload.timeout;

    // Detect current version (from_version)
    let from_version = detect_current_version(&payload, Arc::clone(&runtime)).await;
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
            Arc::clone(&runtime),
            &mut accumulated_output,
            update_exec_runtime,
        ),
    )
    .await;

    // Resolve pipeline outcome (timeout vs. PipelineResult).
    let PipelineResult {
        succeeded,
        resumable: pipeline_resumable,
    } = match execution_result {
        Ok(r) => r,
        Err(_elapsed) => {
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
            final_error = Some(timeout_msg);
            PipelineResult {
                succeeded: false,
                resumable: false,
            }
        }
    };

    if !succeeded {
        final_status = UpdateFinalStatus::Failed;
        if final_error.is_none() {
            final_error = Some("plugin update command failed".to_string());
        }
    }

    // Post-hook scheduling depends on whether the update is resumable.
    //
    // - Non-resumable (success or failure): run inline so we can wait for
    //   completion, capture output, and reflect any side effects in the
    //   final UpdateResultPayload.
    // - Resumable (only meaningful on success): the host is about to
    //   restart, so we spawn the post-hooks fire-and-forget. Their output
    //   would not survive the restart anyway, and blocking would defeat
    //   the purpose of resumable updates.
    let post_ctx = UpdateLifecycleContext::for_post_hook(
        &payload.execute_update_plugin.package_identifier,
        &payload.to_version,
        None,
        payload.release_info.clone(),
        succeeded,
    );

    let resumable_flag = succeeded && pipeline_resumable;

    if resumable_flag {
        // Send the early result payload to the agent runtime *before* the
        // restart-side-effect of post-hooks fires. The runtime forwards this
        // to the controller so it can transition the update to
        // `AwaitingRestart` ahead of the imminent restart signal.
        let early_payload = UpdateResultPayload {
            update_history_id,
            status: UpdateFinalStatus::Completed,
            from_version: from_version.clone(),
            to_version: None, // controller will verify post-restart via AwaitingRestartExecutor
            output: accumulated_output.clone(),
            error: None,
            resumable: Some(true),
        };
        // Fire-and-forget: the receiver may be closed if the runtime tore
        // down (graceful shutdown, agent restart, etc.).
        let _ = early_result_tx.send(early_payload);

        // Fire-and-forget: the host is restarting; we cannot block on hooks.
        tracing::info!(
            update_id = %update_history_id,
            "spawning post-update hooks for resumable update"
        );
        let post_hook_plugins = payload.post_update_hook_plugins.clone();
        let post_runtime = Arc::clone(&runtime);
        let post_output_tx = output_tx.clone();
        tokio::spawn(async move {
            let mut spawned_output = String::new();
            run_post_hook_plugins(
                &post_hook_plugins,
                &post_ctx,
                post_runtime,
                &post_output_tx,
                &mut spawned_output,
            )
            .await;
            // The host is restarting; this output is captured only by the
            // bridge channel (already closed once the runtime tears down).
            tracing::debug!(
                output_len = spawned_output.len(),
                "spawned post-hook task finished"
            );
        });
    } else {
        run_post_hook_plugins(
            &payload.post_update_hook_plugins,
            &post_ctx,
            Arc::clone(&runtime),
            &output_tx,
            &mut accumulated_output,
        )
        .await;
    }

    // Final logging + version detection (skipped for resumable: post-state
    // is unobservable here because the restart is imminent).
    let to_version = if succeeded {
        if resumable_flag {
            tracing::info!(
                software_item = %payload.software_item_name,
                "resumable update dispatched; awaiting post-restart observation"
            );
            send_output(
                &output_tx,
                "[update] Resumable update dispatched; awaiting restart",
                OutputStreamType::System,
            )
            .await;
            None
        } else {
            tracing::info!(software_item = %payload.software_item_name, "update completed successfully");
            send_output(
                &output_tx,
                "[update] Update completed successfully",
                OutputStreamType::System,
            )
            .await;
            let detected = detect_current_version(&payload, Arc::clone(&runtime)).await;
            tracing::debug!(to_version = ?detected, "post-update version detected");
            detected
        }
    } else {
        None
    };

    let result = UpdateResultPayload {
        update_history_id,
        status: final_status,
        from_version,
        to_version,
        output: accumulated_output,
        error: final_error,
        resumable: if resumable_flag { Some(true) } else { None },
    };

    UpdateExecutionResult {
        result,
        resumable: resumable_flag,
    }
}

/// Detect the current version of a software item by delegating to the plugin registry.
///
/// Uses the `detect_version_plugin` from the payload if available.
#[tracing::instrument(skip_all, fields(software_item = %payload.software_item_name))]
async fn detect_current_version(
    payload: &ExecuteUpdatePayload,
    runtime: Arc<dyn HostRuntime>,
) -> Option<String> {
    // The connection context has already been merged into the plugin config
    // by the caller before the update task was spawned.
    let detect_assignment = payload.detect_version_plugin.as_ref();
    let outcome = crate::version_check::check_version(
        detect_assignment,
        None,
        runtime,
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
    runtime: Arc<dyn HostRuntime>,
) -> UpdateResult<ExecuteUpdateResult> {
    let eu = &payload.execute_update_plugin;

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
    runtime: Arc<dyn HostRuntime>,
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

        let lifecycle = (slot.create)(&assignment.config, Arc::clone(&runtime)).map_err(|e| {
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
    runtime: Arc<dyn HostRuntime>,
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

        let lifecycle = match (slot.create)(&assignment.config, Arc::clone(&runtime)) {
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
    runtime: Arc<dyn HostRuntime>,
) -> UpdateResult<()> {
    for assignment in plugins {
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

        let lifecycle = (slot.create)(&assignment.config, Arc::clone(&runtime)).map_err(|e| {
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
    runtime: Arc<dyn HostRuntime>,
) {
    for assignment in plugins {
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

        let lifecycle = match (slot.create)(&assignment.config, Arc::clone(&runtime)) {
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
/// spawned task and a oneshot that resolves once the update pipeline
/// promotes the plugin's update command to interactive mode.
#[cfg(feature = "interactive")]
pub struct InteractiveUpdateHandle {
    pub handle: tokio::task::JoinHandle<UpdateExecutionResult>,
    /// Resolved by the event loop when [`ForwardingInteractiveExecutor`]
    /// promotes the update command to `execute_interactive()`. Errors
    /// (sender dropped) if the pipeline ends without promotion — e.g. the
    /// update fails before reaching the plugin's execute step, or the
    /// executor falls back to non-interactive execution.
    pub channels_rx: tokio::sync::oneshot::Receiver<InteractiveChannels>,
}

/// Tuple type for channels delivered from [`ForwardingInteractiveExecutor`]
/// to [`execute_update_interactive`] via oneshot.
#[cfg(feature = "interactive")]
pub type InteractiveChannels = (mpsc::Sender<Vec<u8>>, mpsc::Sender<i32>, mpsc::Receiver<()>);

/// Fill the dispatch payload's update timeout into a spec that carries no
/// plugin-set timeout, and mark it drain-on-abandon (update commands mutate
/// host state — a dropped update task must not SIGKILL them mid-flight).
/// Plugin-set timeouts are always respected.
fn apply_update_budget(
    spec: &uptrakit_command::CommandSpec,
    update_timeout: std::time::Duration,
) -> uptrakit_command::CommandSpec {
    let mut spec = spec.clone();
    if spec.timeout.is_none() {
        spec.timeout = Some(update_timeout);
    }
    spec.abandonment = uptrakit_command::AbandonmentPolicy::DrainOnAbandon;
    spec
}

/// Wraps the runtime executor for **non-interactive** updates so that every
/// command the update plugin runs carries the update budget from the
/// dispatch payload (see [`apply_update_budget`]). Installed as
/// `update_exec_runtime` by `spawn_update_task` (single updates) and around
/// the per-item updater construction in `run_execute_batch_update` (batch
/// updates), so only update-command execution routes through it — hooks and
/// version detection keep the plain runtime (hooks get their own deadline;
/// detection uses the executor default).
pub(crate) struct BudgetForwardingExecutor {
    pub(crate) inner: Arc<dyn CommandExecutor>,
    pub(crate) update_timeout: std::time::Duration,
}

#[async_trait::async_trait]
impl CommandExecutor for BudgetForwardingExecutor {
    async fn execute(
        &self,
        spec: &uptrakit_command::CommandSpec,
        output_tx: &mpsc::Sender<uptrakit_command::UpdateOutputLine>,
    ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
        self.inner
            .execute(&apply_update_budget(spec, self.update_timeout), output_tx)
            .await
    }

    async fn execute_quiet(
        &self,
        spec: &uptrakit_command::CommandSpec,
    ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
        self.inner
            .execute_quiet(&apply_update_budget(spec, self.update_timeout))
            .await
    }
}

/// Wrap `runtime`'s executor in [`BudgetForwardingExecutor`] for update
/// execution, downcast-gated to `StandardHostRuntime`.
///
/// Plugin construction downcasts the runtime it is handed (RouterOsPlugin →
/// `RouterOsHostRuntime`), so only the one runtime type we can faithfully
/// rebuild is wrapped; any other runtime returns `None` and callers keep the
/// original. RouterOS commands never use the generic `CommandExecutor` (its
/// `executor()` is a Noop), so nothing is lost by the pass-through.
pub(crate) fn budget_runtime_for_update(
    runtime: &Arc<dyn HostRuntime>,
    update_timeout: std::time::Duration,
) -> Option<Arc<dyn HostRuntime>> {
    use uptrakit_plugin_infrastructure_registry::{StandardHostRuntime, construct_host_runtime};
    runtime
        .as_any()
        .downcast_ref::<StandardHostRuntime>()
        .is_some()
        .then(|| {
            let budget_executor: Arc<dyn uptrakit_command::CommandExecutor> =
                Arc::new(BudgetForwardingExecutor {
                    inner: runtime.executor(),
                    update_timeout,
                });
            construct_host_runtime(budget_executor, runtime.capabilities().clone())
        })
}

/// A [`CommandExecutor`] wrapper that intercepts the first `execute()` call
/// and promotes it to `execute_interactive()`.
///
/// This allows `execute_plugin_update` (which calls `plugin.execute_update`
/// → `executor.execute()`) to transparently use a PTY without requiring any
/// changes to the `Plugin` trait. It is passed as `update_exec_runtime` to
/// [`execute_update`] so that only the update command runs through it — pre/post
/// hooks and version detection use the plain `runtime` and are unaffected.
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
    /// Update timeout from the dispatch payload, used to fill `spec.timeout`
    /// on promotion when the plugin didn't set one (see [`CommandExecutor::execute`]
    /// below). Plugin-set timeouts are always respected.
    update_timeout: std::time::Duration,
}

/// Kills the PTY child's process group if the pipeline future is cancelled
/// (e.g. the outer update timeout) while the interactive session is running.
/// Defused on normal completion. Accepted PID-reuse TOCTOU: the guard can
/// fire after the group exited; the window is the instant between exit and
/// defusal on a freshly-vacated pgid — COMPARABLE, not identical, to the
/// deadline path's theoretical window in `drive_interactive_session`: the
/// deadline path reaps via `child.wait().await` after killing, while this
/// sync guard cannot reap (and its `.abort()` of the session task may delay
/// the reap to `kill_on_drop`/the OS), so this guard's recycle window is
/// marginally wider.
#[cfg(feature = "interactive")]
struct InteractiveSessionGuard {
    abort: tokio::task::AbortHandle,
    child_pid: i32,
    armed: bool,
}

#[cfg(feature = "interactive")]
impl Drop for InteractiveSessionGuard {
    fn drop(&mut self) {
        if self.armed {
            // Group-kill FIRST, then abort: kill the group while the direct
            // child is still a member — aborting first drops the session
            // future, whose kill_on_drop reap of the leader races the
            // group-kill window.
            uptrakit_command::kill_process_group(self.child_pid);
            self.abort.abort();
        }
    }
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
            // Propagate the update deadline into the PTY session when the plugin
            // didn't set its own timeout — otherwise an interactive command could
            // run unbounded. Plugin-set timeouts are always respected.
            let promoted_spec = apply_update_budget(spec, self.update_timeout);
            match self
                .inner
                .execute_interactive(&promoted_spec, output_tx)
                .await
            {
                Ok(handle) => {
                    // Deliver channels to execute_update_interactive.
                    // Ignore send errors — the receiver may have been dropped if
                    // the outer function timed out (shouldn't happen in practice).
                    let _ = tx.send((handle.stdin_tx, handle.signal_tx, handle.attention_rx));
                    // Arm a guard that group-kills the PTY child and aborts the
                    // session task if this future is cancelled (e.g. by the
                    // outer update timeout) while awaiting completion below.
                    // Use abort_handle() so the guard never owns the JoinHandle;
                    // we still await the owned `handle.completion` directly.
                    let mut guard = InteractiveSessionGuard {
                        abort: handle.completion.abort_handle(),
                        child_pid: handle.child_pid,
                        armed: true,
                    };
                    // Drive the PTY process to completion.
                    let result = handle.completion.await;
                    guard.armed = false; // defuse: normal completion, nothing to kill
                    result.map_err(|e| {
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
                    self.inner.execute(&promoted_spec, output_tx).await
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
        self.inner
            .execute_quiet(&apply_update_budget(spec, self.update_timeout))
            .await
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

/// Spawn an update interactively with PTY support.
///
/// Runs the same update pipeline as [`execute_update`] but constructs a
/// [`ForwardingInteractiveExecutor`] and passes it as `update_exec_runtime`
/// so that only `execute_plugin_update` runs through it — pre/post hooks and
/// version detection still use the original `runtime` directly. The forwarding
/// executor intercepts the plugin's first `execute()` call and promotes it to
/// `execute_interactive()`, allocating a real PTY without any changes to the
/// `Plugin` trait.
///
/// Returns immediately with an [`InteractiveUpdateHandle`] — it does not await
/// PTY promotion. The handle's `channels_rx` is resolved later by the event
/// loop when (and if) the forwarding executor promotes the update command to
/// interactive mode.
#[cfg(feature = "interactive")]
#[tracing::instrument(skip_all, fields(update_history_id = %payload.update_history_id))]
pub fn execute_update_interactive(
    payload: ExecuteUpdatePayload,
    runtime: Arc<dyn HostRuntime>,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
    early_result_tx: tokio::sync::mpsc::UnboundedSender<UpdateResultPayload>,
) -> InteractiveUpdateHandle {
    let (channels_tx, channels_rx) = tokio::sync::oneshot::channel::<InteractiveChannels>();

    // Extract the inner executor from the runtime to wrap in ForwardingInteractiveExecutor.
    // The forwarding executor intercepts the first execute() call and promotes it to
    // execute_interactive(). We then re-wrap using construct_host_runtime so that
    // the capabilities are preserved.
    use uptrakit_plugin_infrastructure_registry::construct_host_runtime;
    let inner_executor = runtime.executor();
    let caps = runtime.capabilities().clone();
    // Read before `payload` moves into the spawned pipeline task below.
    let update_timeout = payload.timeout;
    let forwarding_executor: Arc<dyn uptrakit_command::CommandExecutor> =
        Arc::new(ForwardingInteractiveExecutor {
            inner: inner_executor,
            channels_tx: parking_lot::Mutex::new(Some(channels_tx)),
            update_timeout,
        });
    let forwarding_runtime = construct_host_runtime(forwarding_executor, caps);

    let handle = tokio::spawn(async move {
        execute_update(
            payload,
            runtime,
            output_tx,
            early_result_tx,
            Some(forwarding_runtime),
        )
        .await
    });

    InteractiveUpdateHandle {
        handle,
        channels_rx,
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test code: `assert!(r.is_ok())` is idiomatic in tests where the success value is not inspected"
    )]

    use super::*;
    use uptrakit_wire::plugin_ids;

    fn test_payload() -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            host_machine_id: String::new(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            to_version: "2.0.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: uptrakit_wire::PluginAssignment {
                plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
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

    fn test_runtime() -> Arc<dyn HostRuntime> {
        use uptrakit_command::LocalCommandExecutor;
        use uptrakit_plugin_infrastructure_core::{HostCapabilities, StandardHostRuntime};
        Arc::new(StandardHostRuntime::new(
            Arc::new(LocalCommandExecutor),
            HostCapabilities::default(),
        ))
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
        payload.pre_update_hook_plugins = vec![uptrakit_wire::PluginAssignment {
            plugin_type: plugin_ids::HOOK_SHELL.clone(),
            package_identifier: String::new(),
            config: serde_json::json!({"pre_command": "echo 'pre-hook executed'"}),
        }];
        payload.release_info = None;

        let (early_tx, _early_rx) = tokio::sync::mpsc::unbounded_channel();
        let result = execute_update(payload, test_runtime(), tx, early_tx, None).await;
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
        payload.pre_update_hook_plugins = vec![uptrakit_wire::PluginAssignment {
            plugin_type: plugin_ids::HOOK_SHELL.clone(),
            package_identifier: String::new(),
            config: serde_json::json!({"pre_command": "exit 1"}),
        }];

        let (early_tx, _early_rx) = tokio::sync::mpsc::unbounded_channel();
        let result = execute_update(payload, test_runtime(), tx, early_tx, None).await;

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
        let result = run_pre_hook_plugins(&[], &ctx, test_runtime(), &tx, &mut output).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_post_hook_plugins_empty_is_noop() {
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateLifecycleContext::for_post_hook("pkg", "1.0", None, None, true);
        let mut output = String::new();
        run_post_hook_plugins(&[], &ctx, test_runtime(), &tx, &mut output).await;
        // No panic or error — noop
    }

    #[tokio::test]
    async fn test_run_batch_pre_hook_plugins_empty_is_noop() {
        let ctx = UpdateLifecycleContext::for_pre_hook("", "", None, None);
        let result = run_batch_pre_hook_plugins(&[], &ctx, test_runtime()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_batch_post_hook_plugins_empty_is_noop() {
        let ctx = UpdateLifecycleContext::for_post_hook("", "", None, None, true);
        run_batch_post_hook_plugins(&[], &ctx, test_runtime()).await;
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
            assets: vec![uptrakit_wire::ReleaseAsset {
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

    // ── Update-budget forwarding tests ──────────────────────────────────────

    /// `CommandExecutor` stub that records the last spec passed to `execute`
    /// or `execute_quiet`, and always succeeds. Mirrors
    /// `interactive_lifecycle_tests::RecordingExecutor` but for the
    /// non-interactive path (no `execute_interactive` override needed).
    struct RecordingExecutor {
        recorded: parking_lot::Mutex<Option<uptrakit_command::CommandSpec>>,
    }

    #[async_trait::async_trait]
    impl CommandExecutor for RecordingExecutor {
        async fn execute(
            &self,
            spec: &uptrakit_command::CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
            *self.recorded.lock() = Some(spec.clone());
            Ok(uptrakit_command::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            spec: &uptrakit_command::CommandSpec,
        ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
            *self.recorded.lock() = Some(spec.clone());
            Ok(uptrakit_command::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }
    }

    /// A plugin-unbudgeted spec gets the dispatch payload's update timeout
    /// and `DrainOnAbandon` — update commands mutate host state, so a dropped
    /// update task must not SIGKILL them mid-flight. Uses
    /// `uptrakit_wire::DEFAULT_UPDATE_TIMEOUT` directly rather than
    /// hand-writing 7200.
    #[tokio::test]
    async fn budget_forwarding_fills_timeout_and_drain_policy() {
        let stub = Arc::new(RecordingExecutor {
            recorded: parking_lot::Mutex::new(None),
        });
        let forwarding = BudgetForwardingExecutor {
            inner: stub.clone(),
            update_timeout: uptrakit_wire::DEFAULT_UPDATE_TIMEOUT,
        };
        let (output_tx, _output_rx) = mpsc::channel(10);
        let spec = uptrakit_command::CommandSpec::shell("true");
        let _ = forwarding.execute(&spec, &output_tx).await;

        let got = stub
            .recorded
            .lock()
            .take()
            .expect("stub should record spec");
        assert_eq!(got.timeout, Some(uptrakit_wire::DEFAULT_UPDATE_TIMEOUT));
        assert_eq!(
            got.abandonment,
            uptrakit_command::AbandonmentPolicy::DrainOnAbandon
        );
    }

    /// A plugin-set timeout survives budget forwarding untouched; the drain
    /// policy is still applied.
    #[tokio::test]
    async fn budget_forwarding_respects_plugin_set_timeout() {
        let stub = Arc::new(RecordingExecutor {
            recorded: parking_lot::Mutex::new(None),
        });
        let forwarding = BudgetForwardingExecutor {
            inner: stub.clone(),
            update_timeout: uptrakit_wire::DEFAULT_UPDATE_TIMEOUT,
        };
        let (output_tx, _output_rx) = mpsc::channel(10);
        let spec = uptrakit_command::CommandSpec::shell("true")
            .with_timeout(std::time::Duration::from_secs(5));
        let _ = forwarding.execute(&spec, &output_tx).await;

        let got = stub
            .recorded
            .lock()
            .take()
            .expect("stub should record spec");
        assert_eq!(got.timeout, Some(std::time::Duration::from_secs(5)));
        assert_eq!(
            got.abandonment,
            uptrakit_command::AbandonmentPolicy::DrainOnAbandon
        );
    }

    /// `budget_runtime_for_update` wraps a `StandardHostRuntime` (forwarding
    /// the budget through its executor) but returns `None` for any other
    /// `HostRuntime` impl. Red check for the RouterOS downcast hazard: make
    /// the helper wrap unconditionally and the second half of this test
    /// fails.
    #[tokio::test]
    async fn budget_runtime_for_update_gates_on_standard_runtime() {
        use uptrakit_plugin_infrastructure_core::{HostCapabilities, StandardHostRuntime};

        // StandardHostRuntime: wrapping succeeds, executor forwards the budget.
        let stub = Arc::new(RecordingExecutor {
            recorded: parking_lot::Mutex::new(None),
        });
        let standard: Arc<dyn HostRuntime> = Arc::new(StandardHostRuntime::new(
            stub.clone(),
            HostCapabilities::default(),
        ));
        let wrapped = budget_runtime_for_update(&standard, uptrakit_wire::DEFAULT_UPDATE_TIMEOUT)
            .expect("StandardHostRuntime must be wrapped");
        let (output_tx, _output_rx) = mpsc::channel(10);
        let spec = uptrakit_command::CommandSpec::shell("true");
        let _ = wrapped.executor().execute(&spec, &output_tx).await;

        let got = stub
            .recorded
            .lock()
            .take()
            .expect("stub should record spec");
        assert_eq!(got.timeout, Some(uptrakit_wire::DEFAULT_UPDATE_TIMEOUT));
        assert_eq!(
            got.abandonment,
            uptrakit_command::AbandonmentPolicy::DrainOnAbandon
        );

        // A non-StandardHostRuntime impl must pass through unwrapped.
        struct OtherRuntime {
            executor: Arc<dyn CommandExecutor>,
            capabilities: HostCapabilities,
        }
        impl HostRuntime for OtherRuntime {
            fn capabilities(&self) -> &HostCapabilities {
                &self.capabilities
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn executor(&self) -> Arc<dyn CommandExecutor> {
                Arc::clone(&self.executor)
            }
        }
        let other: Arc<dyn HostRuntime> = Arc::new(OtherRuntime {
            executor: Arc::new(uptrakit_command::NoopCommandExecutor),
            capabilities: HostCapabilities::default(),
        });
        assert!(
            budget_runtime_for_update(&other, uptrakit_wire::DEFAULT_UPDATE_TIMEOUT).is_none(),
            "non-StandardHostRuntime must pass through unwrapped"
        );
    }
}

#[cfg(all(test, feature = "interactive"))]
mod interactive_lifecycle_tests {
    use super::*;
    use std::time::Duration;
    use uptrakit_command::{CommandSpec, InteractiveHandle, SudoAwareCommandExecutor, SudoContext};

    /// `CommandExecutor` stub whose `execute_interactive` records the spec it
    /// received and always fails, so callers fall back to non-interactive
    /// execution without needing a real PTY.
    struct RecordingExecutor {
        recorded: parking_lot::Mutex<Option<CommandSpec>>,
    }

    #[async_trait::async_trait]
    impl CommandExecutor for RecordingExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
            Ok(uptrakit_command::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
            Ok(uptrakit_command::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }

        fn supports_interactive(&self) -> bool {
            true
        }

        async fn execute_interactive(
            &self,
            spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<InteractiveHandle> {
            *self.recorded.lock() = Some(spec.clone());
            Err(rootcause::report!(
                uptrakit_command::CommandError::UnsupportedOperation(
                    "recording stub never supports interactive execution".to_string()
                )
            ))
        }
    }

    /// Promotion fills `spec.timeout` from `update_timeout` only when the
    /// plugin left it `None`; a plugin-set timeout survives untouched. Also
    /// pins that the promoted timeout survives `SudoAwareCommandExecutor`'s
    /// `apply_sudo` transform — the recording stub sits *behind* the sudo
    /// layer, so observing the timeout there proves it reached the point
    /// `run_command_interactive` would receive it.
    #[tokio::test]
    async fn promotion_fills_timeout_and_survives_sudo_layer() {
        // Case 1: plugin left spec.timeout unset — promotion fills it in.
        let stub = Arc::new(RecordingExecutor {
            recorded: parking_lot::Mutex::new(None),
        });
        let sudo_wrapped: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
            stub.clone(),
            SudoContext::default(),
        ));
        let (channels_tx, _channels_rx) = tokio::sync::oneshot::channel();
        let forwarding = ForwardingInteractiveExecutor {
            inner: sudo_wrapped,
            channels_tx: parking_lot::Mutex::new(Some(channels_tx)),
            update_timeout: Duration::from_secs(123),
        };
        let (output_tx, _output_rx) = mpsc::channel(10);
        let spec = CommandSpec::shell("true");
        let _ = forwarding.execute(&spec, &output_tx).await;

        let got = stub
            .recorded
            .lock()
            .take()
            .expect("stub should record spec");
        assert_eq!(got.timeout, Some(Duration::from_secs(123)));

        // Case 2: plugin-set timeout survives promotion (not overwritten).
        let stub2 = Arc::new(RecordingExecutor {
            recorded: parking_lot::Mutex::new(None),
        });
        let sudo_wrapped2: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
            stub2.clone(),
            SudoContext::default(),
        ));
        let (channels_tx2, _channels_rx2) = tokio::sync::oneshot::channel();
        let forwarding2 = ForwardingInteractiveExecutor {
            inner: sudo_wrapped2,
            channels_tx: parking_lot::Mutex::new(Some(channels_tx2)),
            update_timeout: Duration::from_secs(123),
        };
        let spec_with_timeout = CommandSpec::shell("true").with_timeout(Duration::from_secs(5));
        let _ = forwarding2.execute(&spec_with_timeout, &output_tx).await;

        let got2 = stub2
            .recorded
            .lock()
            .take()
            .expect("stub should record spec");
        assert_eq!(got2.timeout, Some(Duration::from_secs(5)));
    }

    /// Red check for the :1187-equivalent fix: when `execute_interactive`
    /// fails, the non-interactive fallback must run the *promoted* spec
    /// (timeout filled from `update_timeout`), not the plugin's original
    /// unbudgeted spec. Revert that fix and this test fails.
    #[tokio::test]
    async fn interactive_fallback_uses_promoted_spec() {
        struct FallbackRecordingExecutor {
            recorded: parking_lot::Mutex<Option<CommandSpec>>,
        }

        #[async_trait::async_trait]
        impl CommandExecutor for FallbackRecordingExecutor {
            async fn execute(
                &self,
                spec: &CommandSpec,
                _output_tx: &mpsc::Sender<UpdateOutputLine>,
            ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
                *self.recorded.lock() = Some(spec.clone());
                Ok(uptrakit_command::CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            }

            async fn execute_quiet(
                &self,
                _spec: &CommandSpec,
            ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
                Ok(uptrakit_command::CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            }

            fn supports_interactive(&self) -> bool {
                true
            }

            async fn execute_interactive(
                &self,
                _spec: &CommandSpec,
                _output_tx: &mpsc::Sender<UpdateOutputLine>,
            ) -> uptrakit_command::Result<InteractiveHandle> {
                Err(rootcause::report!(
                    uptrakit_command::CommandError::UnsupportedOperation(
                        "interactive unavailable in test".to_string()
                    )
                ))
            }
        }

        let stub = Arc::new(FallbackRecordingExecutor {
            recorded: parking_lot::Mutex::new(None),
        });
        let (channels_tx, _channels_rx) = tokio::sync::oneshot::channel();
        let forwarding = ForwardingInteractiveExecutor {
            inner: stub.clone(),
            channels_tx: parking_lot::Mutex::new(Some(channels_tx)),
            update_timeout: Duration::from_secs(123),
        };
        let (output_tx, _output_rx) = mpsc::channel(10);
        let spec = CommandSpec::shell("true");
        let _ = forwarding.execute(&spec, &output_tx).await;

        let got = stub
            .recorded
            .lock()
            .take()
            .expect("fallback should record spec");
        assert_eq!(got.timeout, Some(Duration::from_secs(123)));
    }

    /// Red check for the `execute_quiet` fix: quiet execution through
    /// `ForwardingInteractiveExecutor` must also apply the update budget.
    #[tokio::test]
    async fn forwarding_execute_quiet_promotes_timeout() {
        struct QuietRecordingExecutor {
            recorded: parking_lot::Mutex<Option<CommandSpec>>,
        }

        #[async_trait::async_trait]
        impl CommandExecutor for QuietRecordingExecutor {
            async fn execute(
                &self,
                _spec: &CommandSpec,
                _output_tx: &mpsc::Sender<UpdateOutputLine>,
            ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
                Ok(uptrakit_command::CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            }

            async fn execute_quiet(
                &self,
                spec: &CommandSpec,
            ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
                *self.recorded.lock() = Some(spec.clone());
                Ok(uptrakit_command::CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            }
        }

        let stub = Arc::new(QuietRecordingExecutor {
            recorded: parking_lot::Mutex::new(None),
        });
        let (channels_tx, _channels_rx) = tokio::sync::oneshot::channel();
        let forwarding = ForwardingInteractiveExecutor {
            inner: stub.clone(),
            channels_tx: parking_lot::Mutex::new(Some(channels_tx)),
            update_timeout: Duration::from_secs(123),
        };
        let spec = CommandSpec::shell("true");
        let _ = forwarding.execute_quiet(&spec).await;

        let got = stub
            .recorded
            .lock()
            .take()
            .expect("stub should record spec");
        assert_eq!(got.timeout, Some(Duration::from_secs(123)));
    }

    /// Through `ForwardingInteractiveExecutor` with a real `LocalCommandExecutor`
    /// inner, cancel the pipeline future mid-await and assert the orphaned PTY
    /// child's process group is killed. Uses real wall-clock time deliberately
    /// (see AGENTS ledger #76): a real child's exit is OS-clock-driven, so
    /// `start_paused` would hang or false-green here. Skipped when the initial
    /// promotion errors (PTY unavailable in the sandbox).
    #[tokio::test]
    async fn cancelling_pipeline_group_kills_orphaned_pty_child() {
        use uptrakit_command::LocalCommandExecutor;

        let marker = format!("uptrakit-test-{}", uuid::Uuid::new_v4());
        let local: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
        let (channels_tx, _channels_rx) = tokio::sync::oneshot::channel();
        let forwarding = Arc::new(ForwardingInteractiveExecutor {
            inner: local,
            channels_tx: parking_lot::Mutex::new(Some(channels_tx)),
            update_timeout: Duration::from_secs(300),
        });

        let (output_tx, mut output_rx) = mpsc::channel(100);
        // trap TERM so only a real SIGKILL (via group-kill) can end the child;
        // marker makes the process uniquely identifiable via pgrep.
        let spec = CommandSpec::shell(format!("trap '' TERM; sleep 300 # {marker}"));

        let exec_future = {
            let forwarding = Arc::clone(&forwarding);
            async move { forwarding.execute(&spec, &output_tx).await }
        };

        let timed = tokio::time::timeout(Duration::from_millis(500), exec_future).await;
        // Drain (and drop) the receiver so the channel doesn't back up; not
        // asserted on, this test cares about process lifecycle only.
        output_rx.close();
        while output_rx.try_recv().is_ok() {}

        if timed.is_ok() {
            // Interactive execution finished within 500ms — PTY unavailable or
            // command failed fast; nothing meaningful to assert. Skip.
            return;
        }

        // The exec future (and the InteractiveSessionGuard inside it) was
        // dropped when tokio::time::timeout cancelled it. Poll for the marked
        // process to disappear, bounded so a real failure doesn't hang the run.
        let mut still_alive = true;
        for _ in 0..50 {
            let found = std::process::Command::new("pgrep")
                .arg("-f")
                .arg(&marker)
                .output();
            match found {
                Ok(out) if !out.stdout.is_empty() => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                _ => {
                    still_alive = false;
                    break;
                }
            }
        }

        assert!(
            !still_alive,
            "orphaned PTY child (marker {marker}) should have been group-killed on cancellation"
        );
    }

    /// Local copy of `tests::test_payload` — the parent helper is private to
    /// the sibling `tests` module and not visible here.
    fn test_payload() -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            host_machine_id: String::new(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "Test App".to_string(),
            to_version: "2.0.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: PluginAssignment {
                plugin_type: uptrakit_wire::plugin_ids::RELEASES_GITHUB.clone(),
                package_identifier: "test-app".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            release_info: None,
            timeout: Duration::from_secs(60),
            interactive: false,
        }
    }

    /// Local copy of `tests::test_runtime` — see `test_payload` above.
    fn test_runtime() -> Arc<dyn HostRuntime> {
        use uptrakit_command::LocalCommandExecutor;
        use uptrakit_plugin_infrastructure_core::{HostCapabilities, StandardHostRuntime};
        Arc::new(StandardHostRuntime::new(
            Arc::new(LocalCommandExecutor),
            HostCapabilities::default(),
        ))
    }

    /// Builds a `PluginAssignment` for the test-support stub lifecycle hook,
    /// configured to stream `line_count` output lines from its pre-hook.
    fn stub_hook_assignment(line_count: usize) -> PluginAssignment {
        PluginAssignment {
            plugin_type: uptrakit_shared_types::plugin_ids::TEST_LIFECYCLE_HOOK,
            package_identifier: "test-lifecycle-hook".to_string(),
            config: serde_json::json!({ "line_count": line_count }),
        }
    }

    /// Fix 1 regression (deadlock): a pre-hook emitting more lines than the
    /// output channel's bounded capacity (100, see `execute_update_pipeline`)
    /// must not wedge `execute_update_interactive`. The function must return
    /// a handle immediately — before the hook finishes streaming — so the
    /// caller can drain `output_rx` concurrently with awaiting completion.
    ///
    /// Real-clock `tokio::time::timeout` (AGENTS ledger #76 exception,
    /// documented per the standards snapshot's `start_paused` rule): the
    /// wedge under test is CHANNEL-CAPACITY-parked (`send().await` on the
    /// full bounded-100 channel), not timer-parked. Paused-time auto-advance
    /// semantics around channel-parked tasks are unreliable in both
    /// directions — the virtual clock may never advance while a task looks
    /// runnable, or may jump through the guard timeout during a brief
    /// all-parked instant on otherwise-healthy code. A real 5s bound is
    /// deterministic instead: healthy code finishes in milliseconds, and the
    /// pre-fix regression (handle awaited only after full drain, or the
    /// pipeline blocked on `send().await` with no reader) fails the 5s bound.
    #[tokio::test]
    async fn interactive_update_returns_handle_before_hook_finishes_streaming() {
        let mut payload = test_payload();
        payload.interactive = true;
        payload.release_info = None; // attestation gate must do no network I/O
        payload.pre_update_hook_plugins = vec![stub_hook_assignment(150)];
        // A trivial local update command — RELEASES_GITHUB's real executor
        // would attempt network I/O in `execute_update`, which is unrelated
        // to what this test pins (channel-capacity deadlock in the hook).
        payload.execute_update_plugin = PluginAssignment {
            plugin_type: uptrakit_wire::plugin_ids::GENERIC_SHELL.clone(),
            package_identifier: "test-app".to_string(),
            config: serde_json::json!({ "update_command": "true" }),
        };

        let runtime = test_runtime();
        let (output_tx, mut output_rx) = mpsc::channel::<UpdateOutputMessage>(100);
        let (early_result_tx, _early_result_rx) = tokio::sync::mpsc::unbounded_channel();

        let test_body = async {
            // Call returns immediately with a handle — obtained BEFORE any
            // draining of output_rx below, proving the fn didn't block while
            // the 150-line hook was still streaming into the bounded-100
            // channel.
            let interactive_handle =
                execute_update_interactive(payload, runtime, output_tx, early_result_tx);

            // Drain output concurrently with awaiting the handle. Without the
            // fix, nothing would ever drain the channel here in a real
            // caller — but even in this test, if `execute_update_interactive`
            // itself blocked before returning, we'd never reach this point at
            // all within the outer timeout.
            let drain = async {
                let mut count = 0usize;
                while output_rx.recv().await.is_some() {
                    count += 1;
                }
                count
            };

            let (drained, join_result) = tokio::join!(drain, interactive_handle.handle);
            (drained, join_result)
        };

        let (drained, join_result) = tokio::time::timeout(Duration::from_secs(5), test_body)
            .await
            .expect(
                "execute_update_interactive deadlocked: pre-hook's 150 lines exceeded the \
                 bounded-100 output channel and nothing drained it within 5s (Fix 1 regression)",
            );

        let exec_result = join_result.expect("update task should not panic");
        assert!(
            drained >= 150,
            "expected at least the hook's 150 output lines to be drained, got {drained}"
        );
        assert_eq!(
            exec_result.result.status,
            UpdateFinalStatus::Completed,
            "update should complete successfully once the hook finishes streaming"
        );
    }

    /// Fix 2 regression (PTY targeting): only the update command's `execute()`
    /// call may be promoted to `execute_interactive()` — a lifecycle hook has
    /// no executor/runtime accessor on `UpdateLifecycleContext` (verified:
    /// `plugin-infrastructure-core/src/traits.rs`) and therefore can never
    /// itself trigger PTY promotion. This test pins that behavior end-to-end
    /// through `execute_update_interactive`: a pre-hook (1 output line) runs
    /// before a `generic.shell` update command, and the recording executor
    /// installed as the base runtime's executor must observe exactly one
    /// `execute_interactive` call, carrying the UPDATE command's spec.
    #[tokio::test]
    async fn interactive_promotion_targets_update_command_not_hook() {
        /// Records every `execute`/`execute_interactive` call as
        /// `"<method>:<spec-summary>"`, then always succeeds so the pipeline
        /// can run to completion without a real PTY.
        struct RecordingUpdateExecutor {
            calls: parking_lot::Mutex<Vec<String>>,
        }

        fn spec_summary(spec: &CommandSpec) -> String {
            format!("{:?}", spec.mode)
        }

        #[async_trait::async_trait]
        impl CommandExecutor for RecordingUpdateExecutor {
            async fn execute(
                &self,
                spec: &CommandSpec,
                _output_tx: &mpsc::Sender<UpdateOutputLine>,
            ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
                self.calls
                    .lock()
                    .push(format!("execute:{}", spec_summary(spec)));
                Ok(uptrakit_command::CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            }

            async fn execute_quiet(
                &self,
                _spec: &CommandSpec,
            ) -> uptrakit_command::Result<uptrakit_command::CommandOutput> {
                Ok(uptrakit_command::CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            }

            fn supports_interactive(&self) -> bool {
                true
            }

            async fn execute_interactive(
                &self,
                spec: &CommandSpec,
                _output_tx: &mpsc::Sender<UpdateOutputLine>,
            ) -> uptrakit_command::Result<InteractiveHandle> {
                self.calls
                    .lock()
                    .push(format!("execute_interactive:{}", spec_summary(spec)));
                let (stdin_tx, _stdin_rx) = mpsc::channel(1);
                let (signal_tx, _signal_rx) = mpsc::channel(1);
                let (_attention_tx, attention_rx) = mpsc::channel(1);
                Ok(InteractiveHandle {
                    // Safe no-op target: `kill_process_group(0)` is a verified
                    // no-op, so a fabricated pid is safe if the guard ever fires.
                    child_pid: 0,
                    stdin_tx,
                    signal_tx,
                    completion: tokio::spawn(async {
                        Ok(uptrakit_command::CommandOutput {
                            output: String::new(),
                            exit_code: 0,
                        })
                    }),
                    attention_rx,
                })
            }
        }

        let recording = Arc::new(RecordingUpdateExecutor {
            calls: parking_lot::Mutex::new(Vec::new()),
        });
        let recording_dyn: Arc<dyn CommandExecutor> = recording.clone();
        let runtime = uptrakit_plugin_infrastructure_registry::construct_host_runtime(
            recording_dyn,
            uptrakit_plugin_infrastructure_core::HostCapabilities::default(),
        );

        let mut payload = test_payload();
        payload.interactive = true;
        payload.release_info = None;
        payload.pre_update_hook_plugins = vec![stub_hook_assignment(1)];
        payload.execute_update_plugin = PluginAssignment {
            plugin_type: uptrakit_wire::plugin_ids::GENERIC_SHELL.clone(),
            package_identifier: "test-app".to_string(),
            config: serde_json::json!({ "update_command": "true" }),
        };

        let (output_tx, mut output_rx) = mpsc::channel::<UpdateOutputMessage>(100);
        let (early_result_tx, _early_result_rx) = tokio::sync::mpsc::unbounded_channel();

        let mut interactive_handle =
            execute_update_interactive(payload, runtime, output_tx, early_result_tx);

        // Prove the hook's participation via its output line appearing before
        // the update command's promotion, by observing it prior to awaiting
        // channels_rx (which resolves only once the update command reaches
        // the forwarding executor's execute() call).
        let mut hook_line_seen = false;
        let mut saw_channels = false;
        let mut output_closed = false;
        let test_body = async {
            // First, drain strictly until the hook's own line is observed.
            // The pipeline runs pre-hooks fully before the update command
            // (`execute_update_pipeline`: hooks -> attestation -> plugin
            // execute), so the hook's line is always sent, and always sent
            // before the update command's `execute()` call that triggers PTY
            // promotion — waiting for it here (before racing channels_rx)
            // proves that ordering rather than assuming it.
            while !hook_line_seen {
                match output_rx.recv().await {
                    Some(line) if line.output.contains("test-lifecycle-hook") => {
                        hook_line_seen = true;
                    }
                    Some(_) => {}
                    None => panic!("output_rx closed before the hook's line was observed"),
                }
            }

            // Now race draining the rest of the output against channels_rx
            // resolving — both must complete, in either order.
            while !output_closed || !saw_channels {
                tokio::select! {
                    maybe_line = output_rx.recv(), if !output_closed => {
                        if maybe_line.is_none() {
                            output_closed = true;
                        }
                    }
                    channels = &mut interactive_handle.channels_rx, if !saw_channels => {
                        channels.expect(
                            "channels_rx should resolve Ok: promotion targets the update \
                             command, not the hook (Fix 2 regression)",
                        );
                        saw_channels = true;
                    }
                }
            }
        };

        tokio::time::timeout(Duration::from_secs(5), test_body)
            .await
            .expect("pipeline did not complete within 5s");

        interactive_handle
            .handle
            .await
            .expect("update task should not panic");

        let calls = recording.calls.lock().clone();
        let interactive_calls: Vec<&String> = calls
            .iter()
            .filter(|c| c.starts_with("execute_interactive:"))
            .collect();
        assert_eq!(
            interactive_calls.len(),
            1,
            "expected exactly one execute_interactive call (the update command); got: {calls:?}"
        );
        assert!(
            interactive_calls[0].contains("true"),
            "the single execute_interactive call should carry the UPDATE command's spec \
             (shell \"true\"), not the hook's; got: {}",
            interactive_calls[0]
        );
    }
}

#[cfg(test)]
mod pipeline_resumable_tests {
    use super::*;

    #[test]
    fn test_pipeline_result_struct_exists() {
        let _ = PipelineResult {
            succeeded: true,
            resumable: true,
        };
        let _ = PipelineResult {
            succeeded: false,
            resumable: false,
        };
    }
}
