//! Authenticated-session lifecycle for the WebSocket handler.
//!
//! Covers: session state, setup helpers, the main event loop, and cleanup /
//! finalization. Extracted from `mod.rs` to keep the file focused on the
//! enrolled-session lifecycle.

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget sends in WS handler intentionally drop results"
)]

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::EntityTrait;

use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::{
    Capability, CloseReason, ControllerMessage, HostConnectivityUpdate, IncomingSeq, OutgoingSeq,
    PingPayload, RegisterPayload, ServiceMessage,
};

use super::super::protocol::{
    AuthenticatedContext, CertIdentity, MessageRateLimiter, WS_MESSAGE_RATE_LIMIT,
    WS_MESSAGE_RATE_WINDOW, close_with_reason, deserialize_service_msg, serialize_controller_msg,
};
use super::credentials::{self, ServiceCredentialTarget};
use super::message_processor::{MessageProcessor, ProcessorMessage, spawn_message_processor};
use super::session_enrolled::upgrade_service_capabilities;
use super::shared_types::WS_WRITE_TIMEOUT;
use super::shared_types::{ProcessorAction, ProcessorResponse, load_linked_host_ids};
use super::{messages, reconnect, service_config, workload};
use crate::AppState;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::service_profile::parse_capabilities;

/// Maximum consecutive unknown messages before closing the connection.
///
/// Prevents a misbehaving or fuzzing client from keeping a connection alive
/// indefinitely by sending only garbage message types. Resets on any known
/// message.
const MAX_CONSECUTIVE_UNKNOWN_MESSAGES: u32 = 10;

// ---------------------------------------------------------------------------
// send_ws_with_timeout
// ---------------------------------------------------------------------------

/// Send a serialized WebSocket message with a timeout, returning `true`
/// on success and `false` if the write failed or timed out.
pub(super) async fn send_ws_with_timeout(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    json: String,
    service_id: uuid::Uuid,
) -> bool {
    match tokio::time::timeout(WS_WRITE_TIMEOUT, sink.send(Message::Text(json.into()))).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            tracing::warn!(
                %service_id,
                "WebSocket write timed out after {}s, dropping connection",
                WS_WRITE_TIMEOUT.as_secs(),
            );
            false
        }
    }
}

// ---------------------------------------------------------------------------
// AuthenticatedSessionState
// ---------------------------------------------------------------------------

/// All state produced during authenticated session setup that the main loop
/// and cleanup phases need.
pub(super) struct AuthenticatedSessionState {
    pub(super) service_id: uuid::Uuid,
    /// The `connection_id` minted by the registry at `register()` time.
    /// Used by cleanup to drive `unregister_current` (race-safe removal) and by
    /// `authenticated_session_ownership` to distinguish current from replaced.
    pub(super) connection_id: uuid::Uuid,
    pub(super) is_system: bool,
    pub(super) has_update_tracking: bool,
    pub(super) has_software_discovery: bool,
    pub(super) has_workload_claims: bool,
    pub(super) service_tenant_id: Option<uuid::Uuid>,
    pub(super) linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    pub(super) push_rx: tokio::sync::mpsc::Receiver<ControllerMessage>,
    pub(super) cancel_token: tokio_util::sync::CancellationToken,
    pub(super) msg_tx: tokio::sync::mpsc::Sender<ProcessorMessage>,
    pub(super) resp_rx: tokio::sync::mpsc::Receiver<ProcessorResponse>,
    pub(super) processor_cancel: tokio_util::sync::CancellationToken,
    pub(super) processor_handle: tokio::task::JoinHandle<()>,
    pub(super) rate_limiter: MessageRateLimiter,
}

// ---------------------------------------------------------------------------
// setup_authenticated_session — stage helpers
// ---------------------------------------------------------------------------

/// Stage 1: Load service from DB and return capabilities, app name, and tenant
/// ID.
///
/// Falls back to empty capabilities on any DB error or missing row so that
/// the setup can continue with a degraded (no-capability) service rather than
/// crashing.
pub(super) async fn load_service_capabilities(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
) -> (BTreeSet<Capability>, Option<String>, Option<uuid::Uuid>) {
    if is_system {
        match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (
                parse_capabilities(&svc.capabilities),
                svc.service_app_name.clone(),
                super::shared_types::system_service_tenant_binding(
                    svc.service_app_name.as_deref(),
                    state.default_tenant_id,
                ),
            ),
            _ => (BTreeSet::new(), None, None),
        }
    } else {
        match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => (
                parse_capabilities(&svc.capabilities),
                svc.service_app_name,
                Some(svc.tenant_id),
            ),
            _ => (BTreeSet::new(), None, None),
        }
    }
}

/// Stage 3: Register the connection in `ServiceConnectionRegistry` and notify
/// the embedded service infrastructure about the new external connection.
///
/// Returns `(push_rx, cancel_token, connection_id)`. The `connection_id` is
/// read from the handle the registry returns — there is no second registry
/// lookup, so an admin `force_disconnect` racing in between cannot cause a
/// panic.
pub(super) fn cancellation_token_from_connection_handle(
    connection: crate::service_connections::ServiceConnectionHandle,
) -> tokio_util::sync::CancellationToken {
    let cancel_token = tokio_util::sync::CancellationToken::new();
    let notify_token = cancel_token.clone();
    tokio::spawn(async move {
        connection.cancelled().await;
        notify_token.cancel();
    });
    cancel_token
}

pub(super) async fn register_connection(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    capabilities: &BTreeSet<Capability>,
    service_app_name: Option<String>,
) -> (
    tokio::sync::mpsc::Receiver<ControllerMessage>,
    tokio_util::sync::CancellationToken,
    uuid::Uuid,
) {
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

    // Notify embedded services about the new external connection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_connected(service_id, capabilities, None, false);
    }

    let connection_id = connection_handle.connection_id();
    let cancel_token = cancellation_token_from_connection_handle(connection_handle);

    (push_rx, cancel_token, connection_id)
}

/// Stage 4: Load linked host IDs shared between the main loop and the processor.
pub(super) async fn load_session_host_ids(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    has_software_discovery: bool,
) -> Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>> {
    if has_software_discovery {
        Arc::new(parking_lot::Mutex::new(
            load_linked_host_ids(state.db(), service_id)
                .await
                .unwrap_or_default(),
        ))
    } else {
        Arc::new(parking_lot::Mutex::new(HashSet::new()))
    }
}

/// Stage 5: Prepare reconnect cleanup and any replayable pending updates.
///
/// Errors are logged but do not abort setup — the connection is still usable.
pub(super) async fn prepare_reconnect_updates_on_connect(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    runtime_instance_id: Option<uuid::Uuid>,
    has_update_hooks: bool,
    out_seq: &mut OutgoingSeq,
) {
    let replay = reconnect::prepare_reconnect_replay(
        state,
        service_id,
        runtime_instance_id,
        has_update_hooks,
        true,
    )
    .await;

    for msg in replay.messages {
        let Some(json) = serialize_controller_msg(out_seq, msg) else {
            continue;
        };
        if !send_ws_with_timeout(sink, json, service_id).await {
            tracing::error!(%service_id, "failed to send replayed pending update on reconnect");
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// receive_register_message
// ---------------------------------------------------------------------------

/// Read the first frame from the service and expect it to be a `Register` message.
///
/// Called as Stage 3 of [`setup_authenticated_session`], immediately after
/// credential and config delivery, before the service begins sending
/// operational messages. The service must send `Register` synchronously from
/// `on_connected` so it arrives here before any other message.
///
/// Returns `Some(RegisterPayload)` on success, or `None` if:
/// - The connection closed or produced a read error.
/// - Rate limiting was exceeded.
/// - Deserialization failed (hard error — malformed frame).
/// - The first message was not `ServiceMessage::Register`.
///
/// On failure the connection is closed with [`CloseReason::ProtocolError`].
pub(super) async fn receive_register_message(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    rate_limiter: &mut MessageRateLimiter,
) -> Option<RegisterPayload> {
    use futures_util::StreamExt as _;

    let _ = (state, out_seq); // unused but kept for consistency with other stage helpers

    let frame = match stream.next().await {
        Some(Ok(f)) => f,
        Some(Err(e)) => {
            tracing::debug!(%service_id, error = %e, "websocket read error waiting for Register");
            return None;
        }
        None => {
            tracing::debug!(%service_id, "connection closed before Register was received");
            return None;
        }
    };

    let text = match frame {
        Message::Text(t) => t,
        Message::Close(_) => {
            tracing::debug!(%service_id, "received Close frame waiting for Register");
            return None;
        }
        _ => {
            tracing::warn!(%service_id, "expected text frame for Register, got non-text frame");
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            return None;
        }
    };

    if !rate_limiter.allow() {
        tracing::warn!(%service_id, "rate limit exceeded on Register frame");
        let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
        return None;
    }

    let deserialized = match deserialize_service_msg(in_seq, &text) {
        Ok(Some(d)) => d,
        Ok(None) => {
            tracing::warn!(%service_id, "Register frame could not be deserialized (unknown type)");
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            return None;
        }
        Err(e) => {
            tracing::debug!(%service_id, error = %e, "hard deserialize error on Register frame");
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            return None;
        }
    };

    match deserialized.message {
        ServiceMessage::Register(payload) => {
            tracing::debug!(
                %service_id,
                capabilities = ?payload.capabilities,
                "received Register from service"
            );
            Some(payload)
        }
        other => {
            tracing::warn!(
                %service_id,
                message_type = ?std::mem::discriminant(&other),
                "expected Register as first message, got unexpected variant; closing connection"
            );
            let _ = close_with_reason(sink, CloseReason::ProtocolError).await;
            None
        }
    }
}

// ---------------------------------------------------------------------------
// setup_authenticated_session
// ---------------------------------------------------------------------------

/// Perform all pre-loop setup for the authenticated handler.
///
/// Loads the service from the DB, delivers credentials, receives the Register
/// handshake, registers the connection, spawns the background processor, and
/// delivers pending updates.
///
/// Returns `None` if the connection must be closed early (e.g. failed Register
/// handshake or write failure).
// All parameters originate from the caller's `AuthenticatedContext` and cannot
// be meaningfully grouped without introducing a wrapper that duplicates it.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters originate from caller context and cannot be meaningfully grouped without wrapper duplication"
)]
pub(super) async fn setup_authenticated_session(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    cert: &CertIdentity,
    is_system: bool,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) -> Option<AuthenticatedSessionState> {
    // Stage 1: Load service record from DB. The DB capabilities are used for
    // credential delivery (DatabaseAccess, NatsAccess, etc.). Session-level
    // capability flags (has_update_tracking, has_software_discovery, etc.) come from the
    // Register handshake in Stage 3 so they are correct on first connect even
    // when the DB row has no stored capabilities yet.
    let (db_capabilities, service_app_name, service_tenant_id) =
        load_service_capabilities(state, service_id, is_system).await;

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Stage 2: Deliver credentials to services that have credential capabilities.
    credentials::deliver_service_credentials(
        sink,
        state,
        &db_capabilities,
        ServiceCredentialTarget {
            service_id,
            is_system,
            service_tenant_id,
            service_app_name: service_app_name.as_deref(),
        },
        out_seq,
    )
    .await?;

    // Stage 2.5: Deliver stored service config entries to services with a known app name.
    if let Some(ref app_name) = service_app_name {
        service_config::deliver_service_config(
            sink,
            state,
            service_id,
            is_system,
            service_tenant_id,
            app_name,
            out_seq,
        )
        .await?;
    }

    // Stage 3: Receive the Register handshake from the service.
    //
    // The service sends `Register` from `on_connected` immediately after the
    // controller completes credential + config delivery. This gives us the
    // authoritative session-level capability set before we register the
    // connection, so all downstream decisions use live data rather than
    // potentially-stale DB values.
    let register_payload = receive_register_message(
        sink,
        stream,
        state,
        service_id,
        out_seq,
        in_seq,
        &mut rate_limiter,
    )
    .await?;
    let runtime_instance_id = register_payload.runtime_instance_id;

    let session_capabilities = register_payload.capabilities.clone();
    let has_update_tracking = session_capabilities.contains(&Capability::UpdateTracking);
    let has_software_discovery = session_capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = session_capabilities.contains(&Capability::UpdateHooks);
    let has_ui_surfaces = session_capabilities.contains(&Capability::UiSurfaces);
    let has_workload_claims = session_capabilities.contains(&Capability::WorkloadClaims);

    // Persist the session capabilities to the DB so that subsequent reconnects
    // (and other controller instances) see the up-to-date capability set.
    upgrade_service_capabilities(
        state.db(),
        service_id,
        is_system,
        register_payload.capabilities,
        &mut { has_ui_surfaces },
    )
    .await;

    // Stage 4: Register the connection and notify embedded services.
    let (push_rx, cancel_token, connection_id) = register_connection(
        state,
        service_id,
        &session_capabilities,
        service_app_name.clone(),
    )
    .await;

    // Stage 5: Load linked host IDs shared between the main loop and the processor.
    let linked_host_ids = load_session_host_ids(state, service_id, has_software_discovery).await;

    // Stage 6: Recover interrupted owned updates and replay any pending updates.
    prepare_reconnect_updates_on_connect(
        sink,
        state,
        service_id,
        runtime_instance_id,
        has_update_hooks,
        out_seq,
    )
    .await;

    // Stage 7: Spawn the background message processor.
    let processor = MessageProcessor {
        state: Arc::clone(state),
        service_id,
        cert: Some(cert.clone()),
        is_system,
        has_update_tracking,
        has_software_discovery,
        has_update_hooks,
        has_ui_surfaces,
        has_workload_claims,
        runtime_instance_id,
        service_app_name,
        service_tenant_id,
        linked_host_ids: Arc::clone(&linked_host_ids),
        report_tracker: ReportTracker::new(),
    };
    let channels = spawn_message_processor(processor);

    Some(AuthenticatedSessionState {
        service_id,
        connection_id,
        is_system,
        has_update_tracking,
        has_software_discovery,
        has_workload_claims,
        service_tenant_id,
        linked_host_ids,
        push_rx,
        cancel_token,
        msg_tx: channels.msg_tx,
        resp_rx: channels.resp_rx,
        processor_cancel: channels.processor_cancel,
        processor_handle: channels.processor_handle,
        rate_limiter,
    })
}

// ---------------------------------------------------------------------------
// cleanup_authenticated_session
// ---------------------------------------------------------------------------

/// Perform all cleanup after the authenticated loop exits normally (not
/// superseded).
pub(super) async fn cleanup_authenticated_session(
    state: &Arc<AppState>,
    session: AuthenticatedSessionState,
) {
    let AuthenticatedSessionState {
        service_id,
        connection_id,
        is_system,
        has_update_tracking,
        has_software_discovery,
        has_workload_claims,
        service_tenant_id,
        linked_host_ids,
        processor_cancel,
        processor_handle,
        ..
    } = session;

    // Cancel the processor task and wait for it to finish.
    processor_cancel.cancel();
    let _ = processor_handle.await;

    // Release all workload claims held by this service.
    if has_workload_claims {
        workload::release_all_claims_on_disconnect(state, service_id).await;
    }

    // Cleanup must not rely on the session-start UiSurfaces snapshot because
    // services can upgrade capabilities in-session via Register.
    if let Some(provider_id) = state
        .surface_proxy_deps
        .registry
        .provider_id_for_service(&service_id)
    {
        state
            .surface_proxy_deps
            .proxy
            .fail_in_flight_for_provider(&provider_id);
        if let Some(tenant_id) = service_tenant_id
            && !state.shutdown_token.is_cancelled()
        {
            state
                .notification
                .event_broadcaster
                .send(tenant_id, AdminEvent::SurfacesChanged)
                .await;
        }
    }
    state
        .surface_proxy_deps
        .registry
        .unregister_service(&service_id);

    // Notify services that this agent's hosts are now offline.
    if !is_system
        && !has_update_tracking
        && has_software_discovery
        && let Some(tenant_id) = service_tenant_id
    {
        let current_ids = linked_host_ids.lock().clone();
        if !current_ids.is_empty() {
            let now = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default();
            let updates: Vec<HostConnectivityUpdate> = current_ids
                .iter()
                .map(|&host_id| HostConnectivityUpdate::offline(host_id, Some(now.clone())))
                .collect();
            state
                .notification
                .notification_service
                .send_connectivity_update(tenant_id, updates)
                .await;
        }
    }

    // Race-safe: a service reconnecting during this cleanup's awaits registers a
    // replacement; unregister_current removes only if we still own the current
    // registration, so the live replacement survives.
    if state
        .service_connections
        .unregister_current(&service_id, connection_id)
        .await
    {
        // Notify embedded services about the disconnection.
        if let Some(ref notifier) = state.embedded_service_notifier {
            notifier.on_external_disconnected(&service_id);
        }
    } else {
        tracing::debug!(
            %service_id,
            %connection_id,
            "cleanup skipped — connection already replaced by a reconnect"
        );
    }

    tracing::debug!(%service_id, "authenticated service disconnected");
}

pub(super) async fn handle_cancelled_authenticated_session_after_close(
    state: &Arc<AppState>,
    session: AuthenticatedSessionState,
) {
    finalize_authenticated_session(state, session).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AuthenticatedSessionOwnership {
    Current,
    Replaced,
    Removed,
}

pub(super) async fn authenticated_session_ownership(
    state: &Arc<AppState>,
    session: &AuthenticatedSessionState,
) -> AuthenticatedSessionOwnership {
    match state
        .service_connections
        .current_connection_id(&session.service_id)
        .await
    {
        Some(id) if id == session.connection_id => AuthenticatedSessionOwnership::Current,
        Some(_) => AuthenticatedSessionOwnership::Replaced,
        None => AuthenticatedSessionOwnership::Removed,
    }
}

pub(super) async fn finalize_authenticated_session(
    state: &Arc<AppState>,
    session: AuthenticatedSessionState,
) {
    match authenticated_session_ownership(state, &session).await {
        AuthenticatedSessionOwnership::Replaced => {
            let AuthenticatedSessionState {
                service_id,
                service_tenant_id,
                processor_cancel,
                processor_handle,
                ..
            } = session;
            processor_cancel.cancel();
            let _ = processor_handle.await;

            if let Some(provider_id) = state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&service_id)
            {
                state
                    .surface_proxy_deps
                    .proxy
                    .fail_in_flight_for_provider(&provider_id);
                if let Some(tenant_id) = service_tenant_id
                    && !state.shutdown_token.is_cancelled()
                {
                    state
                        .notification
                        .event_broadcaster
                        .send(tenant_id, AdminEvent::SurfacesChanged)
                        .await;
                }
            }
            // Always call unregister_service — idempotent no-op when nothing is registered.
            // Matches the unconditional placement in cleanup_embedded_service_session and
            // cleanup_authenticated_session.
            state
                .surface_proxy_deps
                .registry
                .unregister_service(&service_id);
        }
        AuthenticatedSessionOwnership::Current | AuthenticatedSessionOwnership::Removed => {
            cleanup_authenticated_session(state, session).await;
        }
    }
}

// ---------------------------------------------------------------------------
// handle_authenticated_loop
// ---------------------------------------------------------------------------

/// Action returned by [`handle_incoming_text`] to control the main event loop.
pub(super) enum TextAction {
    /// Continue to the next iteration (message was handled inline).
    Continue,
    /// Break out of the loop.
    Break,
    /// Break out of the loop after closing the connection for rate limiting.
    RateLimitBreak,
    /// The message was forwarded to the processor; continue the loop.
    Forwarded,
}

/// Handle a deserialized text frame: fast-path messages inline, forward
/// everything else to the processor.
// All parameters originate from the main event loop and cannot be meaningfully
// grouped without duplicating the AuthenticatedSessionState struct.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters originate from caller context and cannot be meaningfully grouped without wrapper duplication"
)]
pub(super) async fn handle_incoming_text(
    text: &str,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    session: &mut AuthenticatedSessionState,
    consecutive_unknown: &mut u32,
) -> TextAction {
    let deserialized = match deserialize_service_msg(in_seq, text) {
        Ok(Some(m)) => m,
        Ok(None) => return TextAction::Continue,
        Err(e) => {
            tracing::debug!(error = %e, "deserialize error");
            return TextAction::Break;
        }
    };
    let pagination = deserialized.pagination;
    let service_msg = deserialized.message;

    // Fast-path messages handled inline.
    match &service_msg {
        ServiceMessage::Ping(PingPayload { service_ts, .. }) => {
            if messages::handle_ping(sink, out_seq, state, service_id, *service_ts, is_system)
                .await
                .is_break()
            {
                return TextAction::Break;
            }
            *consecutive_unknown = 0;
            return TextAction::Continue;
        }
        ServiceMessage::Disconnecting(payload) => {
            tracing::info!(
                %service_id,
                reason = ?payload.reason,
                "service disconnecting gracefully"
            );
            return TextAction::Break;
        }
        ServiceMessage::Unknown => {
            *consecutive_unknown += 1;
            tracing::warn!(
                %service_id,
                consecutive_unknown = *consecutive_unknown,
                "received unknown service message type; \
                 ignoring for forward compatibility"
            );
            if *consecutive_unknown >= MAX_CONSECUTIVE_UNKNOWN_MESSAGES {
                tracing::warn!(
                    %service_id,
                    "closing connection: {MAX_CONSECUTIVE_UNKNOWN_MESSAGES} \
                     consecutive unknown messages"
                );
                return TextAction::RateLimitBreak;
            }
            return TextAction::Continue;
        }
        _ => {}
    }

    // Known non-fast-path message: reset unknown counter and forward.
    *consecutive_unknown = 0;
    if session
        .msg_tx
        .send(ProcessorMessage {
            message: service_msg,
            pagination,
        })
        .await
        .is_err()
    {
        tracing::debug!("processor channel closed, breaking main loop");
        return TextAction::Break;
    }
    TextAction::Forwarded
}

/// Unified authenticated handler for all service types.
///
/// Called by [`super::service_ws`] after certificate validation, service status
/// check, and sending `ServiceSettings`. Dispatches incoming messages based on
/// the service's capability set.
///
/// Spawns a [`MessageProcessor`] task for heavy message processing. The main
/// loop handles lightweight inline operations and forwards everything else.
#[tracing::instrument(skip_all, fields(service_id = %ctx.service_id))]
pub(crate) async fn handle_authenticated_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    ctx: AuthenticatedContext<'_>,
) {
    let AuthenticatedContext {
        service_id,
        cert,
        is_system,
        out_seq,
        in_seq,
    } = ctx;

    let Some(mut session) = setup_authenticated_session(
        sink, stream, state, service_id, &cert, is_system, out_seq, in_seq,
    )
    .await
    else {
        return;
    };

    let mut consecutive_unknown: u32 = 0;

    // ------------------------------------------------------------------
    // Main operational loop
    // ------------------------------------------------------------------
    loop {
        tokio::select! {
            // 1. Incoming WebSocket messages
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
                        match handle_incoming_text(
                            &text, sink, out_seq, in_seq, state, service_id,
                            is_system, &mut session, &mut consecutive_unknown,
                        ).await {
                            TextAction::Continue | TextAction::Forwarded => {}
                            TextAction::Break => break,
                            TextAction::RateLimitBreak => {
                                let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }

            // 2. Push messages from ServiceConnectionRegistry
            push = session.push_rx.recv() => {
                let Some(msg) = push else { break };
                let Some(json) = serialize_controller_msg(out_seq, msg) else { break };
                if !send_ws_with_timeout(sink, json, service_id).await {
                    break;
                }
            }

            // 3. Responses from the background processor
            resp = session.resp_rx.recv() => {
                let Some(resp) = resp else {
                    tracing::debug!("processor response channel closed");
                    break;
                };

                // Send reply messages
                let mut write_failed = false;
                for reply in resp.replies {
                    let Some(json) = serialize_controller_msg(out_seq, reply) else {
                        write_failed = true;
                        break;
                    };
                    if !send_ws_with_timeout(sink, json, service_id).await {
                        write_failed = true;
                        break;
                    }
                }

                if write_failed {
                    break;
                }

                // Execute the action
                match resp.action {
                    ProcessorAction::Continue => {}
                    ProcessorAction::Break => break,
                    ProcessorAction::CloseWithReason(reason) => {
                        let _ = close_with_reason(sink, reason).await;
                        break;
                    }
                }
            }

            // 4. Connection superseded or force-disconnected
            _ = session.cancel_token.cancelled() => {
                tracing::info!(%service_id, "connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                handle_cancelled_authenticated_session_after_close(state, session).await;
                return;
            }
        }
    }

    // ------------------------------------------------------------------
    // Cleanup
    // ------------------------------------------------------------------
    finalize_authenticated_session(state, session).await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    use uptrakit_web_api_types::events::AdminEvent;
    use uuid::Uuid;

    #[cfg(feature = "db-sqlite")]
    mod db_sqlite {
        use super::super::super::test_support::*;
        use super::*;
        use std::collections::BTreeSet;

        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        use uptrakit_shared_db::entity::{service_host, update_history};
        use uptrakit_wire::Capability;

        use super::super::super::updates;

        #[tokio::test]
        async fn cleanup_authenticated_session_unregisters_runtime_state_even_with_stale_ui_snapshot()
         {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);

            assert_eq!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id),
                Some("provider-a".to_string())
            );

            cleanup_authenticated_session(
                &state,
                test_authenticated_session(service_id, uuid::Uuid::now_v7()),
            )
            .await;

            assert!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id)
                    .is_none(),
                "surface provider should be removed on disconnect"
            );
        }

        #[tokio::test]
        async fn cancelled_authenticated_session_cleans_runtime_state_after_force_disconnect() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let connection_id = register_test_connection(&state, service_id).await;
            state
                .service_connections
                .force_disconnect(&service_id)
                .await;

            handle_cancelled_authenticated_session_after_close(
                &state,
                test_authenticated_session(service_id, connection_id),
            )
            .await;

            assert!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id)
                    .is_none()
            );
        }

        #[tokio::test]
        async fn cancelled_authenticated_session_skips_runtime_cleanup_for_genuine_supersession() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let superseded_id = register_test_connection(&state, service_id).await;
            let _replacement_id = register_test_connection(&state, service_id).await;

            handle_cancelled_authenticated_session_after_close(
                &state,
                test_authenticated_session(service_id, superseded_id),
            )
            .await;

            assert!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id)
                    .is_none(),
                "Replaced branch now unregisters the old provider so the replacement session re-registers"
            );
            assert!(state.service_connections.is_connected(&service_id).await);
        }

        #[tokio::test]
        async fn finalized_authenticated_session_skips_runtime_cleanup_when_session_is_replaced() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let superseded_id = register_test_connection(&state, service_id).await;
            let _replacement_id = register_test_connection(&state, service_id).await;

            finalize_authenticated_session(
                &state,
                test_authenticated_session(service_id, superseded_id),
            )
            .await;

            assert!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id)
                    .is_none(),
                "Replaced branch now unregisters the old provider so the replacement session re-registers"
            );
            assert!(state.service_connections.is_connected(&service_id).await);
        }

        #[cfg(feature = "db-sqlite")]
        #[tokio::test]
        async fn finalize_replaced_session_broadcasts_surfaces_changed_when_provider_registered() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = uuid::Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let superseded_id = register_test_connection(&state, service_id).await;
            let _replacement_id = register_test_connection(&state, service_id).await;

            let mut rx = state
                .notification
                .event_broadcaster
                .subscribe(tenant_id)
                .await;

            let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
            let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
            let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
            finalize_authenticated_session(
                &state,
                AuthenticatedSessionState {
                    service_id,
                    connection_id: superseded_id,
                    is_system: false,
                    has_update_tracking: false,
                    has_software_discovery: false,
                    has_workload_claims: false,
                    service_tenant_id: Some(tenant_id),
                    linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                    push_rx,
                    cancel_token: tokio_util::sync::CancellationToken::new(),
                    msg_tx,
                    resp_rx,
                    processor_cancel: tokio_util::sync::CancellationToken::new(),
                    processor_handle: tokio::spawn(async {}),
                    rate_limiter: MessageRateLimiter::new(
                        WS_MESSAGE_RATE_WINDOW,
                        WS_MESSAGE_RATE_LIMIT,
                    ),
                },
            )
            .await;

            match rx.try_recv() {
                Ok(AdminEvent::SurfacesChanged) => {}
                other => panic!("expected SurfacesChanged from Replaced branch, got {other:?}"),
            }
            assert!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id)
                    .is_none(),
                "provider should be removed by Replaced branch"
            );
        }

        #[cfg(feature = "db-sqlite")]
        #[tokio::test]
        async fn finalize_replaced_session_skips_broadcast_when_no_provider() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = uuid::Uuid::now_v7();
            // Do NOT register a provider — this service never had UiSurfaces.
            let superseded_id = register_test_connection(&state, service_id).await;
            let _replacement_id = register_test_connection(&state, service_id).await;

            let mut rx = state
                .notification
                .event_broadcaster
                .subscribe(tenant_id)
                .await;

            let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
            let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
            let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
            finalize_authenticated_session(
                &state,
                AuthenticatedSessionState {
                    service_id,
                    connection_id: superseded_id,
                    is_system: false,
                    has_update_tracking: false,
                    has_software_discovery: false,
                    has_workload_claims: false,
                    service_tenant_id: Some(tenant_id),
                    linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                    push_rx,
                    cancel_token: tokio_util::sync::CancellationToken::new(),
                    msg_tx,
                    resp_rx,
                    processor_cancel: tokio_util::sync::CancellationToken::new(),
                    processor_handle: tokio::spawn(async {}),
                    rate_limiter: MessageRateLimiter::new(
                        WS_MESSAGE_RATE_WINDOW,
                        WS_MESSAGE_RATE_LIMIT,
                    ),
                },
            )
            .await;

            assert!(
                rx.try_recv().is_err(),
                "no broadcast when service had no surface provider"
            );
        }

        #[cfg(feature = "db-sqlite")]
        #[tokio::test]
        async fn finalize_replaced_session_skips_broadcast_when_no_tenant_id() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = uuid::Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let superseded_id = register_test_connection(&state, service_id).await;
            let _replacement_id = register_test_connection(&state, service_id).await;

            let mut rx = state
                .notification
                .event_broadcaster
                .subscribe(tenant_id)
                .await;

            let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
            let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
            let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
            finalize_authenticated_session(
                &state,
                AuthenticatedSessionState {
                    service_id,
                    connection_id: superseded_id,
                    is_system: true,
                    has_update_tracking: false,
                    has_software_discovery: false,
                    has_workload_claims: false,
                    service_tenant_id: None,
                    linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                    push_rx,
                    cancel_token: tokio_util::sync::CancellationToken::new(),
                    msg_tx,
                    resp_rx,
                    processor_cancel: tokio_util::sync::CancellationToken::new(),
                    processor_handle: tokio::spawn(async {}),
                    rate_limiter: MessageRateLimiter::new(
                        WS_MESSAGE_RATE_WINDOW,
                        WS_MESSAGE_RATE_LIMIT,
                    ),
                },
            )
            .await;

            assert!(
                rx.try_recv().is_err(),
                "no broadcast for system service (no tenant_id)"
            );
        }

        #[cfg(feature = "db-sqlite")]
        #[tokio::test]
        async fn finalize_replaced_session_cancels_in_flight_requests_for_old_provider() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = uuid::Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            // Register superseded connection (receiver dropped — connection_id needed for cleanup).
            let superseded_id = register_test_connection(&state, service_id).await;
            // Register replacement connection, keeping receiver alive so the mpsc channel
            // stays open and the proxy can dispatch the invoke without a SendFailed.
            let (_rx_replacement, _handle_replacement) = state
                .service_connections
                .register(
                    service_id,
                    BTreeSet::from([Capability::UiSurfaces]),
                    None,
                    None,
                    Some("uptrakit-agent-ssh".to_string()),
                )
                .await;

            let state_for_invoke = Arc::clone(&state);
            let invoke_task = tokio::spawn(async move {
                state_for_invoke
                    .surface_proxy_deps
                    .proxy
                    .invoke(
                        &state_for_invoke.service_connections,
                        &state_for_invoke.surface_proxy_deps.registry,
                        crate::surface_proxy::SurfaceInvokeRequest::new(
                            tenant_id,
                            "ssh.guest.panel".to_string(),
                            "refresh".to_string(),
                            "replaced-session-test".to_string(),
                            Some("provider-a".to_string()),
                            crate::surface_proxy::SurfaceCallerOrigin::UserSession {
                                user_id: uuid::Uuid::now_v7(),
                                session_id: "session-1".to_string(),
                            },
                            serde_json::Map::new(),
                            None,
                        ),
                        Some(std::time::Duration::from_secs(30)),
                    )
                    .await
            });
            tokio::task::yield_now().await;

            let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
            let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
            let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
            finalize_authenticated_session(
                &state,
                AuthenticatedSessionState {
                    service_id,
                    connection_id: superseded_id,
                    is_system: false,
                    has_update_tracking: false,
                    has_software_discovery: false,
                    has_workload_claims: false,
                    service_tenant_id: Some(tenant_id),
                    linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                    push_rx,
                    cancel_token: tokio_util::sync::CancellationToken::new(),
                    msg_tx,
                    resp_rx,
                    processor_cancel: tokio_util::sync::CancellationToken::new(),
                    processor_handle: tokio::spawn(async {}),
                    rate_limiter: MessageRateLimiter::new(
                        WS_MESSAGE_RATE_WINDOW,
                        WS_MESSAGE_RATE_LIMIT,
                    ),
                },
            )
            .await;

            let invoke_result =
                tokio::time::timeout(std::time::Duration::from_secs(1), invoke_task)
                    .await
                    .expect("in-flight invoke should resolve after fail_in_flight_for_provider")
                    .expect("invoke task should join");
            assert!(
                matches!(
                    invoke_result,
                    Err(crate::surface_proxy::SurfaceProxyError::ServiceDisconnected)
                ),
                "fail_in_flight_for_provider should have cancelled in-flight invoke: {invoke_result:?}"
            );
        }

        #[cfg(feature = "db-sqlite")]
        #[tokio::test]
        async fn authenticated_cleanup_stale_connection_does_not_evict_replacement() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = uuid::Uuid::now_v7();

            // A registers, then B supersedes A (reconnect).
            let superseded_id = register_test_connection(&state, service_id).await;
            let replacement_id = register_test_connection(&state, service_id).await;
            assert_ne!(superseded_id, replacement_id);

            // A's session runs its finalize/cleanup after B is live.
            let session = test_authenticated_session(service_id, superseded_id);
            finalize_authenticated_session(&state, session).await;

            // B must still be registered — the stale cleanup must not evict it.
            assert!(
                state.service_connections.is_connected(&service_id).await,
                "replacement connection B must survive A's stale cleanup"
            );
            assert_eq!(
                state
                    .service_connections
                    .current_connection_id(&service_id)
                    .await,
                Some(replacement_id),
            );
        }

        #[cfg(feature = "db-sqlite")]
        #[tokio::test]
        async fn cleanup_authenticated_session_stale_id_does_not_evict_replacement() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = uuid::Uuid::now_v7();

            // A registers (capture its id), then B supersedes A.
            let superseded_id = register_test_connection(&state, service_id).await;
            let replacement_id = register_test_connection(&state, service_id).await;

            // Drive cleanup for A's session directly, with A's now-stale id. No tenant /
            // workload claims — isolate the registry-removal branch.
            let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
            let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
            let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
            cleanup_authenticated_session(
                &state,
                AuthenticatedSessionState {
                    service_id,
                    connection_id: superseded_id,
                    is_system: false,
                    has_update_tracking: false,
                    has_software_discovery: false,
                    has_workload_claims: false,
                    service_tenant_id: None,
                    linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                    push_rx,
                    cancel_token: tokio_util::sync::CancellationToken::new(),
                    msg_tx,
                    resp_rx,
                    processor_cancel: tokio_util::sync::CancellationToken::new(),
                    processor_handle: tokio::spawn(async {}),
                    rate_limiter: MessageRateLimiter::new(
                        WS_MESSAGE_RATE_WINDOW,
                        WS_MESSAGE_RATE_LIMIT,
                    ),
                },
            )
            .await;

            // unregister_current(A) returns false because B is current → B survives.
            assert!(state.service_connections.is_connected(&service_id).await);
            assert_eq!(
                state
                    .service_connections
                    .current_connection_id(&service_id)
                    .await,
                Some(replacement_id),
            );
        }

        #[tokio::test]
        async fn reconnect_cleanup_same_instance_leaves_owned_update_in_progress() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = Uuid::now_v7();
            let runtime_id = Uuid::now_v7();
            let capabilities: BTreeSet<Capability> =
                [Capability::SoftwareDiscovery, Capability::UpdateHooks]
                    .into_iter()
                    .collect();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, service_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                service_id,
                Some(runtime_id),
            )
            .await;

            run_embedded_register_once(
                Arc::clone(&state),
                service_id,
                tenant_id,
                capabilities,
                runtime_id,
            )
            .await;

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        }

        #[tokio::test]
        async fn reconnect_cleanup_new_instance_fails_prior_owned_update_even_without_host_links() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let service_id = Uuid::now_v7();
            let old_runtime_id = Uuid::now_v7();
            let new_runtime_id = Uuid::now_v7();
            let capabilities: BTreeSet<Capability> =
                [Capability::SoftwareDiscovery, Capability::UpdateHooks]
                    .into_iter()
                    .collect();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, service_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                service_id,
                Some(old_runtime_id),
            )
            .await;

            service_host::Entity::delete_many()
                .filter(service_host::Column::ServiceId.eq(service_id))
                .exec(state.db())
                .await
                .unwrap();

            run_embedded_register_once(
                Arc::clone(&state),
                service_id,
                tenant_id,
                capabilities,
                new_runtime_id,
            )
            .await;

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::Interrupted);
            assert_eq!(
                row.output,
                "Update interrupted: agent restarted (outcome unknown)"
            );
        }

        #[tokio::test]
        async fn connect_phase_does_not_fail_update_owned_by_different_linked_service() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let owner_service_id = Uuid::now_v7();
            let reconnecting_service_id = Uuid::now_v7();
            let old_runtime_id = Uuid::now_v7();
            let new_runtime_id = Uuid::now_v7();
            let (host_id, software_item_id) =
                insert_linked_host_and_item(state.db(), tenant_id, owner_service_id).await;
            insert_service_row(
                state.db(),
                tenant_id,
                reconnecting_service_id,
                "uptrakit-agent",
            )
            .await;
            relink_service_host(state.db(), reconnecting_service_id, host_id).await;
            let update_history_id = insert_owned_in_progress_update(
                state.db(),
                tenant_id,
                host_id,
                software_item_id,
                owner_service_id,
                Some(old_runtime_id),
            )
            .await;

            updates::recover_owned_updates_on_connect_with_dispatch_mode(
                &state,
                reconnecting_service_id,
                Some(new_runtime_id),
                updates::ReconnectSuccessorDispatchMode::Immediate,
            )
            .await
            .unwrap();
            let _ = updates::load_pending_update_records(&state, reconnecting_service_id)
                .await
                .unwrap();

            let row = update_history::Entity::find_by_id(update_history_id)
                .one(state.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        }

        #[tokio::test]
        async fn rotating_surface_provider_id_fails_old_provider_in_flight_requests() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            state
                .surface_proxy_deps
                .registry
                .register_service(
                    service_id,
                    "uptrakit-agent-ssh",
                    Some(tenant_id),
                    test_surface_registration("provider-a", tenant_id),
                )
                .expect("provider-a registration should succeed");

            let (_rx, _cancel) = state
                .service_connections
                .register(
                    service_id,
                    BTreeSet::from([Capability::UiSurfaces]),
                    None,
                    None,
                    Some("uptrakit-agent-ssh".to_string()),
                )
                .await;

            let state_for_invoke = Arc::clone(&state);
            let invoke_task = tokio::spawn(async move {
                state_for_invoke
                    .surface_proxy_deps
                    .proxy
                    .invoke(
                        &state_for_invoke.service_connections,
                        &state_for_invoke.surface_proxy_deps.registry,
                        crate::surface_proxy::SurfaceInvokeRequest::new(
                            tenant_id,
                            "ssh.guest.panel".to_string(),
                            "refresh".to_string(),
                            "rotate-provider".to_string(),
                            Some("provider-a".to_string()),
                            crate::surface_proxy::SurfaceCallerOrigin::UserSession {
                                user_id: Uuid::now_v7(),
                                session_id: "session-1".to_string(),
                            },
                            serde_json::Map::new(),
                            None,
                        ),
                        Some(std::time::Duration::from_secs(30)),
                    )
                    .await
            });

            tokio::task::yield_now().await;

            let processor = MessageProcessor {
                state: Arc::clone(&state),
                service_id,
                cert: None,
                is_system: false,
                has_update_tracking: false,
                has_software_discovery: false,
                has_update_hooks: false,
                has_ui_surfaces: true,
                has_workload_claims: false,
                runtime_instance_id: None,
                service_app_name: Some("uptrakit-agent-ssh".to_string()),
                service_tenant_id: Some(tenant_id),
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                report_tracker: ReportTracker::new(),
            };

            let response = processor
                .handle_surface_registration(test_surface_registration("provider-b", tenant_id))
                .await;
            assert!(response.replies.is_empty());
            assert!(matches!(response.action, ProcessorAction::Continue));

            let invoke_result =
                tokio::time::timeout(std::time::Duration::from_secs(1), invoke_task)
                    .await
                    .expect("old-provider invoke should complete promptly after provider rotation")
                    .expect("invoke task should join");
            assert!(matches!(
                invoke_result,
                Err(crate::surface_proxy::SurfaceProxyError::ServiceDisconnected)
            ));
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_authenticated_session_broadcasts_surfaces_changed_when_tenant_present() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);
        let connection_id = register_test_connection(&state, service_id).await;

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        cleanup_authenticated_session(
            &state,
            AuthenticatedSessionState {
                service_id,
                connection_id,
                is_system: false,
                has_update_tracking: false,
                has_software_discovery: false,
                has_workload_claims: false,
                service_tenant_id: Some(tenant_id),
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                push_rx,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                msg_tx,
                resp_rx,
                processor_cancel: tokio_util::sync::CancellationToken::new(),
                processor_handle: tokio::spawn(async {}),
                rate_limiter: MessageRateLimiter::new(
                    WS_MESSAGE_RATE_WINDOW,
                    WS_MESSAGE_RATE_LIMIT,
                ),
            },
        )
        .await;

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged, got {other:?}"),
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_authenticated_session_skips_broadcast_when_no_tenant_id() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);
        let connection_id = register_test_connection(&state, service_id).await;

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        cleanup_authenticated_session(
            &state,
            AuthenticatedSessionState {
                service_id,
                connection_id,
                is_system: true,
                has_update_tracking: false,
                has_software_discovery: false,
                has_workload_claims: false,
                service_tenant_id: None,
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                push_rx,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                msg_tx,
                resp_rx,
                processor_cancel: tokio_util::sync::CancellationToken::new(),
                processor_handle: tokio::spawn(async {}),
                rate_limiter: MessageRateLimiter::new(
                    WS_MESSAGE_RATE_WINDOW,
                    WS_MESSAGE_RATE_LIMIT,
                ),
            },
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no broadcast expected for system service"
        );
    }
}
