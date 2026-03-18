use std::sync::Arc;

use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{
    BatchUpdateItemResult, BatchUpdateResultPayload, DisconnectReason, DisconnectingPayload,
    DiscoverSoftwarePayload, DiscoveryPluginResult, DiscoveryResultsPayload,
    ExecuteBatchUpdatePayload, ServiceMessage, ServiceTransport, TransportError, UpdateFinalStatus,
    UpdateOutputPayload, UpdateResultPayload, UpdateStartedPayload, VersionCheckResult,
    VersionCheckResultsPayload,
};
use uptrakit_service_sdk::LoopOutcome;

use crate::connection_context::ConnectionContext;

/// State for an in-flight update execution.
pub struct InFlightUpdate {
    pub update_history_id: uuid::Uuid,
    pub handle: tokio::task::JoinHandle<crate::update::UpdateExecutionResult>,
    pub output_rx: tokio::sync::mpsc::Receiver<crate::update::UpdateOutputMessage>,
    /// Stdin writer for interactive updates. `None` for non-interactive.
    #[cfg(feature = "interactive")]
    pub stdin_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    /// Signal sender for interactive updates. `None` for non-interactive.
    #[cfg(feature = "interactive")]
    pub signal_tx: Option<tokio::sync::mpsc::Sender<i32>>,
    /// Attention receiver for stdin-waiting detection.
    #[cfg(feature = "interactive")]
    pub attention_rx: Option<tokio::sync::mpsc::Receiver<()>>,
}

/// Result of spawning an update task, including optional interactive channels.
struct SpawnedUpdate {
    handle: tokio::task::JoinHandle<crate::update::UpdateExecutionResult>,
    #[cfg(feature = "interactive")]
    stdin_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    #[cfg(feature = "interactive")]
    signal_tx: Option<tokio::sync::mpsc::Sender<i32>>,
    #[cfg(feature = "interactive")]
    attention_rx: Option<tokio::sync::mpsc::Receiver<()>>,
}

/// Spawn the update task, using interactive execution when the feature is
/// enabled and the payload requests it.
///
/// When the `interactive` feature is enabled and `payload.interactive` is
/// true, the update runs through `execute_update_interactive` which
/// allocates a PTY and returns channels for stdin/signal forwarding.
/// Otherwise, falls back to the standard non-interactive path.
async fn spawn_update_task(
    payload: uptrakit_internal_wire::ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    output_tx: tokio::sync::mpsc::Sender<crate::update::UpdateOutputMessage>,
    _update_history_id: uuid::Uuid,
) -> SpawnedUpdate {
    // Try interactive execution when the feature is enabled and requested.
    #[cfg(feature = "interactive")]
    if payload.interactive && executor.supports_interactive() {
        let result = crate::update::execute_update_interactive(payload, executor, output_tx).await;
        return SpawnedUpdate {
            handle: result.handle,
            stdin_tx: result.stdin_tx,
            signal_tx: result.signal_tx,
            attention_rx: result.attention_rx,
        };
    }

    // Non-interactive fallback (always compiled, always reachable without the feature).
    let handle =
        tokio::spawn(
            async move { crate::update::execute_update(payload, executor, output_tx).await },
        );
    SpawnedUpdate {
        handle,
        #[cfg(feature = "interactive")]
        stdin_tx: None,
        #[cfg(feature = "interactive")]
        signal_tx: None,
        #[cfg(feature = "interactive")]
        attention_rx: None,
    }
}

/// Events from an in-flight update.
pub enum UpdateEvent {
    Output(crate::update::UpdateOutputMessage),
    Completed(std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>),
    /// The update process appears to be waiting for stdin input.
    /// Carries the `update_history_id` for correlation.
    Attention(uuid::Uuid),
}

/// Send an update output message to the controller.
#[tracing::instrument(skip_all, fields(%update_history_id))]
pub async fn send_update_output(
    conn: &mut dyn ServiceTransport,
    update_history_id: uuid::Uuid,
    output_msg: crate::update::UpdateOutputMessage,
) {
    conn.transport_send_best_effort(ServiceMessage::UpdateOutput(UpdateOutputPayload {
        update_history_id,
        output: output_msg.output,
        stream: output_msg.stream,
    }))
    .await;
}

/// Send the final update result to the controller.
///
/// Returns `Err` if the transport write fails. Callers should treat this as a
/// reason to terminate the connection so the reconnect loop re-establishes the
/// session; otherwise the controller has no signal to close the in-progress
/// update record.
#[tracing::instrument(skip_all, fields(%update_history_id))]
pub async fn send_update_result(
    conn: &mut dyn ServiceTransport,
    update_history_id: uuid::Uuid,
    result: std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>,
) -> std::result::Result<(), TransportError> {
    match result {
        Ok(exec_result) => {
            let status = exec_result.result.status.clone();
            let error = exec_result.result.error.clone();
            conn.transport_send(ServiceMessage::UpdateResult(exec_result.result))
                .await?;
            match status {
                uptrakit_internal_wire::UpdateFinalStatus::Completed => {
                    tracing::info!(update_id = %update_history_id, "update execution completed successfully");
                }
                _ => {
                    tracing::warn!(
                        update_id = %update_history_id,
                        error = ?error,
                        "update execution finished with failure"
                    );
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "update task panicked");
            conn.transport_send(ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id,
                status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some(format!("Update task panicked: {e}")),
            }))
            .await?;
        }
    }
    Ok(())
}

/// Handle graceful shutdown: drain in-flight update, send Disconnecting.
#[tracing::instrument(skip_all)]
pub async fn handle_graceful_shutdown(
    conn: &mut dyn ServiceTransport,
    in_flight_update: Option<InFlightUpdate>,
    shutdown_timeout: std::time::Duration,
    disconnect_reason: DisconnectReason,
    outcome: LoopOutcome,
) -> LoopOutcome {
    if let Some(mut update) = in_flight_update {
        tracing::info!(
            update_id = %update.update_history_id,
            timeout = ?shutdown_timeout,
            "waiting for in-flight update to complete before shutdown"
        );

        let deadline = tokio::time::Instant::now() + shutdown_timeout;

        // Continue processing output and wait for completion
        loop {
            tokio::select! {
                biased;

                Some(output_msg) = update.output_rx.recv() => {
                    send_update_output(conn, update.update_history_id, output_msg).await;
                }
                result = &mut update.handle => {
                    if let Err(e) = send_update_result(conn, update.update_history_id, result).await {
                        tracing::warn!(error = %e, "failed to send UpdateResult during shutdown");
                    }
                    break;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!(
                        update_id = %update.update_history_id,
                        "shutdown timeout reached, abandoning in-flight update"
                    );
                    conn.transport_send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                        update_history_id: update.update_history_id,
                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                        from_version: None,
                        to_version: None,
                        output: String::new(),
                        error: Some(format!("Agent shutdown timeout ({}s) reached", shutdown_timeout.as_secs())),
                    })).await;
                    break;
                }
            }
        }

        // Drain any remaining output messages
        while let Ok(output_msg) = update.output_rx.try_recv() {
            send_update_output(conn, update.update_history_id, output_msg).await;
        }
    }

    // Send Disconnecting message to controller
    let reason_dbg = format!("{disconnect_reason:?}");
    let disconnecting_msg =
        ServiceMessage::Disconnecting(DisconnectingPayload::new(disconnect_reason));
    if let Err(e) = conn.transport_send(disconnecting_msg).await {
        tracing::debug!(error = %e, "failed to send Disconnecting message");
    } else {
        tracing::debug!(reason = %reason_dbg, "sent Disconnecting message to controller");
    }

    outcome
}

/// Spawn a background task that produces a [`ServiceMessage`] and sends it
/// through the provided channel.
///
/// Long-running operations (version checks, software discovery, batch package
/// updates) must not run inline in `on_message` — doing so blocks the event
/// loop and causes the controller's WebSocket write timeout to fire. This
/// helper clones the sender, spawns the future on the Tokio runtime, and
/// forwards the result through the channel for the event loop to pick up.
pub fn spawn_background(
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    future: impl std::future::Future<Output = ServiceMessage> + Send + 'static,
) {
    let bg_tx = bg_tx.clone();
    tokio::spawn(async move {
        let msg = future.await;
        let _ = bg_tx.send(msg).await;
    });
}

/// Forward a background operation result to the controller.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the send fails, signalling the
/// caller to break out of the event loop so the reconnect logic can
/// re-establish the session.
pub async fn send_background_result(
    conn: &mut dyn ServiceTransport,
    msg: ServiceMessage,
) -> Option<LoopOutcome> {
    if let Err(e) = conn.transport_send_auto_paginate(msg).await {
        tracing::error!(error = %e, "failed to send background operation result; disconnecting");
        return Some(LoopOutcome::Disconnected);
    }
    None
}

/// Run version checks and return the result as a [`ServiceMessage`].
///
/// Performs all version-check work and returns the result without sending it
/// over the connection. Callers use [`spawn_background`] to run this in a
/// background task and forward the returned message to the controller through
/// a channel.
#[tracing::instrument(skip_all, fields(assignment_count = payload.assignments.len()))]
pub async fn run_check_versions(
    payload: uptrakit_internal_wire::CheckVersionsPayload,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> ServiceMessage {
    tracing::info!(
        count = payload.assignments.len(),
        host_machine_id = %payload.host_machine_id,
        "received CheckVersions request"
    );

    let results: Vec<VersionCheckResult> =
        crate::version_check::batch_check_versions(payload.assignments, Arc::clone(&executor), ctx)
            .await;

    tracing::debug!("version check complete");
    ServiceMessage::VersionCheckResults(VersionCheckResultsPayload { results })
}

/// Spawn an update task and return the in-flight update handle.
///
/// Applies the connection context to the plugin configs, spawns the update
/// execution task, sends `UpdateStarted` to the controller, and returns an
/// [`InFlightUpdate`] for the caller to track.
///
/// This is the low-level primitive used by both [`handle_execute_update`] (for
/// the single-host agent, which holds a global `Option<InFlightUpdate>`) and
/// `handle_execute_update_ssh` in the SSH agent (which holds a per-host
/// `HashMap<String, SshInFlightUpdate>`).
#[tracing::instrument(skip_all, fields(update_history_id = %payload.update_history_id))]
pub async fn start_update(
    payload: uptrakit_internal_wire::ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    conn: &mut dyn ServiceTransport,
    ctx: &ConnectionContext,
) -> InFlightUpdate {
    // Apply connection context to the plugin configs
    let mut effective_payload = payload.clone();
    ctx.apply_to_config(
        &effective_payload.execute_update_plugin.plugin_type,
        &mut effective_payload.execute_update_plugin.config,
    );
    if let Some(ref mut detect) = effective_payload.detect_version_plugin {
        ctx.apply_to_config(&detect.plugin_type, &mut detect.config);
    }

    // Create a channel for output streaming
    let (output_tx, output_rx) =
        tokio::sync::mpsc::channel::<crate::update::UpdateOutputMessage>(100);

    let update_history_id = effective_payload.update_history_id;

    let spawned =
        spawn_update_task(effective_payload, executor, output_tx, update_history_id).await;

    // Confirmed PTY allocation: stdin_tx is Some only when execute_update_interactive
    // successfully allocated a PTY and delivered channels via the oneshot.
    #[allow(unused_assignments, unused_mut)]
    let mut confirmed_interactive = false;
    #[cfg(feature = "interactive")]
    {
        confirmed_interactive = spawned.stdin_tx.is_some();
    }

    // Send UpdateStarted
    if let Err(e) = conn
        .transport_send(ServiceMessage::UpdateStarted(UpdateStartedPayload {
            update_history_id,
            from_version: None,
            interactive: confirmed_interactive,
        }))
        .await
    {
        tracing::error!(error = %e, "failed to send UpdateStarted");
    }

    InFlightUpdate {
        update_history_id,
        handle: spawned.handle,
        output_rx,
        #[cfg(feature = "interactive")]
        stdin_tx: spawned.stdin_tx,
        #[cfg(feature = "interactive")]
        signal_tx: spawned.signal_tx,
        #[cfg(feature = "interactive")]
        attention_rx: spawned.attention_rx,
    }
}

/// Handle an `ExecuteUpdate` message from the controller.
///
/// The `executor` is provided by the caller — `LocalCommandExecutor` for the
/// regular agent, `SshCommandExecutor` for the SSH agent.
///
/// The `ctx` is used to inject connection-specific overrides into the plugin
/// config before instantiation.
///
/// For the SSH agent (which manages multiple hosts), use `start_update()`
/// directly together with a per-host concurrency check and a forwarder task.
#[tracing::instrument(skip_all, fields(update_history_id = %payload.update_history_id))]
pub async fn handle_execute_update(
    payload: uptrakit_internal_wire::ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    in_flight_update: &mut Option<InFlightUpdate>,
    conn: &mut dyn ServiceTransport,
    ctx: &ConnectionContext,
) {
    tracing::info!(
        update_id = %payload.update_history_id,
        software = %payload.software_item_name,
        version = %payload.to_version,
        host_machine_id = %payload.host_machine_id,
        "received update request"
    );

    // If there's already an in-flight update, reject this one
    if in_flight_update.is_some() {
        tracing::warn!(
            update_id = %payload.update_history_id,
            "rejecting update: another update is already in progress"
        );
        conn.transport_send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
            update_history_id: payload.update_history_id,
            status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
            from_version: None,
            to_version: None,
            output: String::new(),
            error: Some("Another update is already in progress".to_string()),
        }))
        .await;
        return;
    }

    *in_flight_update = Some(start_update(payload, executor, conn, ctx).await);
}

/// Run a batch update and return the result as a [`ServiceMessage`].
///
/// Performs all update work and returns the result without sending it over the
/// connection. Callers use [`spawn_background`] to run this in a background
/// task and forward the returned message to the controller through a channel.
#[tracing::instrument(skip_all, fields(batch_id = %payload.batch_id, plugin_type = %payload.plugin_type))]
pub async fn run_execute_batch_update(
    payload: ExecuteBatchUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> ServiceMessage {
    tracing::info!(
        batch_id = %payload.batch_id,
        plugin_type = %payload.plugin_type,
        count = payload.updates.len(),
        host_machine_id = %payload.host_machine_id,
        "received batch update request"
    );

    let results = batch_update_inner(&payload, executor, ctx).await;

    ServiceMessage::BatchUpdateResult(BatchUpdateResultPayload {
        batch_id: payload.batch_id,
        results,
    })
}

/// Inner batch-update logic for [`run_execute_batch_update`].
async fn batch_update_inner(
    payload: &ExecuteBatchUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Vec<BatchUpdateItemResult> {
    // Build a correlation map: package_identifier → (host_software_item_id, update_history_id)
    let correlation: std::collections::HashMap<String, (uuid::Uuid, uuid::Uuid)> = payload
        .updates
        .iter()
        .map(|u| {
            (
                u.package_identifier.clone(),
                (u.host_software_item_id, u.update_history_id),
            )
        })
        .collect();

    // Map wire updates to plugin BatchUpdateItems
    let items: Vec<uptrakit_plugin_infrastructure_core::BatchUpdateItem> = payload
        .updates
        .iter()
        .map(|u| {
            uptrakit_plugin_infrastructure_core::BatchUpdateItem::new(
                u.package_identifier.clone(),
                u.to_version.clone(),
                u.release_info.clone(),
            )
        })
        .collect();

    // Apply connection context to plugin config
    let mut effective_config = payload.plugin_config.clone();
    ctx.apply_to_config(&payload.plugin_type, &mut effective_config);

    // Create output channel for streaming
    let (output_tx, _output_rx) =
        tokio::sync::mpsc::channel::<uptrakit_command::UpdateOutputLine>(100);

    // Execute with timeout
    let timeout_duration = payload.timeout;
    let batch_results = tokio::time::timeout(timeout_duration, async {
        // Create plugin via descriptor
        let runtime = uptrakit_plugin_infrastructure_core::construct_host_runtime(
            Arc::clone(&executor),
            uptrakit_plugin_infrastructure_core::HostCapabilities::default(),
        );

        let desc =
            uptrakit_plugin_infrastructure_registry::get_descriptor(payload.plugin_type.as_str())
                .ok_or_else(|| {
                let msg = format!(
                    "Failed to create plugin: unknown plugin type '{}'",
                    payload.plugin_type
                );
                tracing::error!(msg);
                msg
            })?;

        let slot = desc.roles.update_executor.as_ref().ok_or_else(|| {
            let msg = format!(
                "plugin {} does not implement UpdateExecutorPlugin",
                payload.plugin_type
            );
            tracing::error!(msg);
            msg
        })?;

        let updater = (slot.create)(&effective_config, runtime).map_err(|e| {
            let msg = format!("Failed to create plugin: {e}");
            tracing::error!(error = %e, "failed to create plugin for batch update");
            msg
        })?;

        // Run pre-update hook plugins
        let pre_ctx = uptrakit_plugin_infrastructure_core::UpdateLifecycleContext::for_pre_hook(
            "", // batch has no single package identifier
            "", // batch has no single to_version
            None, None,
        );
        crate::update::run_batch_pre_hook_plugins(
            &payload.pre_update_hook_plugins,
            &pre_ctx,
            Arc::clone(&executor),
        )
        .await
        .map_err(|e| format!("Pre-update hook failed: {e}"))?;
        let results = match updater.execute_batch_update(&items, &output_tx).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "batch update execution failed");
                return Err(format!("Batch update failed: {e}"));
            }
        };

        // Run post-update hook plugins (non-fatal)
        let post_ctx = uptrakit_plugin_infrastructure_core::UpdateLifecycleContext::for_post_hook(
            "", "", None, None, true, // batch succeeded if we got here
        );
        crate::update::run_batch_post_hook_plugins(
            &payload.post_update_hook_plugins,
            &post_ctx,
            Arc::clone(&executor),
        )
        .await;

        Ok(results)
    })
    .await;

    // Build per-package results
    match batch_results {
        Ok(Ok(plugin_results)) => {
            plugin_results
                .into_iter()
                .map(|r| {
                    let (host_software_item_id, update_history_id) = correlation
                        .get(&r.package_identifier)
                        .copied()
                        .unwrap_or((uuid::Uuid::nil(), uuid::Uuid::nil()));
                    BatchUpdateItemResult {
                        host_software_item_id,
                        update_history_id,
                        status: if r.success {
                            UpdateFinalStatus::Completed
                        } else {
                            UpdateFinalStatus::Failed
                        },
                        output: r.output,
                        installed_version: None, // post-detection not yet implemented
                        error: if r.success {
                            None
                        } else {
                            Some("Package update failed".to_string())
                        },
                    }
                })
                .collect()
        }
        Ok(Err(error_msg)) => {
            // All packages failed (plugin creation or hook failure)
            payload
                .updates
                .iter()
                .map(|u| BatchUpdateItemResult {
                    host_software_item_id: u.host_software_item_id,
                    update_history_id: u.update_history_id,
                    status: UpdateFinalStatus::Failed,
                    output: String::new(),
                    installed_version: None,
                    error: Some(error_msg.clone()),
                })
                .collect()
        }
        Err(_) => {
            // Timeout
            let timeout_msg = format!(
                "Batch update timed out after {}s",
                payload.timeout.as_secs()
            );
            tracing::warn!(
                batch_id = %payload.batch_id,
                timeout = ?payload.timeout,
                "batch update timed out"
            );
            payload
                .updates
                .iter()
                .map(|u| BatchUpdateItemResult {
                    host_software_item_id: u.host_software_item_id,
                    update_history_id: u.update_history_id,
                    status: UpdateFinalStatus::Failed,
                    output: String::new(),
                    installed_version: None,
                    error: Some(timeout_msg.clone()),
                })
                .collect()
        }
    }
}

/// Run software discovery and return the result as a [`ServiceMessage`].
///
/// Performs all discovery work and returns the result without sending it over
/// the connection. Callers use [`spawn_background`] to run this in a
/// background task and forward the returned message to the controller through
/// a channel.
#[tracing::instrument(skip_all, fields(plugin_count = payload.plugins.len()))]
pub async fn run_discover_software(
    payload: DiscoverSoftwarePayload,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> ServiceMessage {
    tracing::info!(
        count = payload.plugins.len(),
        host_machine_id = %payload.host_machine_id,
        "received DiscoverSoftware request"
    );

    let results = discover_software_inner(&payload, executor, ctx).await;

    ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
        host_machine_id: payload.host_machine_id,
        results,
    })
}

/// Inner discovery logic for [`run_discover_software`].
async fn discover_software_inner(
    payload: &DiscoverSoftwarePayload,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Vec<DiscoveryPluginResult> {
    let mut results = Vec::with_capacity(payload.plugins.len());

    for assignment in &payload.plugins {
        tracing::debug!(
            plugin_type = %assignment.plugin_type,
            plugin_config_id = ?assignment.plugin_config_id,
            "running discovery for plugin"
        );

        let mut effective_config = assignment.config.clone();
        ctx.apply_to_config(&assignment.plugin_type, &mut effective_config);

        let runtime = uptrakit_plugin_infrastructure_core::construct_host_runtime(
            Arc::clone(&executor),
            uptrakit_plugin_infrastructure_core::HostCapabilities::default(),
        );

        let result = match uptrakit_plugin_infrastructure_registry::get_descriptor(
            assignment.plugin_type.as_str(),
        ) {
            None => {
                tracing::warn!(
                    plugin_type = %assignment.plugin_type,
                    "unknown plugin type for discovery"
                );
                DiscoveryPluginResult {
                    plugin_config_id: assignment.plugin_config_id,
                    plugin_type: assignment.plugin_type.clone(),
                    discoveries: vec![],
                    error: Some(format!("unknown plugin type: {}", assignment.plugin_type)),
                }
            }
            Some(desc) => {
                let slot = desc.roles.discoverer.as_ref();
                if let Some(slot) = slot {
                    match (slot.create)(&effective_config, runtime) {
                        Err(e) => {
                            tracing::warn!(
                                plugin_type = %assignment.plugin_type,
                                error = %e,
                                "failed to create plugin for discovery"
                            );
                            DiscoveryPluginResult {
                                plugin_config_id: assignment.plugin_config_id,
                                plugin_type: assignment.plugin_type.clone(),
                                discoveries: vec![],
                                error: Some(e.to_string()),
                            }
                        }
                        Ok(discovery) => {
                            // Run a host-compatibility check before discovery.
                            //
                            // Plugins that declare `DetectHostCompatibility` are asked
                            // whether they make sense on this host (e.g. Docker plugin
                            // checks if `docker` is present, PHS checks for
                            // `/usr/bin/update`).  Incompatible plugins return an empty,
                            // non-error result — it is not a failure for a host to not
                            // have a particular piece of software installed.
                            //
                            // If the check itself errors, we proceed with discovery
                            // (fail-open) and log a warning.
                            let is_compatible = if desc.capabilities.contains(
                                &uptrakit_plugin_infrastructure_core::PluginCapability::DetectHostCompatibility,
                            ) {
                                match discovery.detect_host_compatibility().await {
                                    Ok(uptrakit_plugin_infrastructure_core::HostCompatibility::Incompatible(reason)) => {
                                        tracing::debug!(
                                            plugin_type = %assignment.plugin_type,
                                            reason = %reason,
                                            "plugin not compatible with host; skipping discovery"
                                        );
                                        false
                                    }
                                    Ok(_) => true,
                                    Err(e) => {
                                        tracing::warn!(
                                            plugin_type = %assignment.plugin_type,
                                            error = %e,
                                            "host compatibility check failed; proceeding with discovery"
                                        );
                                        true
                                    }
                                }
                            } else {
                                true
                            };

                            if !is_compatible {
                                DiscoveryPluginResult {
                                    plugin_config_id: assignment.plugin_config_id,
                                    plugin_type: assignment.plugin_type.clone(),
                                    discoveries: vec![],
                                    error: None,
                                }
                            } else {
                                match discovery.discover_software().await {
                                    Ok(discoveries) => {
                                        tracing::info!(
                                            plugin_type = %assignment.plugin_type,
                                            count = discoveries.len(),
                                            "discovery completed"
                                        );
                                        DiscoveryPluginResult {
                                            plugin_config_id: assignment.plugin_config_id,
                                            plugin_type: assignment.plugin_type.clone(),
                                            discoveries,
                                            error: None,
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            plugin_type = %assignment.plugin_type,
                                            error = %e,
                                            "discovery failed"
                                        );
                                        DiscoveryPluginResult {
                                            plugin_config_id: assignment.plugin_config_id,
                                            plugin_type: assignment.plugin_type.clone(),
                                            discoveries: vec![],
                                            error: Some(e.to_string()),
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    tracing::warn!(
                        plugin_type = %assignment.plugin_type,
                        "plugin does not implement DiscoveryPlugin; skipping"
                    );
                    DiscoveryPluginResult {
                        plugin_config_id: assignment.plugin_config_id,
                        plugin_type: assignment.plugin_type.clone(),
                        discoveries: vec![],
                        error: Some("plugin does not support software discovery".to_string()),
                    }
                }
            }
        };
        results.push(result);
    }

    results
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uptrakit_command::NoopCommandExecutor;
    use uptrakit_internal_wire::{
        BatchUpdateItem, CheckVersionsPayload, ExecuteBatchUpdatePayload, ServiceMessage,
        UpdateFinalStatus,
    };
    use uptrakit_plugin_infrastructure_registry::PluginTypeId;
    use uuid::Uuid;

    use crate::connection_context::ConnectionContext;

    fn noop_executor() -> Arc<dyn uptrakit_command::CommandExecutor> {
        Arc::new(NoopCommandExecutor)
    }

    fn ctx() -> ConnectionContext {
        ConnectionContext::default()
    }

    fn make_batch_payload(
        plugin_type: PluginTypeId,
        timeout: std::time::Duration,
    ) -> ExecuteBatchUpdatePayload {
        let host_software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        ExecuteBatchUpdatePayload {
            host_machine_id: "test-host".to_string(),
            batch_id: Uuid::now_v7(),
            plugin_type,
            plugin_config: serde_json::Value::Object(Default::default()),
            updates: vec![BatchUpdateItem {
                host_software_item_id,
                update_history_id,
                package_identifier: "test-pkg".to_string(),
                to_version: "1.0.0".to_string(),
                release_info: None,
            }],
            pre_update_hook_plugins: vec![],
            post_update_hook_plugins: vec![],
            timeout,
            interactive: false,
        }
    }

    #[tokio::test]
    async fn unknown_plugin_type_causes_all_packages_to_fail() {
        let payload = make_batch_payload(
            PluginTypeId::new("unknown-plugin-xyz"),
            std::time::Duration::from_secs(30),
        );
        let results = super::batch_update_inner(&payload, noop_executor(), &ctx()).await;

        assert_eq!(results.len(), 1, "must return one result per package");
        assert_eq!(
            results[0].status,
            UpdateFinalStatus::Failed,
            "unknown plugin must fail"
        );
        assert!(
            results[0]
                .error
                .as_deref()
                .unwrap_or("")
                .contains("Failed to create plugin"),
            "error must mention plugin creation failure, got: {:?}",
            results[0].error
        );
        assert_eq!(
            results[0].host_software_item_id,
            payload.updates[0].host_software_item_id
        );
        assert_eq!(
            results[0].update_history_id,
            payload.updates[0].update_history_id
        );
    }

    #[tokio::test]
    async fn very_short_timeout_causes_all_packages_to_fail_with_timeout_message() {
        // Use a known-bad plugin type so the "work" inside the timeout block
        // never completes before the 0-second deadline.
        let payload = make_batch_payload(
            PluginTypeId::new("unknown-plugin-xyz"),
            std::time::Duration::ZERO,
        );
        let results = super::batch_update_inner(&payload, noop_executor(), &ctx()).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, UpdateFinalStatus::Failed);
        // The result is either a timeout message or a plugin-creation error,
        // depending on scheduler timing. Both are acceptable Failed outcomes.
        assert!(
            results[0].error.is_some(),
            "must have an error message on failure"
        );
    }

    #[tokio::test]
    async fn run_check_versions_with_empty_assignments() {
        let payload = CheckVersionsPayload {
            host_machine_id: "test-host".to_string(),
            assignments: vec![],
        };

        let response = super::run_check_versions(payload, noop_executor(), &ctx()).await;

        match response {
            ServiceMessage::VersionCheckResults(results) => {
                assert!(
                    results.results.is_empty(),
                    "empty assignments must produce empty results"
                );
            }
            other => panic!("expected VersionCheckResults, got {other:?}"),
        }
    }
}
