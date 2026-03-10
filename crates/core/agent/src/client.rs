use std::sync::Arc;

use uptrakit_agent_core::ConnectionContext;
use uptrakit_command::{
    CommandExecutor, LocalCommandExecutor, SudoAwareCommandExecutor, SudoContext,
};
use uptrakit_internal_wire::ServiceMessage;

// Re-export shared types so main.rs can reference them from one place.
pub(crate) use uptrakit_agent_core::{InFlightUpdate, UpdateEvent};

/// Events produced by the local agent's service loop.
///
/// Extends [`UpdateEvent`] with a [`BackgroundResult`](AgentEvent::BackgroundResult)
/// variant for results from background tasks (version checks, discovery, batch
/// updates) that must not block the event loop.
pub(crate) enum AgentEvent {
    /// Progress from an in-flight update task (output line or completion).
    Update(UpdateEvent),
    /// The in-flight update process appears to be waiting for stdin input.
    Attention(uuid::Uuid),
    /// A background operation completed and produced a [`ServiceMessage`] that
    /// should be forwarded to the controller.
    BackgroundResult(ServiceMessage),
}

/// Extract the attention channel from an `InFlightUpdate`, if available.
///
/// Returns `None` when the `interactive` feature is not enabled.
fn take_attention_rx(
    #[allow(unused_variables)] update: &mut InFlightUpdate,
) -> Option<tokio::sync::mpsc::Receiver<()>> {
    #[cfg(feature = "interactive")]
    {
        return update.attention_rx.take();
    }
    #[allow(unreachable_code)]
    None
}

/// Restore the attention channel into the `InFlightUpdate`.
fn restore_attention_rx(
    #[allow(unused_variables)] update: &mut InFlightUpdate,
    #[allow(unused_variables)] rx: Option<tokio::sync::mpsc::Receiver<()>>,
) {
    #[cfg(feature = "interactive")]
    {
        update.attention_rx = rx;
    }
}

/// Poll the in-flight update for events (output, completion, and — when the
/// `interactive` feature is enabled — stdin attention).
///
/// Pends forever when no update is in flight.
pub(crate) async fn poll_in_flight_update(
    in_flight_update: &mut Option<InFlightUpdate>,
) -> AgentEvent {
    let Some(update) = in_flight_update else {
        return std::future::pending().await;
    };
    // Extract the attention channel into a local so we can pass field-level
    // borrows to tokio::select! without conflicting with `&mut update.handle`.
    let mut attention_rx = take_attention_rx(update);
    let update_history_id = update.update_history_id;
    let event = tokio::select! {
        biased;
        Some(output_msg) = update.output_rx.recv() => {
            AgentEvent::Update(UpdateEvent::Output(output_msg))
        }
        result = &mut update.handle => {
            AgentEvent::Update(UpdateEvent::Completed(result))
        }
        Some(()) = recv_attention_rx(&mut attention_rx) => {
            AgentEvent::Attention(update_history_id)
        }
    };
    // Put the channel back so attention detection continues across polls.
    restore_attention_rx(update, attention_rx);
    event
}

/// Build the executor for the local agent.
///
/// Wraps [`LocalCommandExecutor`] with [`SudoAwareCommandExecutor`] using the
/// default [`SudoContext`] (non-root user, sudo available, auto policy).
/// This matches the pre-`privileged`-flag behaviour where plugins hard-coded
/// `sudo` directly.
pub(crate) fn make_executor() -> Arc<dyn CommandExecutor> {
    let raw: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
    Arc::new(SudoAwareCommandExecutor::new(raw, SudoContext::default()))
}

/// Spawn a `CheckVersions` operation as a background task.
///
/// The work runs on a separate tokio task so the event loop remains responsive
/// for pings, signals, and other controller messages. The result is sent
/// through `bg_tx` for forwarding to the controller.
pub(crate) fn spawn_check_versions(
    payload: uptrakit_internal_wire::CheckVersionsPayload,
    executor: Arc<dyn CommandExecutor>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        assignment_count = payload.assignments.len(),
        "spawning background CheckVersions task"
    );
    uptrakit_agent_core::spawn_background(bg_tx, async move {
        let msg = uptrakit_agent_core::run_check_versions(
            payload,
            executor,
            &ConnectionContext::default(),
        )
        .await;
        tracing::debug!(host_machine_id = %host_machine_id, "background CheckVersions task completed");
        msg
    });
}

/// Handle an `ExecuteUpdate` message from the controller.
///
/// Delegates to the shared agent-core implementation with the provided
/// executor. Passes a default `ConnectionContext` (no overrides needed
/// for the local agent).
pub(crate) async fn handle_execute_update(
    payload: uptrakit_internal_wire::ExecuteUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    in_flight_update: &mut Option<InFlightUpdate>,
    conn: &mut uptrakit_service_sdk::ControllerConnection,
) {
    uptrakit_agent_core::handle_execute_update(
        payload,
        executor,
        in_flight_update,
        conn,
        &ConnectionContext::default(),
    )
    .await;
}

/// Spawn a `DiscoverSoftware` operation as a background task.
///
/// The work runs on a separate tokio task so the event loop remains responsive
/// for pings, signals, and other controller messages. The result is sent
/// through `bg_tx` for forwarding to the controller.
pub(crate) fn spawn_discover_software(
    payload: uptrakit_internal_wire::DiscoverSoftwarePayload,
    executor: Arc<dyn CommandExecutor>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        plugin_count = payload.plugins.len(),
        "spawning background DiscoverSoftware task"
    );
    uptrakit_agent_core::spawn_background(bg_tx, async move {
        let msg = uptrakit_agent_core::run_discover_software(
            payload,
            executor,
            &ConnectionContext::default(),
        )
        .await;
        tracing::debug!(host_machine_id = %host_machine_id, "background DiscoverSoftware task completed");
        msg
    });
}

/// Spawn an `ExecuteBatchUpdate` operation as a background task.
///
/// The work runs on a separate tokio task so the event loop remains responsive
/// for pings, signals, and other controller messages. The result is sent
/// through `bg_tx` for forwarding to the controller.
pub(crate) fn spawn_execute_batch_update(
    payload: uptrakit_internal_wire::ExecuteBatchUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let batch_id = payload.batch_id;
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        batch_id = %batch_id,
        update_count = payload.updates.len(),
        "spawning background ExecuteBatchUpdate task"
    );
    uptrakit_agent_core::spawn_background(bg_tx, async move {
        let msg = uptrakit_agent_core::run_execute_batch_update(
            payload,
            executor,
            &ConnectionContext::default(),
        )
        .await;
        tracing::debug!(
            host_machine_id = %host_machine_id,
            batch_id = %batch_id,
            "background ExecuteBatchUpdate task completed"
        );
        msg
    });
}

pub(crate) use uptrakit_agent_core::{
    handle_graceful_shutdown, send_update_output, send_update_result,
};

/// Forward stdin data or a signal from the controller to the in-flight update.
#[cfg(feature = "interactive")]
pub(crate) fn handle_update_stdin_data(
    payload: uptrakit_internal_wire::UpdateStdinDataPayload,
    in_flight_update: &Option<InFlightUpdate>,
) {
    let Some(update) = in_flight_update else {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "received UpdateStdinData but no in-flight update exists; ignoring"
        );
        return;
    };
    if update.update_history_id != payload.update_history_id {
        tracing::debug!(
            expected = %update.update_history_id,
            received = %payload.update_history_id,
            "UpdateStdinData update_history_id mismatch; ignoring"
        );
        return;
    }

    if let Some(signal) = payload.signal {
        if let Some(ref signal_tx) = update.signal_tx {
            if signal_tx.try_send(signal).is_err() {
                tracing::warn!("signal channel full or closed; dropping signal {signal}");
            }
        } else {
            tracing::debug!("signal_tx not available for this update; ignoring signal");
        }
    } else if let Some(ref stdin_tx) = update.stdin_tx {
        use base64::Engine as _;
        match base64::engine::general_purpose::STANDARD.decode(&payload.data) {
            Ok(bytes) => {
                if stdin_tx.try_send(bytes).is_err() {
                    tracing::warn!("stdin channel full or closed; dropping stdin data");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to decode base64 stdin data");
            }
        }
    } else {
        tracing::debug!("stdin_tx not available for this update; ignoring stdin data");
    }
}

/// Receive attention notification from the optional attention channel.
///
/// Returns `Some(())` when the update process appears to be waiting for stdin
/// input (heuristic: no output for ~10 seconds). Pends forever when the
/// channel is `None`.
async fn recv_attention_rx(
    attention_rx: &mut Option<tokio::sync::mpsc::Receiver<()>>,
) -> Option<()> {
    if let Some(rx) = attention_rx {
        return rx.recv().await;
    }
    std::future::pending().await
}
