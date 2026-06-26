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
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget sends in WS handler intentionally drop results"
)]
#![expect(
    clippy::map_err_ignore,
    reason = "original WS parse errors carry no useful context"
)]

mod audit_service;
mod audit_surface;
mod cert;
mod credentials;
mod discovery;
mod message_processor;
pub(super) mod messages;
mod reconnect;
mod renewal;
mod service_config;
mod shared_types;
mod surface_wire;
#[cfg(test)]
pub(super) mod test_support;
mod update_tracking;
mod updates;
mod workload;

use cert::{
    ApprovalPollResult, CertificateResult, handle_request_certificate, poll_approval_status,
};
use credentials::deliver_service_credentials;
pub(crate) use discovery::trigger_discovery_for_agent_host;
use message_processor::{MessageProcessor, ProcessorMessage, spawn_message_processor};
use shared_types::{ProcessorAction, ProcessorResponse, load_linked_host_ids};
pub(crate) use updates::dispatch_next_batch_update;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use sea_orm::EntityTrait;

use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::{
    Capability, CloseReason, ControllerMessage, ErrorCode, ErrorPayload, HostConnectivityUpdate,
    IncomingSeq, OutgoingSeq, PingPayload, RegisterPayload, ServiceMessage,
};

use super::protocol::{
    AuthenticatedContext, CertIdentity, MessageRateLimiter, WS_MESSAGE_RATE_LIMIT,
    WS_MESSAGE_RATE_WINDOW, close_with_reason, deserialize_service_msg, record_service_activity,
    record_system_service_activity, send_pong, serialize_controller_msg,
};
use crate::AppState;
use uptrakit_web_api_types::events::AdminEvent;
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

/// Send a serialized WebSocket message with a timeout, returning `true`
/// on success and `false` if the write failed or timed out.
async fn send_ws_with_timeout(
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
struct AuthenticatedSessionState {
    service_id: uuid::Uuid,
    connected_at: time::OffsetDateTime,
    is_system: bool,
    has_update_tracking: bool,
    has_software_discovery: bool,
    has_workload_claims: bool,
    service_tenant_id: Option<uuid::Uuid>,
    linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    push_rx: tokio::sync::mpsc::Receiver<ControllerMessage>,
    cancel_token: tokio_util::sync::CancellationToken,
    msg_tx: tokio::sync::mpsc::Sender<ProcessorMessage>,
    resp_rx: tokio::sync::mpsc::Receiver<ProcessorResponse>,
    processor_cancel: tokio_util::sync::CancellationToken,
    processor_handle: tokio::task::JoinHandle<()>,
    rate_limiter: MessageRateLimiter,
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
async fn load_service_capabilities(
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
                system_service_tenant_binding(
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
/// Returns `(push_rx, cancel_token, connected_at)`.
fn cancellation_token_from_connection_handle(
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

async fn register_connection(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    capabilities: &BTreeSet<Capability>,
    service_app_name: Option<String>,
) -> (
    tokio::sync::mpsc::Receiver<ControllerMessage>,
    tokio_util::sync::CancellationToken,
    time::OffsetDateTime,
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

    let connected_at = state
        .service_connections
        .connected_at(&service_id)
        .await
        .expect("connected service should have a registered timestamp");

    let cancel_token = cancellation_token_from_connection_handle(connection_handle);

    (push_rx, cancel_token, connected_at)
}

/// Stage 4: Load linked host IDs shared between the main loop and the processor.
async fn load_session_host_ids(
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
async fn prepare_reconnect_updates_on_connect(
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
// Embedded service message handler
// ---------------------------------------------------------------------------

/// Run a message handler loop for an embedded service.
///
/// This creates a [`MessageProcessor`] configured for an embedded (in-process)
/// service and reads messages from the provided channel. Replies are pushed
/// back through the [`ServiceConnectionRegistry`].
///
/// Used by `embedded_support::run_embedded_message_handler`.
pub(crate) async fn run_embedded_message_handler(
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    capabilities: &BTreeSet<Capability>,
    app_name: &str,
    service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    run_embedded_message_handler_inner(
        state,
        EmbeddedHandlerSession {
            service_id,
            is_system: false,
            service_tenant_id: Some(tenant_id),
            app_name,
        },
        capabilities,
        service_rx,
        cancel,
    )
    .await;
}

pub(crate) async fn run_embedded_system_message_handler(
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    service_tenant_id: Option<uuid::Uuid>,
    capabilities: &BTreeSet<Capability>,
    app_name: &str,
    service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    run_embedded_message_handler_inner(
        state,
        EmbeddedHandlerSession {
            service_id,
            is_system: true,
            service_tenant_id,
            app_name,
        },
        capabilities,
        service_rx,
        cancel,
    )
    .await;
}

struct EmbeddedHandlerSession<'a> {
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    app_name: &'a str,
}

async fn run_embedded_message_handler_inner(
    state: Arc<AppState>,
    session: EmbeddedHandlerSession<'_>,
    capabilities: &BTreeSet<Capability>,
    mut service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let has_software_discovery = capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = capabilities.contains(&Capability::UpdateHooks);
    let has_ui_surfaces = capabilities.contains(&Capability::UiSurfaces);
    let has_workload_claims = capabilities.contains(&Capability::WorkloadClaims);
    let has_update_tracking = capabilities.contains(&Capability::UpdateTracking);

    let linked_host_ids =
        load_session_host_ids(&state, session.service_id, has_software_discovery).await;

    let mut processor = MessageProcessor {
        state: Arc::clone(&state),
        service_id: session.service_id,
        cert: None,
        is_system: session.is_system,
        has_update_tracking,
        has_software_discovery,
        has_update_hooks,
        has_ui_surfaces,
        has_workload_claims,
        runtime_instance_id: None,
        service_app_name: Some(session.app_name.to_string()),
        service_tenant_id: session.service_tenant_id,
        linked_host_ids,
        report_tracker: ReportTracker::new(),
    };

    'msg_loop: loop {
        let msg = tokio::select! {
            biased;
            () = cancel.cancelled() => break 'msg_loop,
            msg = service_rx.recv() => match msg {
                Some(m) => m,
                None => break 'msg_loop,
            },
        };

        // dispatch and reply-send are wrapped in separate cancellable selects so
        // that drain/abort can interrupt even when a SeaORM query or a channel
        // send is in progress. Dropping dispatch mid-flight cancels any in-flight
        // DB query; the connection is returned to the pool and the transaction
        // rolled back. cleanup_embedded_service_session handles workload release.
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => break 'msg_loop,
            r = processor.dispatch(msg, None) => r,
        };

        for reply in response.replies {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break 'msg_loop,
                _ = state.service_connections.send(&session.service_id, reply) => {}
            }
        }

        match response.action {
            ProcessorAction::Continue => {}
            ProcessorAction::Break | ProcessorAction::CloseWithReason(_) => {
                tracing::info!(
                    service_id = %session.service_id,
                    app_name = session.app_name,
                    "embedded message handler stopping (processor requested break)"
                );
                break 'msg_loop;
            }
        }
    }

    cleanup_embedded_service_session(
        &state,
        session.service_id,
        session.app_name,
        has_workload_claims,
        session.service_tenant_id,
    )
    .await;

    tracing::debug!(
        service_id = %session.service_id,
        app_name = session.app_name,
        "embedded message handler exited"
    );
}

async fn cleanup_embedded_service_session(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    _service_app_name: &str,
    has_workload_claims: bool,
    tenant_id: Option<uuid::Uuid>,
) {
    if has_workload_claims {
        workload::release_all_claims_on_disconnect(state, service_id).await;
    }

    if let Some(provider_id) = state
        .surface_proxy_deps
        .registry
        .provider_id_for_service(&service_id)
    {
        state
            .surface_proxy_deps
            .proxy
            .fail_in_flight_for_provider(&provider_id);
        if let Some(tid) = tenant_id
            && !state.shutdown_token.is_cancelled()
        {
            state
                .notification
                .event_broadcaster
                .send(tid, AdminEvent::SurfacesChanged)
                .await;
        }
    }
    state
        .surface_proxy_deps
        .registry
        .unregister_service(&service_id);

    state.service_connections.unregister(&service_id).await;

    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_disconnected(&service_id);
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
async fn receive_register_message(
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
async fn setup_authenticated_session(
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
    deliver_service_credentials(
        sink,
        state,
        &db_capabilities,
        credentials::ServiceCredentialTarget {
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
    let (push_rx, cancel_token, connected_at) = register_connection(
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
        connected_at,
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
async fn cleanup_authenticated_session(state: &Arc<AppState>, session: AuthenticatedSessionState) {
    let AuthenticatedSessionState {
        service_id,
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

    state.service_connections.unregister(&service_id).await;

    // Notify embedded services about the disconnection.
    if let Some(ref notifier) = state.embedded_service_notifier {
        notifier.on_external_disconnected(&service_id);
    }

    tracing::debug!(%service_id, "authenticated service disconnected");
}

async fn handle_cancelled_authenticated_session_after_close(
    state: &Arc<AppState>,
    session: AuthenticatedSessionState,
) {
    finalize_authenticated_session(state, session).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthenticatedSessionOwnership {
    Current,
    Replaced,
    Removed,
}

async fn authenticated_session_ownership(
    state: &Arc<AppState>,
    session: &AuthenticatedSessionState,
) -> AuthenticatedSessionOwnership {
    match state
        .service_connections
        .connected_at(&session.service_id)
        .await
    {
        Some(connected_at) if connected_at == session.connected_at => {
            AuthenticatedSessionOwnership::Current
        }
        Some(_) => AuthenticatedSessionOwnership::Replaced,
        None => AuthenticatedSessionOwnership::Removed,
    }
}

async fn finalize_authenticated_session(state: &Arc<AppState>, session: AuthenticatedSessionState) {
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
enum TextAction {
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
async fn handle_incoming_text(
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
    let cancel_token = cancellation_token_from_connection_handle(connection_handle);

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

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    use std::sync::Arc;
    use uptrakit_wire::limits::MAX_SHORT_STRING_LEN;

    use uptrakit_wire::surfaces;
    use uuid::Uuid;

    #[test]
    fn system_service_tenant_binding_only_targets_mqtt() {
        let tenant_id = uuid::Uuid::now_v7();
        assert_eq!(
            system_service_tenant_binding(Some("uptrakit-mqtt"), tenant_id),
            Some(tenant_id)
        );
        assert_eq!(
            system_service_tenant_binding(Some("uptrakit-scheduler"), tenant_id),
            None
        );
        assert_eq!(system_service_tenant_binding(None, tenant_id), None);
    }

    #[test]
    fn service_config_scope_validation_requires_exact_tenant_for_bound_sessions() {
        let tenant_id = uuid::Uuid::now_v7();
        assert!(is_valid_service_config_scope(
            Some(tenant_id),
            Some(tenant_id)
        ));
        assert!(!is_valid_service_config_scope(Some(tenant_id), None));
        assert!(!is_valid_service_config_scope(
            Some(tenant_id),
            Some(uuid::Uuid::now_v7())
        ));
        assert!(is_valid_service_config_scope(None, None));
        assert!(is_valid_service_config_scope(None, Some(tenant_id)));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn store_service_config_scope_violation_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-mqtt".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::StoreServiceConfig(uptrakit_wire::StoreServiceConfigPayload::new(
                    "req-store-denied".to_string(),
                    None,
                    "clients.primary".to_string(),
                    serde_json::json!({"enabled": true}),
                    true,
                )),
                None,
            )
            .await;

        let [ControllerMessage::ServiceConfigAck(ack)] = response.replies.as_slice() else {
            panic!("expected exactly one ServiceConfigAck reply");
        };
        assert_eq!(ack.request_id, "req-store-denied");
        assert!(!ack.success);
        assert_eq!(
            ack.error.as_deref(),
            Some("service cannot write config outside its tenant binding")
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_STORE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.request_id.as_deref(), Some("req-store-denied"));
        assert_eq!(row.target_type.as_deref(), Some("service_config"));
        assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
        let details = row
            .details_json
            .as_ref()
            .expect("scope denial audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["requested_scope"], "global");
        assert_eq!(details["service_tenant_id"], tenant_id.to_string());
        assert_eq!(details["requested_tenant_id"], serde_json::Value::Null);
        assert_eq!(details["reason_code"], "outside_tenant_binding");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn delete_service_config_scope_violation_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        let requested_tenant_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-mqtt".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::DeleteServiceConfig(
                    uptrakit_wire::DeleteServiceConfigPayload::new(
                        "req-delete-denied".to_string(),
                        Some(requested_tenant_id),
                        "clients.primary".to_string(),
                    ),
                ),
                None,
            )
            .await;

        let [ControllerMessage::ServiceConfigAck(ack)] = response.replies.as_slice() else {
            panic!("expected exactly one ServiceConfigAck reply");
        };
        assert_eq!(ack.request_id, "req-delete-denied");
        assert!(!ack.success);
        assert_eq!(
            ack.error.as_deref(),
            Some("service cannot delete config outside its tenant binding")
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.request_id.as_deref(), Some("req-delete-denied"));
        assert_eq!(row.target_type.as_deref(), Some("service_config"));
        assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
        let details = row
            .details_json
            .as_ref()
            .expect("scope denial audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["requested_scope"], "tenant");
        assert_eq!(details["service_tenant_id"], tenant_id.to_string());
        assert_eq!(
            details["requested_tenant_id"],
            requested_tenant_id.to_string()
        );
        assert_eq!(details["reason_code"], "outside_tenant_binding");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_scope_violation_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let requested_tenant_id = Uuid::now_v7();
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-mqtt".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: requested_tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("notifications.email").unwrap(),
                interaction_id: surfaces::InteractionId::new("configure_smtp").unwrap(),
                idempotency_key: "scope-violation".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::from_iter([(
                    "host".to_string(),
                    serde_json::Value::String("smtp.example.invalid".to_string()),
                )]),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(
            error.code,
            surfaces::SurfaceActionErrorCode::PermissionDenied
        );
        assert_eq!(
            error.message,
            "service cannot invoke actions outside its tenant"
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(row.target_id, None);
        assert_eq!(
            row.target_display.as_deref(),
            Some("notifications.email/configure_smtp")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("scope denial audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["surface_id"], "notifications.email");
        assert_eq!(details["interaction_id"], "configure_smtp");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["service_tenant_id"], tenant_id.to_string());
        assert_eq!(
            details["requested_tenant_id"],
            requested_tenant_id.to_string()
        );
        assert_eq!(details["reason_code"], "outside_tenant_binding");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_invalid_payload_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
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
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "x".repeat(MAX_SHORT_STRING_LEN + 1),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(
            row.target_display.as_deref(),
            Some("ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("invalid payload audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["reason_code"], "invalid_request");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_invalid_tenant_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
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
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: "not-a-uuid".to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "invalid-tenant".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        let details = row
            .details_json
            .as_ref()
            .expect("invalid tenant audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["reason_code"], "invalid_tenant_id");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_lookup_failure_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.surfaces[0].interactions[0].required_permission = None;
        state
            .surface_proxy_deps
            .registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("surface registration should succeed");
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
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "lookup-failure".to_string(),
                target_provider_id: Some("missing-provider".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(error.code, surfaces::SurfaceActionErrorCode::InvalidRequest);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        let details = row
            .details_json
            .as_ref()
            .expect("lookup failure audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "missing-provider");
        assert_eq!(details["reason_code"], "invalid_provider");
        assert!(details.get("provider_kind").is_none());
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_success_emits_success_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.surfaces[0].interactions[0].required_permission = None;
        state
            .surface_proxy_deps
            .registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("surface registration should succeed");
        let (mut rx, _cancel) = state
            .service_connections
            .register(
                service_id,
                BTreeSet::from([Capability::UiSurfaces]),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        let proxy = Arc::clone(&state.surface_proxy_deps.proxy);
        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
                proxy.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"ok": true})),
                        error: None,
                    },
                );
            }
        });
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
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "surface-success".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(reply.success);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(
            row.target_display.as_deref(),
            Some("ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("success audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
        assert!(details.get("reason_code").is_none());
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_action_provider_unavailable_emits_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.surfaces[0].interactions[0].required_permission = None;
        state
            .surface_proxy_deps
            .registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                registration,
            )
            .expect("surface registration should succeed");
        let (rx, _cancel) = state
            .service_connections
            .register(
                service_id,
                BTreeSet::from([Capability::UiSurfaces]),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        drop(rx);
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
        let request_id = Uuid::now_v7();

        let response = processor
            .handle_surface_action_request(surfaces::SurfaceActionRequest {
                request_id,
                tenant_id: tenant_id.to_string(),
                surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                idempotency_key: "surface-provider-unavailable".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: surfaces::CallerOrigin::Provider {
                    provider_id: "provider-a".to_string(),
                },
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
            })
            .await;

        let [ControllerMessage::SurfaceActionResponse(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one SurfaceActionResponse reply");
        };
        assert_eq!(reply.request_id, request_id);
        assert!(!reply.success);
        let error = reply.error.as_ref().expect("error payload should exist");
        assert_eq!(
            error.code,
            surfaces::SurfaceActionErrorCode::ProviderUnavailable
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let request_id_string = request_id.to_string();
        assert_eq!(row.request_id.as_deref(), Some(request_id_string.as_str()));
        let details = row
            .details_json
            .as_ref()
            .expect("failed audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
        assert_eq!(details["reason_code"], "provider_unavailable");
        assert!(details.get("params").is_none());
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

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invalid_surface_registration_emits_validation_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
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
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.effective_tenant_binding.tenant_id = None;

        let response = processor.handle_surface_registration(registration).await;

        let [ControllerMessage::Error(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one Error reply");
        };
        assert_eq!(reply.code, ErrorCode::BadRequest);
        assert!(reply.message.contains("invalid surface registration"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
        assert_eq!(row.target_id.as_deref(), Some("provider-a"));
        assert_eq!(row.target_display.as_deref(), Some("provider-a"));
        let details = row
            .details_json
            .as_ref()
            .expect("validation failure audit should include details");
        assert_eq!(details["provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["framework_generation"], "1.0");
        assert_eq!(details["capability_count"], 4);
        assert_eq!(details["surface_count"], 1);
        assert_eq!(details["reason_code"], "invalid_tenant_binding");
        assert!(details.get("surfaces").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn incompatible_surface_registration_emits_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
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
        let mut registration = test_surface_registration("provider-a", tenant_id);
        registration.framework_generation = surfaces::FrameworkGeneration::new(2, 0);

        let response = processor.handle_surface_registration(registration).await;

        let [ControllerMessage::Error(reply)] = response.replies.as_slice() else {
            panic!("expected exactly one Error reply");
        };
        assert_eq!(reply.code, ErrorCode::BadRequest);
        assert!(reply.message.contains("UnsupportedGeneration"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-agent-ssh"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
        assert_eq!(row.target_id.as_deref(), Some("provider-a"));
        assert_eq!(row.target_display.as_deref(), Some("provider-a"));
        let details = row
            .details_json
            .as_ref()
            .expect("rejection audit should include details");
        assert_eq!(details["provider_id"], "provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["framework_generation"], "2.0");
        assert_eq!(details["capability_count"], 4);
        assert_eq!(details["surface_count"], 1);
        assert_eq!(details["reason_code"], "unsupported_generation");
        assert!(details.get("surfaces").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn successful_system_surface_registration_emits_success_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: true,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-scheduler".to_string()),
            service_tenant_id: None,
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let mut registration = test_surface_registration("provider-system", tenant_id);
        registration.effective_tenant_binding.scope = surfaces::Scope::Global;
        registration.effective_tenant_binding.tenant_id = None;

        let response = processor.handle_surface_registration(registration).await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_PROVIDER_REGISTER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-scheduler"));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_provider"));
        assert_eq!(row.target_id.as_deref(), Some("provider-system"));
        assert_eq!(row.target_display.as_deref(), Some("provider-system"));
        let details = row
            .details_json
            .as_ref()
            .expect("success audit should include details");
        assert_eq!(details["provider_id"], "provider-system");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["framework_generation"], "1.0");
        assert_eq!(details["capability_count"], 4);
        assert_eq!(details["surface_count"], 1);
        assert!(details.get("reason_code").is_none());
        assert!(details.get("surfaces").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    mod db_sqlite {
        use super::super::test_support::*;
        use super::*;
        use std::collections::BTreeMap;

        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        use tokio_util::sync::CancellationToken;
        use uptrakit_shared_db::entity::{service_host, update_history};

        #[tokio::test]
        async fn embedded_system_handler_cleanup_releases_claims_and_unregisters_state() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
            let notifier = Arc::new(MockEmbeddedNotifier::default());
            let state = Arc::new(AppState {
                embedded_service_notifier: Some(notifier.clone()),
                ..(*state).clone()
            });

            let service_id = Uuid::now_v7();
            let mqtt_capabilities: BTreeSet<Capability> = [
                Capability::SystemService,
                Capability::UiSurfaces,
                Capability::WorkloadClaims,
            ]
            .into_iter()
            .collect();
            let _ = state
                .service_connections
                .register(
                    service_id,
                    mqtt_capabilities.clone(),
                    None,
                    None,
                    Some("uptrakit-mqtt".to_string()),
                )
                .await;

            state
                .surface_proxy_deps
                .registry
                .register_service(
                    service_id,
                    "uptrakit-mqtt",
                    Some(tenant_id),
                    test_surface_registration("provider-mqtt", tenant_id),
                )
                .expect("service surface registration should succeed");

            let claim_key = format!("clients.{}", Uuid::now_v7());
            let claim_result = state.workload_claim_registry.try_claim(
                service_id,
                state.controller_id,
                BTreeMap::from([(claim_key.clone(), tenant_id)]),
            );
            assert!(claim_result.granted.contains(&claim_key));
            assert!(state.service_connections.is_connected(&service_id).await);
            assert_eq!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id),
                Some("provider-mqtt".to_string())
            );

            let (service_tx, service_rx) = tokio::sync::mpsc::channel(1);
            drop(service_tx);

            run_embedded_system_message_handler(
                state.clone(),
                service_id,
                Some(tenant_id),
                &mqtt_capabilities,
                "uptrakit-mqtt",
                service_rx,
                CancellationToken::new(),
            )
            .await;

            assert!(!state.service_connections.is_connected(&service_id).await);
            assert!(
                state
                    .surface_proxy_deps
                    .registry
                    .provider_id_for_service(&service_id)
                    .is_none()
            );
            assert!(
                state
                    .workload_claim_registry
                    .service_claims(service_id)
                    .is_empty()
            );
            assert_eq!(*notifier.disconnected.lock(), vec![service_id]);
        }

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
                test_authenticated_session(service_id, time::OffsetDateTime::now_utc()),
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
        async fn cancelled_authenticated_session_cleans_runtime_state_after_force_disconnect() {
            let db = crate::test_harness::setup_migrated_db().await;
            let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
            let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

            let service_id = Uuid::now_v7();
            register_test_runtime_state(&state, service_id, tenant_id);
            let connected_at = register_test_connection(&state, service_id).await;
            state
                .service_connections
                .force_disconnect(&service_id)
                .await;

            handle_cancelled_authenticated_session_after_close(
                &state,
                test_authenticated_session(service_id, connected_at),
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
            let superseded_connected_at = register_test_connection(&state, service_id).await;
            let _replacement_connected_at = register_test_connection(&state, service_id).await;

            handle_cancelled_authenticated_session_after_close(
                &state,
                test_authenticated_session(service_id, superseded_connected_at),
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
            let superseded_connected_at = register_test_connection(&state, service_id).await;
            let _replacement_connected_at = register_test_connection(&state, service_id).await;

            finalize_authenticated_session(
                &state,
                test_authenticated_session(service_id, superseded_connected_at),
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
            let superseded_at = register_test_connection(&state, service_id).await;
            let _replacement_at = register_test_connection(&state, service_id).await;

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
                    connected_at: superseded_at,
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
            let superseded_at = register_test_connection(&state, service_id).await;
            let _replacement_at = register_test_connection(&state, service_id).await;

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
                    connected_at: superseded_at,
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
            let superseded_at = register_test_connection(&state, service_id).await;
            let _replacement_at = register_test_connection(&state, service_id).await;

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
                    connected_at: superseded_at,
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
            // Register superseded connection (receiver dropped — only timestamp needed).
            let superseded_at = register_test_connection(&state, service_id).await;
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
                    connected_at: superseded_at,
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
    async fn cleanup_embedded_session_broadcasts_surfaces_changed_when_tenant_present() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        cleanup_embedded_service_session(
            &state,
            service_id,
            "uptrakit-agent-ssh",
            false,
            Some(tenant_id),
        )
        .await;

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged, got {other:?}"),
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_embedded_session_skips_broadcast_when_no_tenant_id() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        cleanup_embedded_service_session(
            &state,
            service_id,
            "uptrakit-agent-ssh",
            false,
            None, // system service — no tenant
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no broadcast expected for system service"
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_authenticated_session_broadcasts_surfaces_changed_when_tenant_present() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);
        let connected_at = register_test_connection(&state, service_id).await;

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
                connected_at,
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
        let connected_at = register_test_connection(&state, service_id).await;

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
                connected_at,
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

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_registration_success_broadcasts_surfaces_changed() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

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
            .handle_surface_registration(test_surface_registration("provider-a", tenant_id))
            .await;

        assert!(response.replies.is_empty(), "success path returns cont()");

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged on success, got {other:?}"),
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_registration_rejection_does_not_broadcast() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        let service_id_b = uuid::Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        insert_test_service_row(&db, tenant_id, service_id_b, "uptrakit-agent-ssh-2").await;

        // Register provider-a from service_id (succeeds).
        state
            .surface_proxy_deps
            .registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                test_surface_registration("provider-a", tenant_id),
            )
            .expect("first registration should succeed");

        let mut rx = state
            .notification
            .event_broadcaster
            .subscribe(tenant_id)
            .await;

        // Try to claim the SAME provider ID ("provider-a") from a different service
        // (service_id_b). The registry rejects this because provider-a is already bound
        // to service_id — two different services cannot share the same provider ID.
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id: service_id_b,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh-2".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let response = processor
            .handle_surface_registration(test_surface_registration("provider-a", tenant_id))
            .await;

        assert!(
            !response.replies.is_empty(),
            "rejection path returns an error reply"
        );
        assert!(
            rx.try_recv().is_err(),
            "no broadcast expected on rejected registration"
        );
    }
}
