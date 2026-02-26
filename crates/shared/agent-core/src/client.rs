use std::sync::Arc;

use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{
    DisconnectReason, DisconnectingPayload, DiscoverSoftwarePayload, DiscoveryPluginResult,
    DiscoveryResultsPayload, ServiceMessage, UpdateOutputPayload, UpdateResultPayload,
    UpdateStartedPayload, VersionCheckResult, VersionCheckResultsPayload,
};
use uptrakit_service_sdk::{ControllerConnection, LoopOutcome};

use crate::connection_context::ConnectionContext;

/// State for an in-flight update execution.
pub struct InFlightUpdate {
    pub update_history_id: uuid::Uuid,
    pub handle: tokio::task::JoinHandle<crate::update::UpdateExecutionResult>,
    pub output_rx: tokio::sync::mpsc::Receiver<crate::update::UpdateOutputMessage>,
}

/// Events from an in-flight update.
pub enum UpdateEvent {
    Output(crate::update::UpdateOutputMessage),
    Completed(std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>),
}

/// Send an update output message to the controller.
pub async fn send_update_output(
    conn: &mut ControllerConnection,
    update_history_id: uuid::Uuid,
    output_msg: crate::update::UpdateOutputMessage,
) {
    conn.send_best_effort(ServiceMessage::UpdateOutput(UpdateOutputPayload {
        update_history_id,
        output: output_msg.output,
        stream: output_msg.stream,
    }))
    .await;
}

/// Send the final update result to the controller.
pub async fn send_update_result(
    conn: &mut ControllerConnection,
    update_history_id: uuid::Uuid,
    result: std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>,
) {
    match result {
        Ok(exec_result) => {
            conn.send_best_effort(ServiceMessage::UpdateResult(exec_result.result))
                .await;
            tracing::info!(update_id = %update_history_id, "update execution completed");
        }
        Err(e) => {
            tracing::error!(error = %e, "update task panicked");
            conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id,
                status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some(format!("Update task panicked: {e}")),
            }))
            .await;
        }
    }
}

/// Handle graceful shutdown: drain in-flight update, send Disconnecting.
pub async fn handle_graceful_shutdown(
    conn: &mut ControllerConnection,
    in_flight_update: Option<InFlightUpdate>,
    timeout_seconds: u32,
    disconnect_reason: DisconnectReason,
    outcome: LoopOutcome,
) -> LoopOutcome {
    use std::time::Duration;

    if let Some(mut update) = in_flight_update {
        tracing::info!(
            update_id = %update.update_history_id,
            timeout_seconds,
            "waiting for in-flight update to complete before shutdown"
        );

        let timeout = Duration::from_secs(u64::from(timeout_seconds));
        let deadline = tokio::time::Instant::now() + timeout;

        // Continue processing output and wait for completion
        loop {
            tokio::select! {
                biased;

                Some(output_msg) = update.output_rx.recv() => {
                    send_update_output(conn, update.update_history_id, output_msg).await;
                }
                result = &mut update.handle => {
                    send_update_result(conn, update.update_history_id, result).await;
                    break;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!(
                        update_id = %update.update_history_id,
                        "shutdown timeout reached, abandoning in-flight update"
                    );
                    conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                        update_history_id: update.update_history_id,
                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                        from_version: None,
                        to_version: None,
                        output: String::new(),
                        error: Some(format!("Agent shutdown timeout ({timeout_seconds}s) reached")),
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
    let disconnecting_msg =
        ServiceMessage::Disconnecting(DisconnectingPayload::new(disconnect_reason));
    if let Err(e) = conn.send(disconnecting_msg).await {
        tracing::debug!(error = %e, "failed to send Disconnecting message");
    } else {
        tracing::debug!(reason = ?disconnect_reason, "sent Disconnecting message to controller");
    }

    outcome
}

/// Handle a `CheckVersions` message from the controller.
///
/// The `executor` is provided by the caller — `LocalCommandExecutor` for the
/// regular agent, `SshCommandExecutor` for the SSH agent.
///
/// The `ctx` is used to inject connection-specific overrides (e.g. a remote
/// Docker host for the SSH agent) into each plugin config before creation.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub async fn handle_check_versions(
    payload: uptrakit_internal_wire::CheckVersionsPayload,
    executor: Arc<dyn CommandExecutor>,
    conn: &mut ControllerConnection,
    ctx: &ConnectionContext,
) -> Option<LoopOutcome> {
    tracing::info!(
        count = payload.assignments.len(),
        host_machine_id = %payload.host_machine_id,
        "received CheckVersions request"
    );

    // Pre-refresh package indexes for plugin types that support it.
    // Deduplicate by plugin type so we only refresh once per type.
    // Look at both detect_version and fetch_releases assignments.
    {
        let mut refreshed_types = std::collections::HashSet::new();
        for assignment in &payload.assignments {
            for plugin_assignment in assignment
                .detect_version
                .iter()
                .chain(assignment.fetch_releases.iter())
            {
                if refreshed_types.contains(&plugin_assignment.plugin_type) {
                    continue;
                }
                let mut effective_config = plugin_assignment.config.clone();
                ctx.apply_to_config(&plugin_assignment.plugin_type, &mut effective_config);

                if let Ok(plugin) = uptrakit_plugin_infrastructure_registry::PluginRegistry::create_plugin(
                    plugin_assignment.plugin_type.clone(),
                    &effective_config,
                    Arc::clone(&executor),
                ) && plugin.has_capability(
                    uptrakit_plugin_infrastructure_registry::PluginCapability::RefreshPackageIndex,
                ) {
                    tracing::info!(plugin_type = %plugin_assignment.plugin_type, "refreshing package index");
                    if let Err(e) = plugin.refresh_package_index().await {
                        tracing::warn!(plugin_type = %plugin_assignment.plugin_type, error = %e, "failed to refresh package index");
                    }
                    refreshed_types.insert(plugin_assignment.plugin_type.clone());
                }
            }
        }
    }

    use futures_util::stream::{self, StreamExt};
    let results: Vec<VersionCheckResult> = stream::iter(payload.assignments)
        .map(|assignment| {
            let executor = Arc::clone(&executor);
            let ctx = ctx.clone();
            async move {
                tracing::debug!(
                    software_item_id = %assignment.software_item_id,
                    name = %assignment.name,
                    "checking version"
                );
                let outcome = crate::version_check::check_version(
                    assignment.detect_version.as_ref(),
                    assignment.fetch_releases.as_ref(),
                    executor,
                    &ctx,
                )
                .await;
                VersionCheckResult {
                    software_item_id: assignment.software_item_id,
                    installed_version: outcome.installed_version,
                    latest_version: outcome.latest_version,
                    error: outcome.error,
                }
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;

    let response = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload { results });
    if let Err(e) = conn.send(response).await {
        tracing::error!(error = %e, "failed to send VersionCheckResults");
        return Some(LoopOutcome::Disconnected);
    }
    tracing::debug!("sent VersionCheckResults");
    None
}

/// Handle an `ExecuteUpdate` message from the controller.
///
/// The `executor` is provided by the caller — `LocalCommandExecutor` for the
/// regular agent, `SshCommandExecutor` for the SSH agent.
///
/// The `ctx` is used to inject connection-specific overrides into the plugin
/// config before instantiation.
pub async fn handle_execute_update(
    payload: uptrakit_internal_wire::ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    in_flight_update: &mut Option<InFlightUpdate>,
    conn: &mut ControllerConnection,
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
        conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
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

    // Spawn update execution task
    let handle = tokio::spawn(async move {
        crate::update::execute_update(effective_payload, executor, output_tx).await
    });

    // Send UpdateStarted
    if let Err(e) = conn
        .send(ServiceMessage::UpdateStarted(UpdateStartedPayload {
            update_history_id,
            from_version: None,
        }))
        .await
    {
        tracing::error!(error = %e, "failed to send UpdateStarted");
    }

    // Track the in-flight update
    *in_flight_update = Some(InFlightUpdate {
        update_history_id,
        handle,
        output_rx,
    });
}

/// Handle a `DiscoverSoftware` message from the controller.
///
/// Iterates over each plugin assignment, attempts discovery via the plugin
/// registry, and returns all results in a single `DiscoveryResults` message.
/// Plugin-level errors are recorded in the result rather than aborting the
/// entire discovery run.
///
/// The `ctx` is used to inject connection-specific overrides (e.g. a remote
/// Docker host for the SSH agent) into each plugin config before creation.
///
/// Returns `Some(LoopOutcome::Disconnected)` if sending the response fails.
pub async fn handle_discover_software(
    payload: DiscoverSoftwarePayload,
    executor: Arc<dyn CommandExecutor>,
    conn: &mut ControllerConnection,
    ctx: &ConnectionContext,
) -> Option<LoopOutcome> {
    tracing::info!(
        count = payload.plugins.len(),
        host_machine_id = %payload.host_machine_id,
        "received DiscoverSoftware request"
    );

    let mut results = Vec::with_capacity(payload.plugins.len());

    for assignment in payload.plugins {
        tracing::debug!(
            plugin_type = %assignment.plugin_type,
            plugin_config_id = ?assignment.plugin_config_id,
            "running discovery for plugin"
        );

        let mut effective_config = assignment.config.clone();
        ctx.apply_to_config(&assignment.plugin_type, &mut effective_config);

        let result =
            match uptrakit_plugin_infrastructure_registry::PluginRegistry::create_plugin_for_discovery(
                assignment.plugin_type.clone(),
                &effective_config,
                Arc::clone(&executor),
            ) {
                Err(e) => {
                    tracing::warn!(
                        plugin_type = %assignment.plugin_type,
                        error = %e,
                        "failed to create plugin for discovery"
                    );
                    DiscoveryPluginResult {
                        plugin_config_id: assignment.plugin_config_id,
                        plugin_type: assignment.plugin_type,
                        discoveries: vec![],
                        error: Some(e.to_string()),
                    }
                }
                Ok(plugin) => {
                    use uptrakit_plugin_infrastructure_registry::PluginCapability;
                    if !plugin.has_capability(PluginCapability::DiscoverLocalSoftware) {
                        tracing::warn!(
                            plugin_type = %assignment.plugin_type,
                            "plugin does not support DiscoverLocalSoftware; skipping"
                        );
                        DiscoveryPluginResult {
                            plugin_config_id: assignment.plugin_config_id,
                            plugin_type: assignment.plugin_type,
                            discoveries: vec![],
                            error: Some("plugin does not support software discovery".to_string()),
                        }
                    } else {
                        match plugin.discover_software().await {
                            Ok(discoveries) => {
                                tracing::info!(
                                    plugin_type = %assignment.plugin_type,
                                    count = discoveries.len(),
                                    "discovery completed"
                                );
                                DiscoveryPluginResult {
                                    plugin_config_id: assignment.plugin_config_id,
                                    plugin_type: assignment.plugin_type,
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
                                    plugin_type: assignment.plugin_type,
                                    discoveries: vec![],
                                    error: Some(e.to_string()),
                                }
                            }
                        }
                    }
                }
            };
        results.push(result);
    }

    let response = ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
        host_machine_id: payload.host_machine_id,
        results,
    });
    if let Err(e) = conn.send(response).await {
        tracing::error!(error = %e, "failed to send DiscoveryResults");
        return Some(LoopOutcome::Disconnected);
    }
    tracing::debug!("sent DiscoveryResults");
    None
}
