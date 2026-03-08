use std::sync::Arc;

use uptrakit_agent_core::ConnectionContext;
use uptrakit_command::{
    CommandExecutor, LocalCommandExecutor, SudoAwareCommandExecutor, SudoContext,
};
use uptrakit_service_sdk::{ControllerConnection, LoopOutcome};

// Re-export shared types so main.rs can reference them from one place.
pub(crate) use uptrakit_agent_core::{InFlightUpdate, UpdateEvent};

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

/// Handle a `CheckVersions` message from the controller.
///
/// Delegates to the shared agent-core implementation with the provided
/// executor. Passes a default `ConnectionContext` (no overrides needed
/// for the local agent).
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_check_versions(
    payload: uptrakit_internal_wire::CheckVersionsPayload,
    executor: Arc<dyn CommandExecutor>,
    conn: &mut ControllerConnection,
) -> Option<LoopOutcome> {
    uptrakit_agent_core::handle_check_versions(
        payload,
        executor,
        conn,
        &ConnectionContext::default(),
    )
    .await
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
    conn: &mut ControllerConnection,
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

/// Handle a `DiscoverSoftware` message from the controller.
///
/// Delegates to the shared agent-core implementation with the provided
/// executor. Passes a default `ConnectionContext` (no overrides needed
/// for the local agent).
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_discover_software(
    payload: uptrakit_internal_wire::DiscoverSoftwarePayload,
    executor: Arc<dyn CommandExecutor>,
    conn: &mut ControllerConnection,
) -> Option<LoopOutcome> {
    uptrakit_agent_core::handle_discover_software(
        payload,
        executor,
        conn,
        &ConnectionContext::default(),
    )
    .await
}

/// Handle an `ExecuteBatchHostPackageUpdate` message from the controller.
///
/// Delegates to the shared agent-core implementation with the provided
/// executor.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_execute_batch_host_package_update(
    payload: uptrakit_internal_wire::ExecuteBatchHostPackageUpdatePayload,
    executor: Arc<dyn CommandExecutor>,
    conn: &mut ControllerConnection,
) -> Option<LoopOutcome> {
    uptrakit_agent_core::handle_execute_batch_host_package_update(
        payload,
        executor,
        conn,
        &ConnectionContext::default(),
    )
    .await
}

pub(crate) use uptrakit_agent_core::{
    handle_graceful_shutdown, send_update_output, send_update_result,
};
