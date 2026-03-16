//! Update-tracking message handlers.
//!
//! Handles `ServiceTriggerUpdate` and `ServiceTriggerHostBatchUpdate` messages
//! from services with the `UpdateTracking` capability.

use std::sync::Arc;

use uptrakit_internal_wire::{
    ControllerMessage, ErrorCode, ErrorPayload, ServiceHostBatchUpdateTriggerPayload,
    ServiceUpdateTriggerPayload,
};
use uptrakit_web_api_types::events::AdminEvent;

use super::shared_types::ProcessorResponse;
use crate::AppState;
use crate::queries::update_types::ActorType;

/// Handle a `ServiceTriggerUpdate` message.
#[tracing::instrument(skip_all)]
pub(super) async fn handle_service_trigger_update(
    state: &Arc<AppState>,
    payload: &ServiceUpdateTriggerPayload,
) -> ProcessorResponse {
    match crate::queries::update_triggers::trigger_update_for_host(
        state.db(),
        &state.notification_service,
        crate::queries::update_triggers::TriggerUpdateParams {
            tenant_id: payload.tenant_id,
            item_id: payload.software_item_id,
            host_id: payload.host_id,
            to_version: payload.to_version.clone(),
            actor_type: ActorType::Mqtt,
            actor_id: &payload.mqtt_client_id.to_string(),
            release_info: None,
            interactive: false,
        },
    )
    .await
    {
        Ok(result) => {
            tracing::info!(
                update_id = %result.update_history_id,
                software_item_id = %payload.software_item_id,
                host_id = %payload.host_id,
                "service-triggered update dispatched"
            );
            state
                .notification_service
                .push_software_states_for_tenant(state.db(), payload.tenant_id)
                .await;
            state
                .broadcast
                .event_broadcaster
                .send(
                    payload.tenant_id,
                    AdminEvent::UpdateTriggered {
                        update_history_id: result.update_history_id,
                        host_id: payload.host_id,
                        software_item_id: payload.software_item_id,
                    },
                )
                .await;
            ProcessorResponse::cont()
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                software_item_id = %payload.software_item_id,
                host_id = %payload.host_id,
                "service-triggered update failed"
            );
            ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: err.to_string(),
            }))
        }
    }
}

/// Handle a `ServiceTriggerHostBatchUpdate` message.
#[tracing::instrument(skip_all)]
pub(super) async fn handle_service_trigger_host_batch_update(
    state: &Arc<AppState>,
    payload: &ServiceHostBatchUpdateTriggerPayload,
) -> ProcessorResponse {
    let category_filter = if payload.security_only {
        Some("security")
    } else {
        None
    };
    let outdated = match crate::queries::update_batches::find_outdated_items_for_host(
        state.db(),
        payload.tenant_id,
        payload.host_id,
        category_filter,
        None,
    )
    .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::warn!(
                error = %err,
                host_id = %payload.host_id,
                "service-triggered host batch update: failed to find outdated items"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: err.to_string(),
            }));
        }
    };

    if outdated.is_empty() {
        return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::BadRequest,
            message: "no outdated items found on this host".to_string(),
        }));
    }

    let params = crate::queries::update_batches::CreateBatchParams {
        tenant_id: payload.tenant_id,
        batch_type: crate::queries::update_types::BatchType::HostUpdate,
        actor_type: ActorType::Mqtt,
        actor_id: &payload.mqtt_client_id.to_string(),
    };
    match crate::queries::update_batches::create_batch(
        state.db(),
        &state.notification_service,
        &params,
        outdated,
    )
    .await
    {
        Ok(resp) => {
            if let Some(batch_id) = resp.batch_id {
                tracing::info!(
                    %batch_id,
                    host_id = %payload.host_id,
                    "service-triggered host batch update dispatched"
                );
                state
                    .notification_service
                    .push_software_states_for_tenant(state.db(), payload.tenant_id)
                    .await;
                ProcessorResponse::cont()
            } else {
                ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::BadRequest,
                    message: "no eligible items for batch update".to_string(),
                }))
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                host_id = %payload.host_id,
                "service-triggered host batch update failed"
            );
            ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: err.to_string(),
            }))
        }
    }
}
