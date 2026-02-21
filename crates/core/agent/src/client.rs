use uptrakit_internal_wire::{
    DisconnectReason, DisconnectingPayload, ServiceMessage, UpdateOutputPayload,
    UpdateResultPayload, UpdateStartedPayload, VersionCheckResult, VersionCheckResultsPayload,
};
use uptrakit_service_sdk::{ControllerConnection, LoopOutcome};

/// State for an in-flight update execution.
pub(crate) struct InFlightUpdate {
    pub update_history_id: uuid::Uuid,
    pub handle: tokio::task::JoinHandle<crate::update::UpdateExecutionResult>,
    pub output_rx: tokio::sync::mpsc::Receiver<crate::update::UpdateOutputMessage>,
}

/// Events from an in-flight update.
pub(crate) enum UpdateEvent {
    Output(crate::update::UpdateOutputMessage),
    Completed(std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>),
}

/// Send an update output message to the controller.
pub(crate) async fn send_update_output(
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
pub(crate) async fn send_update_result(
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
pub(crate) async fn handle_graceful_shutdown(
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

/// Handle a CheckVersions message from the controller.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_check_versions(
    payload: uptrakit_internal_wire::CheckVersionsPayload,
    conn: &mut ControllerConnection,
) -> Option<LoopOutcome> {
    tracing::info!(
        count = payload.assignments.len(),
        "received CheckVersions request"
    );

    let executor: std::sync::Arc<dyn uptrakit_command::CommandExecutor> =
        std::sync::Arc::new(uptrakit_command::LocalCommandExecutor);

    // Pre-refresh package indexes for provider types that support it.
    // Deduplicate by provider type so we only refresh once per type.
    {
        let mut refreshed_types = std::collections::HashSet::new();
        for assignment in &payload.assignments {
            if refreshed_types.contains(&assignment.provider_type) {
                continue;
            }
            if let Ok(provider) = uptrakit_provider_registry::ProviderRegistry::create_provider(
                assignment.provider_type.clone(),
                &assignment.config,
                std::sync::Arc::clone(&executor),
            ) && provider
                .has_capability(uptrakit_provider_registry::ProviderCapability::RefreshPackageIndex)
            {
                tracing::info!(provider_type = %assignment.provider_type, "refreshing package index");
                if let Err(e) = provider.refresh_package_index().await {
                    tracing::warn!(provider_type = %assignment.provider_type, error = %e, "failed to refresh package index");
                }
                refreshed_types.insert(assignment.provider_type.clone());
            }
        }
    }

    use futures_util::stream::{self, StreamExt};
    let results: Vec<VersionCheckResult> = stream::iter(payload.assignments)
        .map(|assignment| {
            let executor = std::sync::Arc::clone(&executor);
            async move {
                tracing::debug!(
                    software_item_id = %assignment.software_item_id,
                    name = %assignment.name,
                    provider_type = %assignment.provider_type,
                    "checking version"
                );
                let outcome = crate::version_check::check_version(
                    assignment.provider_type,
                    &assignment.config,
                    &assignment.package_identifier,
                    executor,
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

/// Handle an ExecuteUpdate message from the controller.
pub(crate) async fn handle_execute_update(
    payload: uptrakit_internal_wire::ExecuteUpdatePayload,
    in_flight_update: &mut Option<InFlightUpdate>,
    conn: &mut ControllerConnection,
) {
    tracing::info!(
        update_id = %payload.update_history_id,
        software = %payload.software_item_name,
        version = %payload.to_version,
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

    // Create a channel for output streaming
    let (output_tx, output_rx) =
        tokio::sync::mpsc::channel::<crate::update::UpdateOutputMessage>(100);

    let update_history_id = payload.update_history_id;

    // Spawn update execution task
    let handle =
        tokio::spawn(async move { crate::update::execute_update(payload, output_tx).await });

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
