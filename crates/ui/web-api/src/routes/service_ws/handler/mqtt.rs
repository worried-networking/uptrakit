//! MQTT register phase handler and MQTT-specific message handlers.
//!
//! Contains the `MqttContext` struct, `handle_mqtt_register_phase`, and
//! handler functions for MQTT-specific match arms (`ReleaseTenants`,
//! `MqttClientStatus`, `MqttTriggerUpdate`).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};

use uptrakit_internal_wire::{
    CloseReason, ControllerMessage, ErrorCode, ErrorPayload, IncomingSeq,
    MqttClientConnectionStatus as WireMqttClientConnectionStatus, MqttClientStatusPayload,
    MqttReleaseTenantsPayload, MqttTenantConfig, MqttUpdateTriggerPayload, OutgoingSeq,
    PingPayload, ServiceMessage,
};
use uptrakit_web_api_types::settings_mqtt::MqttClientConnectionStatus as ApiMqttClientConnectionStatus;

use super::LoopAction;
use crate::AppState;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;
use crate::routes::service_ws::protocol::{
    MessageRateLimiter, close_with_reason, deserialize_service_msg, record_service_activity,
    send_pong, serialize_controller_msg,
};

// ---------------------------------------------------------------------------
// MqttContext
// ---------------------------------------------------------------------------

/// Context returned by the MQTT register phase.
pub(super) struct MqttContext {
    pub(super) instance_id: String,
    pub(super) max_tenants: u32,
    pub(super) tenant_configs: Vec<MqttTenantConfig>,
}

// ---------------------------------------------------------------------------
// handle_mqtt_register_phase
// ---------------------------------------------------------------------------

/// Handle the MQTT Register handshake (pre-loop phase).
///
/// Waits for a `Register` message, reconciles or assigns tenants, and returns
/// the MQTT context. Returns `None` if the connection is closed or the phase
/// fails.
pub(super) async fn handle_mqtt_register_phase(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    rate_limiter: &mut MessageRateLimiter,
) -> Option<MqttContext> {
    // Wait for Register message.
    let (instance_id, max_tenants, active_mqtt_clients) = loop {
        let msg = match stream.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                tracing::debug!(error = %e, "websocket receive error");
                return None;
            }
            None => return None,
        };

        if !rate_limiter.allow() {
            let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
            return None;
        }

        match msg {
            Message::Text(text) => {
                let service_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                    Ok(Some(m)) => m,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::debug!(error = %e, "deserialize error");
                        return None;
                    }
                };

                match service_msg {
                    ServiceMessage::Register(payload) => {
                        tracing::debug!(
                            %service_id,
                            capabilities = ?payload.capabilities,
                            "received Register"
                        );
                        break (
                            payload.instance_id,
                            payload.max_tenants,
                            payload.active_mqtt_clients,
                        );
                    }
                    ServiceMessage::Ping(PingPayload { service_ts }) => {
                        if send_pong(sink, out_seq, service_ts).await.is_err() {
                            return None;
                        }
                        if let Err(e) = record_service_activity(state.db(), service_id, None).await
                        {
                            tracing::warn!(
                                error = %e,
                                %service_id,
                                "failed to record service activity"
                            );
                        }
                    }
                    _ => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: ErrorCode::BadRequest,
                            message: "expected register message".to_string(),
                        });
                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        return None;
                    }
                }
            }
            Message::Close(_) => return None,
            _ => {}
        }
    };

    // Create lease coordinator for tenant reconciliation.
    let lease_coordinator =
        MqttLeaseCoordinator::new(state.db().clone(), state.service_connections.clone());

    // Reconcile MQTT clients if reconnecting with active clients.
    let tenant_configs = if !active_mqtt_clients.is_empty() {
        match lease_coordinator
            .reconcile_mqtt_clients(service_id, &instance_id, &active_mqtt_clients)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to reconcile mqtt clients");
                vec![]
            }
        }
    } else {
        let requested = if max_tenants == 0 { 100 } else { max_tenants };
        match lease_coordinator
            .assign_available_tenants(service_id, &instance_id, requested)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to assign mqtt clients");
                vec![]
            }
        }
    };

    Some(MqttContext {
        instance_id,
        max_tenants,
        tenant_configs,
    })
}

// ---------------------------------------------------------------------------
// handle_release_tenants
// ---------------------------------------------------------------------------

/// Handle a `ReleaseTenants` message: release MQTT client leases.
pub(super) async fn handle_release_tenants(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &MqttReleaseTenantsPayload,
    lease_coordinator: Option<&MqttLeaseCoordinator>,
) -> LoopAction {
    // Suppress unused-variable warning -- state is part of the standard
    // handler signature but not directly used here.
    let _ = state;

    if let Some(lc) = lease_coordinator
        && let Err(e) = lc
            .release_mqtt_clients(&service_id, &payload.mqtt_client_ids)
            .await
    {
        tracing::warn!(
            error = %e,
            "failed to release mqtt clients"
        );
    }

    tracing::info!(
        %service_id,
        count = payload.mqtt_client_ids.len(),
        "MQTT service released mqtt clients"
    );

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_mqtt_client_status
// ---------------------------------------------------------------------------

/// Handle a `MqttClientStatus` message: update MQTT client connection status.
pub(super) async fn handle_mqtt_client_status(
    state: &Arc<AppState>,
    payload: &MqttClientStatusPayload,
) -> LoopAction {
    let status = match payload.status {
        WireMqttClientConnectionStatus::Online => ApiMqttClientConnectionStatus::Online,
        WireMqttClientConnectionStatus::Offline => ApiMqttClientConnectionStatus::Offline,
        WireMqttClientConnectionStatus::Connecting => ApiMqttClientConnectionStatus::Connecting,
    };

    if let Err(e) = crate::mqtt_client_store::update_mqtt_client_status(
        state.db(),
        payload.mqtt_client_id,
        status,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            "failed to update mqtt client status"
        );
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_mqtt_trigger_update
// ---------------------------------------------------------------------------

/// Handle a `MqttTriggerUpdate` message: validate tenant assignment, trigger
/// update for host.
pub(super) async fn handle_mqtt_trigger_update(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    payload: &MqttUpdateTriggerPayload,
    mqtt_context: Option<&MqttContext>,
) -> LoopAction {
    // Validate tenant is assigned to this MQTT service.
    let tenant_assigned = mqtt_context
        .map(|mctx| {
            mctx.tenant_configs
                .iter()
                .any(|c| c.tenant_id == payload.tenant_id)
        })
        .unwrap_or(false);

    if !tenant_assigned {
        let err_msg = ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::BadRequest,
            message: "tenant not assigned to this MQTT service".to_string(),
        });
        if let Some(json) = serialize_controller_msg(out_seq, err_msg) {
            let _ = sink.send(Message::Text(json.into())).await;
        }
        return LoopAction::Continue;
    }

    match crate::queries::update_triggers::trigger_update_for_host(
        state.db(),
        &state.notification_service,
        crate::queries::update_triggers::TriggerUpdateParams {
            tenant_id: payload.tenant_id,
            item_id: payload.software_item_id,
            host_id: payload.host_id,
            to_version: payload.to_version.clone(),
            actor_type: "mqtt",
            actor_id: &payload.mqtt_client_id.to_string(),
            release_info: None,
        },
    )
    .await
    {
        Ok(result) => {
            tracing::info!(
                update_id = %result.update_history_id,
                software_item_id = %payload.software_item_id,
                host_id = %payload.host_id,
                mqtt_client_id = %payload.mqtt_client_id,
                agent_connected = result.agent_connected,
                "MQTT-triggered update dispatched"
            );
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                software_item_id = %payload.software_item_id,
                host_id = %payload.host_id,
                "MQTT-triggered update failed"
            );
            let err_msg = ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: err.to_string(),
            });
            if let Some(json) = serialize_controller_msg(out_seq, err_msg) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
        }
    }

    LoopAction::Continue
}
