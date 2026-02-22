use std::sync::Arc;

use uptrakit_command::LocalCommandExecutor;
use uptrakit_service_sdk::{ControllerConnection, LoopOutcome};

// Re-export shared types so main.rs can reference them from one place.
pub(crate) use uptrakit_agent_core::{InFlightUpdate, UpdateEvent};

/// Handle a `CheckVersions` message from the controller.
///
/// Creates a `LocalCommandExecutor` and delegates to the shared agent-core
/// implementation.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_check_versions(
    payload: uptrakit_internal_wire::CheckVersionsPayload,
    conn: &mut ControllerConnection,
) -> Option<LoopOutcome> {
    let executor = Arc::new(LocalCommandExecutor);
    uptrakit_agent_core::handle_check_versions(payload, executor, conn).await
}

/// Handle an `ExecuteUpdate` message from the controller.
///
/// Creates a `LocalCommandExecutor` and delegates to the shared agent-core
/// implementation.
pub(crate) async fn handle_execute_update(
    payload: uptrakit_internal_wire::ExecuteUpdatePayload,
    in_flight_update: &mut Option<InFlightUpdate>,
    conn: &mut ControllerConnection,
) {
    let executor = Arc::new(LocalCommandExecutor);
    uptrakit_agent_core::handle_execute_update(payload, executor, in_flight_update, conn).await;
}

pub(crate) use uptrakit_agent_core::{
    handle_graceful_shutdown, send_update_output, send_update_result,
};
