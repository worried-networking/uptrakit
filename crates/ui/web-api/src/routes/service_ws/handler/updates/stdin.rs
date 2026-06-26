//! `StdinAttention` message handling (broadcast + notify + audit).

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::{host, update_history};

use super::audit::emit_stdin_attention_audit;
use super::{ProcessorResponse, validate_host_link_visibility};
use crate::AppState;

/// Handle a `StdinAttention` message from the agent.
///
/// Broadcasts a stdin attention event to all SSE subscribers of the update.
#[tracing::instrument(skip_all, fields(%service_id, update_history_id = %payload.update_history_id))]
pub(in super::super) async fn handle_stdin_attention(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &uptrakit_wire::StdinAttentionPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    // Validate that this service owns the update
    let record = match validate_host_link_visibility(
        state.db(),
        service_id,
        payload.update_history_id,
        &linked_host_ids,
    )
    .await
    {
        Ok(record) => record,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "StdinAttention ownership validation failed"
            );
            return ProcessorResponse::cont();
        }
    };

    let updated = match crate::queries::update_batches::touch_stdin_attention_if_owned(
        state.db(),
        payload.update_history_id,
        service_id,
        runtime_instance_id,
        payload.hint.clone(),
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, "StdinAttention ownership validation failed");
            emit_stdin_attention_audit(
                state,
                service_id,
                &record,
                payload.hint.as_deref(),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("touch_failed"),
            )
            .await;
            return ProcessorResponse::cont();
        }
    };

    if updated == 0 {
        tracing::debug!(
            update_history_id = %payload.update_history_id,
            "ignoring stale StdinAttention from non-owner"
        );
        emit_stdin_attention_audit(
            state,
            service_id,
            &record,
            payload.hint.as_deref(),
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("not_owned"),
        )
        .await;
        return ProcessorResponse::cont();
    }

    state
        .broadcast
        .update_output_broadcaster
        .send_stdin_attention(payload.update_history_id, payload.hint.clone())
        .await;

    // Fire notification so admins can be alerted that input is needed.
    if let Ok(Some(latest_record)) = update_history::Entity::find_by_id(payload.update_history_id)
        .one(state.db())
        .await
    {
        let host_name = host::Entity::find_by_id(latest_record.host_id)
            .one(state.db())
            .await
            .ok()
            .flatten()
            .map(|h| h.friendly_name);

        let sw_name = uptrakit_shared_db::entity::software_item::Entity::find_by_id(
            latest_record.software_item_id,
        )
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|s| s.name);

        {
            let mut event = crate::notifications::events::NotificationEvent::new(
                latest_record.tenant_id,
                crate::notifications::events::NotificationEventDetails::StdinAttention {
                    update_history_id: payload.update_history_id,
                    hint: payload.hint.clone(),
                },
            );
            event.host_id = Some(latest_record.host_id);
            event.host_name = host_name;
            event.software_item_id = Some(latest_record.software_item_id);
            event.software_item_name = sw_name;
            state.notification.notification_dispatcher.dispatch(event);
        }
    }

    emit_stdin_attention_audit(
        state,
        service_id,
        &record,
        payload.hint.as_deref(),
        uptrakit_audit_log::AuditOutcome::Success,
        None,
    )
    .await;

    tracing::debug!(
        hint = ?payload.hint,
        "broadcast StdinAttention for update"
    );
    ProcessorResponse::cont()
}
