//! Unified capability-gated WebSocket handler for all service types.
//!
//! This module replaces the three separate handlers (`agent_ws`, `mqtt_ws`,
//! `ssh_agent_ws`) with a single pair of handler functions that dispatch
//! messages based on the service's persisted capability set.
//!
//! ## Background message processing
//!
//! Heavy message processing (DB queries, notifications, etc.) is offloaded
//! to a [`MessageProcessor`] task spawned per connection. The main loop
//! reads WebSocket frames, handles lightweight inline operations (Ping/Pong,
//! Disconnecting, Unknown, Close, rate limiting), and forwards everything
//! else to the processor via a bounded MPSC channel.
//!
//! The processor handles messages sequentially (preserving ordering) and
//! sends [`ProcessorResponse`](shared_types::ProcessorResponse) values back
//! to the main loop, which serializes and writes replies to the WebSocket
//! sink with `out_seq` staying in the main loop.
//!
//! # Public API
//!
//! - [`handle_authenticated_loop`] -- post-certificate operational loop.
//! - [`handle_enrolled_loop`] -- pre-certificate enrollment loop.
//! - [`trigger_discovery_for_agent_host`] -- send `DiscoverSoftware` to an
//!   agent for a specific host (also used by `hosts.rs`).

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget sends in WS handler intentionally drop results"
)]

mod audit_service;
mod audit_surface;
mod cert;
mod credentials;
mod discovery;
mod embedded;
mod message_processor;
pub(super) mod messages;
mod reconnect;
mod renewal;
mod service_config;
mod session_authenticated;
mod shared_types;
mod surface_wire;
#[cfg(test)]
pub(super) mod test_support;
#[cfg(test)]
mod tests;
mod update_tracking;
mod updates;
mod workload;

use cert::{
    ApprovalPollResult, CertificateResult, handle_request_certificate, poll_approval_status,
};
pub(crate) use discovery::trigger_discovery_for_agent_host;
pub(crate) use embedded::{run_embedded_message_handler, run_embedded_system_message_handler};
pub(crate) use session_authenticated::handle_authenticated_loop;
pub(crate) use updates::dispatch_next_batch_update;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeSet;
use std::sync::Arc;

use sea_orm::EntityTrait;

use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_wire::{
    Capability, CloseReason, ControllerMessage, ErrorCode, ErrorPayload, IncomingSeq, OutgoingSeq,
    PingPayload, ServiceMessage,
};

use super::protocol::{
    MessageRateLimiter, WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW, close_with_reason,
    deserialize_service_msg, record_service_activity, record_system_service_activity, send_pong,
    serialize_controller_msg,
};
use crate::AppState;
use uptrakit_wire::service_profile::parse_capabilities;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Interval between approval-status DB polls in enrolled loops.
const APPROVAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Maximum time to wait for a WebSocket write (`sink.send()`) to complete.
///
/// If a service stops reading from the WebSocket, the OS TCP send buffer fills
/// and `sink.send()` blocks indefinitely. This timeout bounds the hang so that
/// the handler loop can break and clean up the connection. Kept deliberately
/// shorter than the agent-side `SEND_TIMEOUT` (30 s) so the controller detects
/// the stuck connection first.
const WS_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Maximum consecutive unknown messages before closing the connection.
///
/// Prevents a misbehaving or fuzzing client from keeping a connection alive
/// indefinitely by sending only garbage message types. Resets on any known
/// message.
const MAX_CONSECUTIVE_UNKNOWN_MESSAGES: u32 = 10;
const MQTT_SERVICE_APP_NAME: &str = "uptrakit-mqtt";

fn system_service_tenant_binding(
    service_app_name: Option<&str>,
    default_tenant_id: uuid::Uuid,
) -> Option<uuid::Uuid> {
    (service_app_name == Some(MQTT_SERVICE_APP_NAME)).then_some(default_tenant_id)
}

pub(super) fn is_valid_service_config_scope(
    service_tenant_id: Option<uuid::Uuid>,
    payload_tenant_id: Option<uuid::Uuid>,
) -> bool {
    match service_tenant_id {
        Some(bound_tenant_id) => payload_tenant_id == Some(bound_tenant_id),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// upgrade_service_capabilities
// ---------------------------------------------------------------------------

/// Persist the service's current capability set to the database and refresh
/// in-session gating flags.
pub(super) async fn upgrade_service_capabilities(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    is_system: bool,
    capabilities: std::collections::BTreeSet<Capability>,
    has_ui_surfaces: &mut bool,
) {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_wire::service_profile::serialize_capabilities;

    let new_caps_json = serialize_capabilities(&capabilities);
    let had_ui_surfaces = *has_ui_surfaces;
    *has_ui_surfaces = capabilities.contains(&Capability::UiSurfaces);

    if had_ui_surfaces != *has_ui_surfaces {
        tracing::info!(
            %service_id,
            ui_surfaces = *has_ui_surfaces,
            "service UiSurfaces capability changed in-session",
        );
    }

    let persist_result = if is_system {
        sys_svc_entity::ActiveModel {
            id: Set(service_id),
            capabilities: Set(new_caps_json),
            ..Default::default()
        }
        .update(db)
        .await
        .map(|_| ())
    } else {
        service::ActiveModel {
            id: Set(service_id),
            capabilities: Set(new_caps_json),
            ..Default::default()
        }
        .update(db)
        .await
        .map(|_| ())
    };

    match persist_result {
        Ok(()) => {
            tracing::debug!(%service_id, "persisted updated service capabilities");
        }
        Err(e) => {
            tracing::warn!(
                %service_id,
                error = %e,
                "failed to persist updated service capabilities"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// EnrolledSessionState
// ---------------------------------------------------------------------------

/// All state produced during enrolled session setup that the main loop and
/// cleanup phases need.
struct EnrolledSessionState {
    push_rx: tokio::sync::mpsc::Receiver<ControllerMessage>,
    cancel_token: tokio_util::sync::CancellationToken,
    approved: bool,
    rate_limiter: MessageRateLimiter,
    approval_poll: tokio::time::Interval,
}

// ---------------------------------------------------------------------------
// setup_enrolled_session
// ---------------------------------------------------------------------------

/// Perform all pre-loop setup for the enrolled handler.
///
/// Loads the service from the DB, registers the connection, detects the
/// external scheduler capability, and checks the initial approval status.
async fn setup_enrolled_session(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
) -> EnrolledSessionState {
    // Fetch service to derive capabilities and app name for registration.
    let (capabilities, service_app_name): (BTreeSet<Capability>, Option<String>) = if is_system {
        match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (parse_capabilities(&svc.capabilities), svc.service_app_name),
            _ => (BTreeSet::new(), None),
        }
    } else {
        match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (parse_capabilities(&svc.capabilities), svc.service_app_name),
            _ => (BTreeSet::new(), None),
        }
    };

    // Register in service_connections.
    let (push_rx, connection_handle) = state
        .service_connections
        .register(
            service_id,
            capabilities.clone(),
            None,
            None,
            service_app_name,
        )
        .await;
    let cancel_token =
        session_authenticated::cancellation_token_from_connection_handle(connection_handle);

    // Notify embedded services about the new external connection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_connected(service_id, &capabilities, None, is_system);
    }

    // Check current status to set initial approved flag.
    let mut approved = false;
    if is_system {
        if let Ok(Some(svc)) = sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
            && svc.status == sys_svc_entity::SystemServiceStatus::Approved
        {
            approved = true;
        }
    } else if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        && svc.status == service::ServiceStatus::Approved
    {
        approved = true;
        audit_service::emit_service_enrollment_completed_audit_event(state, service_id).await;
    }

    let rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Dedicated interval for polling approval status from the DB.
    let mut approval_poll = tokio::time::interval(APPROVAL_POLL_INTERVAL);
    approval_poll.tick().await; // skip immediate first tick

    EnrolledSessionState {
        push_rx,
        cancel_token,
        approved,
        rate_limiter,
        approval_poll,
    }
}

/// Clean up after an enrolled loop exits normally (not superseded).
async fn cleanup_enrolled_session(
    state: &AppState,
    service_id: uuid::Uuid,
    session: &EnrolledSessionState,
) {
    if session.cancel_token.is_cancelled() {
        return;
    }
    state.service_connections.unregister(&service_id).await;

    // Notify embedded services about the disconnection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_disconnected(&service_id);
    }

    tracing::debug!(%service_id, "enrolled service disconnected");
}

/// Unified enrolled handler for all service types.
///
/// Handles Ping, RequestCertificate, and polls for approval changes at a
/// fixed interval (decoupled from client-controlled ping frequency).
#[tracing::instrument(skip_all, fields(%service_id, is_system))]
pub(crate) async fn handle_enrolled_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    let mut session = setup_enrolled_session(state, service_id, is_system).await;

    loop {
        tokio::select! {
            msg = stream.next() => {
                let Some(msg) = msg else { break };
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "websocket receive error");
                        break;
                    }
                };
                if !session.rate_limiter.allow() {
                    let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                    break;
                }

                match msg {
                    Message::Text(text) => {
                        let service_msg: ServiceMessage =
                            match deserialize_service_msg(in_seq, &text) {
                                Ok(Some(m)) => m.message,
                                Ok(None) => continue,
                                Err(e) => {
                                    tracing::debug!(error = %e, "deserialize error");
                                    break;
                                }
                            };

                        match service_msg {
                            ServiceMessage::Ping(PingPayload { service_ts, .. }) => {
                                let Ok(controller_ts) =
                                    send_pong(sink, out_seq, service_ts).await
                                else {
                                    break;
                                };
                                tracing::trace!(
                                    service_ts,
                                    controller_ts,
                                    "ping/pong (enrolled)"
                                );
                                let activity_result = if is_system {
                                    record_system_service_activity(
                                        state.db(),
                                        service_id,
                                        None,
                                    )
                                    .await
                                } else {
                                    record_service_activity(state.db(), service_id, None).await
                                };
                                if let Err(e) = activity_result {
                                    tracing::warn!(
                                        error = %e,
                                        %service_id,
                                        "failed to record service activity"
                                    );
                                }
                            }
                            ServiceMessage::RequestCertificate(payload) => {
                                match handle_request_certificate(
                                    sink, state, service_id, is_system,
                                    session.approved, out_seq, &payload,
                                ).await {
                                    CertificateResult::Break => break,
                                    CertificateResult::NotApproved => continue,
                                }
                            }
                            ServiceMessage::Enroll(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "already enrolled".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "service disconnecting gracefully during enrollment"
                                );
                                break;
                            }
                            ServiceMessage::AuditEvent(payload) => {
                                let _ = audit_service::ingest_service_audit_event(
                                    state,
                                    service_id,
                                    is_system,
                                    None,
                                    None,
                                    payload,
                                )
                                .await;
                            }
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            push = session.push_rx.recv() => {
                let Some(msg) = push else { break };

                // Track state transitions; handle Rejected specially (send + break).
                let is_rejected = matches!(&msg, ControllerMessage::Rejected(_));
                if matches!(&msg, ControllerMessage::Approved(_)) {
                    session.approved = true;
                }

                let Some(json) = serialize_controller_msg(out_seq, msg) else { break };
                match tokio::time::timeout(
                    WS_WRITE_TIMEOUT,
                    sink.send(Message::Text(json.into())),
                ).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => {
                        tracing::warn!(
                            %service_id,
                            "WebSocket write timed out after {}s during enrollment, dropping connection",
                            WS_WRITE_TIMEOUT.as_secs(),
                        );
                        break;
                    }
                }
                if is_rejected {
                    break;
                }
            }
            // Dedicated approval poll at a fixed interval.
            _ = session.approval_poll.tick(), if !session.approved => {
                match poll_approval_status(sink, state, service_id, is_system, out_seq).await {
                    ApprovalPollResult::Approved => session.approved = true,
                    ApprovalPollResult::Rejected => break,
                    ApprovalPollResult::Unchanged => {}
                }
            }
            _ = session.cancel_token.cancelled() => {
                tracing::info!(%service_id, "enrolled connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Same as authenticated: genuine supersession skips cleanup, but
                // force-disconnect removes the registry entry so we must notify.
                if !state.service_connections.is_connected(&service_id).await
                    && let Some(ref notifier) = state.embedded_service_notifier
                {
                    notifier.on_external_disconnected(&service_id);
                }
                return;
            }
        }
    }

    cleanup_enrolled_session(state, service_id, &session).await;
}
