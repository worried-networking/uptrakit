//! `UpdateOutput` message handling (owner-safe persist + broadcast).

use std::collections::HashSet;
use std::sync::Arc;

use uptrakit_wire::UpdateOutputPayload;

use super::{ProcessorResponse, validate_host_link_visibility};
use crate::AppState;

/// Handle an `UpdateOutput` message: validate ownership and persist output in
/// one owner-safe step before broadcasting it.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id))]
pub(in super::super) async fn handle_update_output(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateOutputPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::trace!(
        update_id = %payload.update_history_id,
        stream = ?payload.stream,
        "update output"
    );
    if validate_host_link_visibility(
        state.db(),
        service_id,
        payload.update_history_id,
        &linked_host_ids,
    )
    .await
    .is_err()
    {
        return ProcessorResponse::cont();
    }

    let outcome = match crate::queries::update_batches::append_update_output_if_owned(
        state.db(),
        payload.update_history_id,
        service_id,
        runtime_instance_id,
        payload.stream,
        &payload.output,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(
                error = %error,
                update_id = %payload.update_history_id,
                "failed to persist update output"
            );
            return ProcessorResponse::cont();
        }
    };

    let persisted_lines = outcome.into_persisted_lines();
    if persisted_lines.is_empty() {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "ignoring stale UpdateOutput"
        );
        return ProcessorResponse::cont();
    }

    for line in persisted_lines {
        state
            .broadcast
            .update_output_broadcaster
            .send_line(
                payload.update_history_id,
                line.id,
                line.output,
                line.stream,
                line.created_at,
            )
            .await;
    }

    ProcessorResponse::cont()
}
