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
    /// A background operation completed and produced a [`ServiceMessage`] that
    /// should be forwarded to the controller.
    BackgroundResult(ServiceMessage),
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

/// Spawn an `ExecuteBatchHostPackageUpdate` operation as a background task.
///
/// The work runs on a separate tokio task so the event loop remains responsive
/// for pings, signals, and other controller messages. The result is sent
/// through `bg_tx` for forwarding to the controller.
pub(crate) fn spawn_execute_batch_host_package_update(
    payload: uptrakit_internal_wire::ExecuteBatchHostPackageUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let batch_id = payload.batch_id;
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        batch_id = %batch_id,
        update_count = payload.updates.len(),
        "spawning background ExecuteBatchHostPackageUpdate task"
    );
    uptrakit_agent_core::spawn_background(bg_tx, async move {
        let msg = uptrakit_agent_core::run_execute_batch_host_package_update(
            payload,
            executor,
            &ConnectionContext::default(),
        )
        .await;
        tracing::debug!(
            host_machine_id = %host_machine_id,
            batch_id = %batch_id,
            "background ExecuteBatchHostPackageUpdate task completed"
        );
        msg
    });
}

pub(crate) use uptrakit_agent_core::{
    handle_graceful_shutdown, send_update_output, send_update_result,
};
