//! Enrolled-session lifecycle for the WebSocket handler.
//!
//! Covers: session state, setup helpers, the main event loop, and cleanup.
//! Extracted from `mod.rs` to keep the enrolled-session logic self-contained.

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget sends in WS handler intentionally drop results"
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::EntityTrait;

use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_wire::{
    Capability, CloseReason, ControllerMessage, ErrorCode, ErrorPayload, IncomingSeq, OutgoingSeq,
    PingPayload, ServiceMessage,
};

use super::super::protocol::{
    MessageRateLimiter, WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW, close_with_reason,
    deserialize_service_msg, record_service_activity, record_system_service_activity, send_pong,
    serialize_controller_msg,
};
use super::audit_service;
use super::cert::{
    ApprovalPollResult, CertificateResult, handle_request_certificate, poll_approval_status,
};
use super::shared_types::WS_WRITE_TIMEOUT;
use crate::AppState;
use uptrakit_wire::service_profile::parse_capabilities;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Interval between approval-status DB polls in enrolled loops.
const APPROVAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

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
    connection_id: uuid::Uuid,
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
    let connection_id = connection_handle.connection_id();
    let cancel_token =
        super::session_authenticated::cancellation_token_from_connection_handle(connection_handle);

    // Notify embedded services about the new external connection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_connected(service_id, &capabilities, None, is_system);
    }
    // Now-yielded embedded services must give up their surface providers
    // before this connection's own registration can be admitted, or a
    // matching Universal registration would collide (A3).
    super::surface_eviction::evict_yielded_service_surfaces(state).await;

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
        connection_id,
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
    // Race-safe: only remove if this cleanup still owns the current registration.
    if state
        .service_connections
        .unregister_current(&service_id, session.connection_id)
        .await
    {
        // Notify embedded services about the disconnection.
        if let Some(ref notifier) = state.embedded_service_notifier {
            notifier.on_external_disconnected(&service_id);
        }
        tracing::debug!(%service_id, "enrolled service disconnected");
    } else {
        tracing::debug!(
            %service_id,
            connection_id = %session.connection_id,
            "enrolled cleanup skipped — connection already replaced by a reconnect"
        );
    }
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

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    use uuid::Uuid;

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn enrolled_cleanup_stale_connection_does_not_evict_replacement() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

        // A's session sets up (registers A). session.connection_id == A's id.
        let mut session = setup_enrolled_session(&state, service_id, false).await;
        // B reconnects and supersedes A in the registry.
        let (_rx_b, handle_b) = state
            .service_connections
            .register(
                service_id,
                std::collections::BTreeSet::new(),
                None,
                None,
                None,
            )
            .await;

        // Force the guarded path: a fresh, uncancelled token so cleanup does NOT
        // early-return, but the session still holds A's stale connection_id.
        session.cancel_token = tokio_util::sync::CancellationToken::new();
        cleanup_enrolled_session(&state, service_id, &session).await;

        // unregister_current(A) returns false because B is current → B survives.
        assert!(state.service_connections.is_connected(&service_id).await);
        assert_eq!(
            state
                .service_connections
                .current_connection_id(&service_id)
                .await,
            Some(handle_b.connection_id()),
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn setup_enrolled_session_emits_enrollment_completed_audit_for_already_approved_service()
    {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

        let session = setup_enrolled_session(&state, service_id, false).await;
        assert!(session.approved);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
    }
}
