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
    MqttReleaseTenantsPayload, MqttTenantConfig, MqttTriggerHostPackageUpdatePayload,
    MqttUpdateTriggerPayload, OutgoingSeq, PingPayload, ServiceMessage,
};
use uptrakit_web_api_types::settings_mqtt::MqttClientConnectionStatus as ApiMqttClientConnectionStatus;

use super::LoopAction;
use crate::AppState;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;
use crate::queries::update_types::ActorType;
use crate::routes::service_ws::protocol::{
    MessageRateLimiter, close_with_reason, deserialize_service_msg, record_service_activity,
    send_pong, serialize_controller_msg,
};

// ---------------------------------------------------------------------------
// MqttHandshake / MqttContext
// ---------------------------------------------------------------------------

/// Intermediate result of the MQTT `Register` handshake.
///
/// Returned before the service is registered in `ServiceConnectionRegistry`
/// so that the caller can register first, then call
/// [`complete_mqtt_registration`] to assign or reconcile leases.
pub(super) struct MqttHandshake {
    pub(super) instance_id: String,
    pub(super) max_tenants: u32,
    pub(super) active_mqtt_clients: Vec<uuid::Uuid>,
}

/// Full MQTT context available after registration and lease assignment.
pub(super) struct MqttContext {
    pub(super) instance_id: String,
    pub(super) tenant_configs: Vec<MqttTenantConfig>,
}

// ---------------------------------------------------------------------------
// handle_mqtt_register_handshake
// ---------------------------------------------------------------------------

/// Wait for the MQTT `Register` message and return a [`MqttHandshake`].
///
/// This is the first phase of MQTT setup and deliberately does **not** touch
/// `ServiceConnectionRegistry` or perform any lease operations.  The caller
/// must register the service before calling [`complete_mqtt_registration`].
///
/// Returns `None` if the connection is closed or the phase fails.
pub(super) async fn handle_mqtt_register_handshake(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    rate_limiter: &mut MessageRateLimiter,
) -> Option<MqttHandshake> {
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
                    ServiceMessage::Ping(PingPayload { service_ts, .. }) => {
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

    Some(MqttHandshake {
        instance_id,
        max_tenants,
        active_mqtt_clients,
    })
}

// ---------------------------------------------------------------------------
// complete_mqtt_registration
// ---------------------------------------------------------------------------

/// Assign or reconcile MQTT tenant leases after service registration.
///
/// Must be called **after** the service has been added to
/// `ServiceConnectionRegistry` so that capacity queries succeed.
/// Returns the [`MqttContext`] used by the main authenticated loop.
pub(super) async fn complete_mqtt_registration(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    handshake: MqttHandshake,
) -> MqttContext {
    let lease_coordinator =
        MqttLeaseCoordinator::new(state.db().clone(), state.service_connections.clone());

    let tenant_configs = if !handshake.active_mqtt_clients.is_empty() {
        match lease_coordinator
            .reconcile_mqtt_clients(service_id, &handshake.instance_id, &handshake.active_mqtt_clients)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to reconcile mqtt clients");
                vec![]
            }
        }
    } else {
        let requested = if handshake.max_tenants == 0 { 100 } else { handshake.max_tenants };
        match lease_coordinator
            .assign_available_tenants(service_id, &handshake.instance_id, requested)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to assign mqtt clients");
                vec![]
            }
        }
    };

    MqttContext {
        instance_id: handshake.instance_id,
        tenant_configs,
    }
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
        _ => ApiMqttClientConnectionStatus::Offline,
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
            actor_type: ActorType::Mqtt,
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
            // Push updated software states immediately so that the MQTT/HA
            // entity reflects `in_progress: true` without waiting for the
            // agent's UpdateStarted message.
            state
                .notification_service
                .push_software_states_for_tenant(state.db(), payload.tenant_id)
                .await;
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

// ---------------------------------------------------------------------------
// handle_mqtt_trigger_host_package_update
// ---------------------------------------------------------------------------

/// Handle a `MqttTriggerHostPackageUpdate` message: trigger a batch update of
/// all outdated host packages on a host.
pub(super) async fn handle_mqtt_trigger_host_package_update(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    payload: &MqttTriggerHostPackageUpdatePayload,
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

    match crate::queries::update_batches::trigger_all_host_package_updates_for_host(
        state.db(),
        &state.notification_service,
        payload.tenant_id,
        payload.host_id,
        ActorType::Mqtt,
        &payload.mqtt_client_id.to_string(),
        payload.security_only,
    )
    .await
    {
        Ok(Some(batch_id)) => {
            tracing::info!(
                %batch_id,
                host_id = %payload.host_id,
                mqtt_client_id = %payload.mqtt_client_id,
                "MQTT-triggered host package batch update dispatched"
            );
            // Push updated software states so that `update_in_progress: true`
            // is reflected in the MQTT/HA entity immediately.
            state
                .notification_service
                .push_software_states_for_tenant(state.db(), payload.tenant_id)
                .await;
        }
        Ok(None) => {
            let err_msg = ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: "no outdated packages found on this host".to_string(),
            });
            if let Some(json) = serialize_controller_msg(out_seq, err_msg) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                host_id = %payload.host_id,
                "MQTT-triggered host package batch update failed"
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
