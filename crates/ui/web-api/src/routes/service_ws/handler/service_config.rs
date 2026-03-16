//! Controller-side handlers for the service config store wire protocol.
//!
//! Provides:
//! - [`deliver_service_config`]: called during session setup to send all stored
//!   entries for this `service_app_name` to the connecting service.
//! - [`handle_store_service_config`]: upsert handler for `StoreServiceConfig`.
//! - [`handle_delete_service_config`]: delete handler for `DeleteServiceConfig`.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;

use uptrakit_internal_wire::{
    ControllerMessage, DeleteServiceConfigPayload, OutgoingSeq, ServiceConfigAckPayload,
    ServiceConfigDeliveryPayload, ServiceConfigEntry, ServiceConfigKey,
    ServiceConfigUpdatedPayload, StoreServiceConfigPayload,
};

use super::shared_types::ProcessorResponse;
use crate::AppState;
use crate::routes::service_ws::protocol::serialize_controller_msg;

/// Deliver all stored config entries for `service_app_name` to the connecting service.
///
/// Called during session setup after credential delivery.
/// Returns `Some(())` on success or `None` if the WebSocket write failed.
pub(super) async fn deliver_service_config(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    service_app_name: &str,
    out_seq: &mut OutgoingSeq,
) -> Option<()> {
    let rows = match crate::queries::service_config::load_for_service(state.db(), service_app_name)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                "failed to load service config entries; skipping delivery"
            );
            return Some(()); // non-fatal: continue session setup
        }
    };

    if rows.is_empty() {
        return Some(());
    }

    let entries: Vec<ServiceConfigEntry> = rows
        .into_iter()
        .map(|r| ServiceConfigEntry::new(r.tenant_id, r.key, r.value))
        .collect();

    let msg = ControllerMessage::ServiceConfigDelivery(ServiceConfigDeliveryPayload::new(entries));
    if let Some(json) = serialize_controller_msg(out_seq, msg)
        && sink.send(Message::Text(json.into())).await.is_err()
    {
        return None;
    }

    Some(())
}

/// Handle a `StoreServiceConfig` message: upsert the entry, ACK, and broadcast.
pub(super) async fn handle_store_service_config(
    state: &Arc<AppState>,
    service_app_name: &str,
    service_id: uuid::Uuid,
    payload: StoreServiceConfigPayload,
) -> ProcessorResponse {
    let result = crate::queries::service_config::upsert(
        state.db(),
        service_app_name,
        payload.tenant_id,
        &payload.key,
        payload.value.clone(),
        payload.sensitive,
    )
    .await;

    match result {
        Ok(plaintext_value) => {
            // ACK to the requesting service.
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::success(
                payload.request_id,
            ));

            // Broadcast ServiceConfigUpdated to all OTHER instances of the same service.
            let update = ControllerMessage::ServiceConfigUpdated(ServiceConfigUpdatedPayload::new(
                vec![ServiceConfigEntry::new(
                    payload.tenant_id,
                    payload.key.clone(),
                    plaintext_value,
                )],
                vec![],
            ));
            state
                .service_connections
                .broadcast_to_app_except(service_app_name, service_id, update)
                .await;

            tracing::debug!(
                service_app_name,
                key = %payload.key,
                "stored service config entry"
            );
            ProcessorResponse::reply(ack)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %payload.key,
                "failed to store service config entry"
            );
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::error(
                payload.request_id,
                e.to_string(),
            ));
            ProcessorResponse::reply(ack)
        }
    }
}

/// Handle a `DeleteServiceConfig` message: delete the entry, ACK, and broadcast.
pub(super) async fn handle_delete_service_config(
    state: &Arc<AppState>,
    service_app_name: &str,
    service_id: uuid::Uuid,
    payload: DeleteServiceConfigPayload,
) -> ProcessorResponse {
    let result = crate::queries::service_config::delete(
        state.db(),
        service_app_name,
        payload.tenant_id,
        &payload.key,
    )
    .await;

    match result {
        Ok(_deleted) => {
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::success(
                payload.request_id,
            ));

            // Broadcast ServiceConfigUpdated to all OTHER instances.
            let update = ControllerMessage::ServiceConfigUpdated(ServiceConfigUpdatedPayload::new(
                vec![],
                vec![ServiceConfigKey::new(
                    payload.tenant_id,
                    payload.key.clone(),
                )],
            ));
            state
                .service_connections
                .broadcast_to_app_except(service_app_name, service_id, update)
                .await;

            tracing::debug!(
                service_app_name,
                key = %payload.key,
                "deleted service config entry"
            );
            ProcessorResponse::reply(ack)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %payload.key,
                "failed to delete service config entry"
            );
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::error(
                payload.request_id,
                e.to_string(),
            ));
            ProcessorResponse::reply(ack)
        }
    }
}
