//! `UpdateStarted` message handling and the started-event broadcast.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::service;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::{AuditEventPayload, UpdateStartedPayload};

use super::super::audit_service;
use super::lookups::{resolve_host_name, resolve_software_item_name};
use super::{ProcessorResponse, emit_batch_progress_event, validate_host_link_visibility};
use crate::AppState;

/// Metadata extracted from an `update_history` record when marking it
/// in-progress. Used by the subsequent broadcast phase.
pub(super) struct UpdateStartedInfo {
    pub(super) batch_id: Option<uuid::Uuid>,
    pub(super) host_id: uuid::Uuid,
    pub(super) software_item_id: uuid::Uuid,
    pub(super) tenant_id: uuid::Uuid,
}

/// Push MQTT state updates, broadcast `AdminEvent::UpdateStarted`, and emit
/// optional batch progress — all fire-and-forget side-effects.
pub(super) async fn broadcast_update_started_events(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateStartedPayload,
    info: &UpdateStartedInfo,
) {
    // Push updated software states to MQTT services so that the
    // in_progress flag transitions to true immediately.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    // Broadcast AdminEvent::UpdateStarted so the history-list SSE subscribers
    // can update the "Input Required" badge in real-time without reloading.
    state
        .notification
        .event_broadcaster
        .send(
            info.tenant_id,
            AdminEvent::UpdateStarted {
                update_history_id: payload.update_history_id,
                host_id: info.host_id,
                software_item_id: info.software_item_id,
                interactive: payload.interactive,
            },
        )
        .await;

    let software_name = resolve_software_item_name(state, info.software_item_id).await;
    let host_name = resolve_host_name(state, info.host_id).await;
    let target_display = format!("{software_name} on {host_name}");
    let update_payload = AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED.to_string(),
        tenant_id: Some(info.tenant_id.to_string()),
        target_type: Some("update_history".to_string()),
        target_id: Some(payload.update_history_id.to_string()),
        target_display: Some(target_display),
        outcome: uptrakit_audit_log::AuditOutcome::Success
            .as_str()
            .to_string(),
        details_json: Some(
            serde_json::json!({
                "batch_id": info.batch_id,
                "from_version": payload.from_version,
                "host_id": info.host_id,
                "interactive": payload.interactive,
                "software_item_id": info.software_item_id,
            })
            .to_string(),
        ),
        request_id: None,
        correlation_id: None,
    };
    let _ = audit_service::ingest_service_audit_event(
        state,
        service_id,
        false,
        Some(info.tenant_id),
        None,
        update_payload,
    )
    .await;

    // Emit batch progress event if this update is part of a batch.
    if let Some(batch_id) = info.batch_id {
        let batch_payload = AuditEventPayload {
            action_type: uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_STARTED
                .to_string(),
            tenant_id: Some(info.tenant_id.to_string()),
            target_type: Some("batch_update".to_string()),
            target_id: Some(batch_id.to_string()),
            target_display: Some(host_name.clone()),
            outcome: uptrakit_audit_log::AuditOutcome::Success
                .as_str()
                .to_string(),
            details_json: Some(
                serde_json::json!({
                    "batch_id": batch_id,
                    "host_id": info.host_id,
                    "interactive": payload.interactive,
                    "software_item_id": info.software_item_id,
                    "update_history_id": payload.update_history_id,
                })
                .to_string(),
            ),
            request_id: None,
            correlation_id: None,
        };
        let _ = audit_service::ingest_service_audit_event(
            state,
            service_id,
            false,
            Some(info.tenant_id),
            None,
            batch_payload,
        )
        .await;

        emit_batch_progress_event(
            state,
            batch_id,
            crate::batch_progress_broadcaster::BatchProgressEvent::UpdateStarted {
                update_history_id: payload.update_history_id,
                software_item_name: software_name,
                host_name,
            },
        )
        .await;
    }
}

/// Handle an `UpdateStarted` message: validate ownership, set status to
/// `InProgress`, clear previous output lines.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id))]
pub(in super::super) async fn handle_update_started(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateStartedPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::info!(
        update_id = %payload.update_history_id,
        from_version = ?payload.from_version,
        "update started"
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

    let claim = match crate::queries::update_batches::claim_or_replay_update_start_db(
        state.db(),
        payload.update_history_id,
        service_id,
        runtime_instance_id,
        payload.interactive,
    )
    .await
    {
        Ok(claim) => claim,
        Err(error) => {
            tracing::warn!(
                error = %error,
                update_history_id = %payload.update_history_id,
                "failed to claim or replay update start"
            );
            return ProcessorResponse::cont();
        }
    };

    match claim {
        crate::queries::update_batches::ClaimExecutionOutcome::Claimed(info) => {
            let info = UpdateStartedInfo {
                batch_id: info.batch_id,
                host_id: info.host_id,
                software_item_id: info.software_item_id,
                tenant_id: info.tenant_id,
            };
            state
                .broadcast
                .update_output_broadcaster
                .get_or_create_channel(payload.update_history_id)
                .await;
            state
                .broadcast
                .update_output_broadcaster
                .send_agent_claimed(payload.update_history_id, service_id)
                .await;
            broadcast_update_started_events(state, service_id, payload, &info).await;
        }
        crate::queries::update_batches::ClaimExecutionOutcome::Replay(_) => {
            state
                .broadcast
                .update_output_broadcaster
                .get_or_create_channel(payload.update_history_id)
                .await;
            state
                .broadcast
                .update_output_broadcaster
                .send_agent_claimed(payload.update_history_id, service_id)
                .await;
        }
        crate::queries::update_batches::ClaimExecutionOutcome::Rejected => {
            return ProcessorResponse::cont();
        }
    }

    ProcessorResponse::cont()
}
