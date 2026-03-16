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

mod cert;
mod credentials;
mod discovery;
pub(super) mod messages;
mod renewal;
mod service_config;
mod shared_types;
mod update_tracking;
mod updates;

use cert::{
    ApprovalPollResult, CertificateResult, handle_request_certificate, poll_approval_status,
};
use credentials::deliver_service_credentials;
pub(crate) use discovery::trigger_discovery_for_agent_host;
use shared_types::{ProcessorAction, ProcessorResponse, load_linked_host_ids};
use updates::deliver_pending_updates;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use thiserror::Error;

use rootcause::prelude::*;
use sea_orm::EntityTrait;

use uptrakit_internal_wire::limits::WireValidate;
use uptrakit_internal_wire::report_tracker::ReportTracker;
use uptrakit_internal_wire::{
    Capability, CloseReason, ControllerMessage, ErrorCode, ErrorPayload, HostConnectivityUpdate,
    IncomingSeq, OutgoingSeq, PingPayload, RegisterPayload, ReportPagination, ServiceMessage,
};
use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_shared_macros::impl_report_conversion;

use super::protocol::{
    AuthenticatedContext, CertIdentity, MessageRateLimiter, WS_MESSAGE_RATE_LIMIT,
    WS_MESSAGE_RATE_WINDOW, close_with_reason, deserialize_service_msg, record_service_activity,
    record_system_service_activity, send_pong, serialize_controller_msg,
};
use crate::AppState;
use uptrakit_internal_wire::service_profile::parse_capabilities;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum size of the `update_history.output` column (50 MB).
///
/// Docker image pulls generate very verbose progress output (tens of megabytes
/// for large images). This cap covers virtually all real-world update outputs
/// while preventing unbounded DB growth.
///
/// When the cap is first exceeded, a visible system output line is emitted
/// into the stream and the `output_truncated` flag is set on the history
/// record so the UI can display a persistent warning banner.
const MAX_UPDATE_OUTPUT_BYTES: usize = 52_428_800;

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

/// Bounded channel capacity for messages forwarded to the processor.
const PROCESSOR_CHANNEL_CAPACITY: usize = 32;

/// Bounded channel capacity for responses from the processor.
const RESPONSE_CHANNEL_CAPACITY: usize = 32;

// ---------------------------------------------------------------------------
// LoopAction
// ---------------------------------------------------------------------------

/// Signal returned by message handlers to control the authenticated loop.
pub(super) enum LoopAction {
    /// Continue processing messages.
    Continue,
    /// Break out of the main loop (normal disconnect or error).
    Break,
}

impl LoopAction {
    /// Returns `true` if this action signals the loop should break.
    pub(super) fn is_break(&self) -> bool {
        matches!(self, Self::Break)
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Internal error type for helper functions (deliver_pending_updates, etc.).
#[derive(Debug, Error)]
enum HandlerError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("websocket send failed")]
    WebSocketSend,
}

type HandlerResult<T> = std::result::Result<T, Report<HandlerError>>;

impl_report_conversion!(sea_orm::DbErr => HandlerError::Database);

// ---------------------------------------------------------------------------
// ProcessorMessage
// ---------------------------------------------------------------------------

/// A deserialized service message forwarded to the background processor.
struct ProcessorMessage {
    message: ServiceMessage,
    pagination: Option<ReportPagination>,
}

// ---------------------------------------------------------------------------
// MessageProcessor
// ---------------------------------------------------------------------------

/// Background message processor spawned per WebSocket connection.
///
/// Receives deserialized [`ServiceMessage`] values via an MPSC channel,
/// dispatches them to the appropriate handler, and sends
/// [`ProcessorResponse`] values back to the main loop.
struct MessageProcessor {
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    cert: CertIdentity,
    is_system: bool,
    is_mqtt: bool,
    has_software_discovery: bool,
    has_update_hooks: bool,
    has_ui_extensions: bool,
    service_app_name: Option<String>,
    service_tenant_id: Option<uuid::Uuid>,
    linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    report_tracker: ReportTracker,
}

impl MessageProcessor {
    /// Run the processor loop: read messages, dispatch handlers, send responses.
    async fn run(
        mut self,
        mut msg_rx: tokio::sync::mpsc::Receiver<ProcessorMessage>,
        resp_tx: tokio::sync::mpsc::Sender<ProcessorResponse>,
    ) {
        while let Some(pm) = msg_rx.recv().await {
            let response = self.dispatch(pm.message, pm.pagination).await;
            if resp_tx.send(response).await.is_err() {
                // Main loop dropped -- connection is closing.
                break;
            }
        }
    }

    /// Dispatch a single service message to the appropriate handler.
    ///
    /// Messages are grouped by capability gate. Universal messages (available
    /// to all service types) are handled last.
    async fn dispatch(
        &mut self,
        service_msg: ServiceMessage,
        pagination: Option<ReportPagination>,
    ) -> ProcessorResponse {
        match service_msg {
            // -- SoftwareDiscovery capability --
            ServiceMessage::ReportHosts(payload) if self.has_software_discovery => {
                messages::handle_report_hosts(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }
            ServiceMessage::VersionCheckResults(payload)
                if self.has_software_discovery && !self.is_mqtt =>
            {
                messages::handle_version_check_results(&self.state, self.service_id, &payload).await
            }
            ServiceMessage::DiscoveryResults(payload) if self.has_software_discovery => {
                messages::handle_discovery_results(
                    &self.state,
                    self.service_id,
                    payload,
                    pagination.as_ref(),
                    &mut self.report_tracker,
                )
                .await
            }

            // -- UpdateHooks capability --
            msg @ (ServiceMessage::UpdateStarted(_)
            | ServiceMessage::UpdateOutput(_)
            | ServiceMessage::UpdateResult(_)
            | ServiceMessage::BatchUpdateResult(_)
            | ServiceMessage::StdinAttention(_))
                if self.has_update_hooks =>
            {
                self.dispatch_update_hooks(msg).await
            }

            // -- UpdateTracking capability --
            msg @ (ServiceMessage::ServiceTriggerUpdate(_)
            | ServiceMessage::ServiceTriggerHostBatchUpdate(_))
                if self.is_mqtt =>
            {
                self.dispatch_mqtt(msg).await
            }

            // -- UiExtensions capability --
            msg @ (ServiceMessage::ExtensionRegister(_)
            | ServiceMessage::ExtensionActionsRegister(_)
            | ServiceMessage::ExtensionResponse(_)
            | ServiceMessage::ExtensionRequest(_))
                if self.has_ui_extensions =>
            {
                self.dispatch_extensions(msg).await
            }

            // -- Universal messages (all capabilities) --
            ServiceMessage::RenewCertificate(payload) => {
                messages::handle_renew_certificate(
                    &self.state,
                    self.service_id,
                    &self.cert,
                    &payload,
                    self.is_system,
                )
                .await
            }
            ServiceMessage::ReportPluginConfig(payload) => {
                messages::handle_report_plugin_config(&self.state, self.service_id, &payload).await
            }
            ServiceMessage::StoreServiceConfig(payload) => {
                service_config::handle_store_service_config(
                    &self.state,
                    self.service_app_name.as_deref().unwrap_or(""),
                    self.service_id,
                    payload,
                )
                .await
            }
            ServiceMessage::DeleteServiceConfig(payload) => {
                service_config::handle_delete_service_config(
                    &self.state,
                    self.service_app_name.as_deref().unwrap_or(""),
                    self.service_id,
                    payload,
                )
                .await
            }
            _ => ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: "message not supported for this service capability".to_string(),
            })),
        }
    }

    /// Dispatch update-hooks messages (UpdateStarted, UpdateOutput, etc.).
    async fn dispatch_update_hooks(&self, msg: ServiceMessage) -> ProcessorResponse {
        match msg {
            ServiceMessage::UpdateStarted(payload) => {
                updates::handle_update_started(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }
            ServiceMessage::UpdateOutput(payload) => {
                updates::handle_update_output(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }
            ServiceMessage::UpdateResult(payload) => {
                updates::handle_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                )
                .await
            }
            ServiceMessage::BatchUpdateResult(payload) => {
                updates::handle_batch_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                )
                .await
            }
            ServiceMessage::StdinAttention(payload) => {
                updates::handle_stdin_attention(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }
            _ => unreachable!("dispatch_update_hooks called with non-update message"),
        }
    }

    /// Dispatch update-tracking messages (ServiceTriggerUpdate, etc.).
    async fn dispatch_mqtt(&self, msg: ServiceMessage) -> ProcessorResponse {
        match msg {
            ServiceMessage::ServiceTriggerUpdate(payload) => {
                update_tracking::handle_service_trigger_update(&self.state, &payload).await
            }
            ServiceMessage::ServiceTriggerHostBatchUpdate(payload) => {
                update_tracking::handle_service_trigger_host_batch_update(&self.state, &payload)
                    .await
            }
            _ => unreachable!("dispatch_mqtt called with non-update-tracking message"),
        }
    }

    /// Dispatch UI extension messages (ExtensionRegister, ExtensionResponse, etc.).
    async fn dispatch_extensions(&mut self, msg: ServiceMessage) -> ProcessorResponse {
        match msg {
            ServiceMessage::ExtensionRegister(payload) => {
                self.handle_extension_register(payload).await
            }
            ServiceMessage::ExtensionActionsRegister(payload) => {
                self.handle_extension_actions_register(payload)
            }
            ServiceMessage::ExtensionResponse(payload) => {
                let request_id = payload.request_id.clone();
                self.state.extension_proxy.complete(&request_id, payload);
                ProcessorResponse::cont()
            }
            ServiceMessage::ExtensionRequest(payload) => {
                self.handle_extension_request(payload).await
            }
            _ => unreachable!("dispatch_extensions called with non-extension message"),
        }
    }

    /// Handle an `ExtensionRegister` message: validate and register manifests.
    async fn handle_extension_register(
        &self,
        payload: uptrakit_internal_wire::extension::ExtensionRegisterPayload,
    ) -> ProcessorResponse {
        if let Err(e) = payload.wire_validate() {
            tracing::warn!(
                service_id = %self.service_id,
                error = %e,
                "invalid ExtensionRegister payload"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: format!("invalid extension manifests: {e}"),
            }));
        }
        let app_name = self.service_app_name.as_deref().unwrap_or("unknown");
        if let Err(e) = self.state.extension_registry.register_service(
            self.service_id,
            app_name,
            payload.manifests,
            payload.encryption_public_key,
        ) {
            tracing::warn!(
                service_id = %self.service_id,
                error = %e,
                "extension registration rejected"
            );
            ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: e.to_string(),
            }))
        } else {
            tracing::info!(
                service_id = %self.service_id,
                app_name,
                "registered UI extensions"
            );
            ProcessorResponse::cont()
        }
    }

    /// Handle an `ExtensionActionsRegister` message: validate and register actions.
    fn handle_extension_actions_register(
        &self,
        payload: uptrakit_internal_wire::extension::ExtensionActionsPayload,
    ) -> ProcessorResponse {
        if let Err(e) = payload.wire_validate() {
            tracing::warn!(
                service_id = %self.service_id,
                error = %e,
                "invalid ExtensionActionsRegister payload"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: format!("invalid extension actions: {e}"),
            }));
        }
        let app_name = self.service_app_name.as_deref().unwrap_or("unknown");
        self.state
            .extension_registry
            .register_actions(self.service_id, app_name, payload.actions);
        tracing::info!(
            service_id = %self.service_id,
            app_name,
            "registered extension actions"
        );
        ProcessorResponse::cont()
    }

    /// Handle an `ExtensionRequest` message: service-initiated extension action.
    async fn handle_extension_request(
        &self,
        payload: uptrakit_internal_wire::extension::ExtensionRequestPayload,
    ) -> ProcessorResponse {
        let request_id = payload.request_id.clone();
        tracing::debug!(
            service_id = %self.service_id,
            request_id = %request_id,
            extension_id = %payload.extension_id,
            action_id = %payload.action_id,
            "service-initiated extension action request"
        );

        let owner = self
            .state
            .extension_registry
            .find_owner(&payload.extension_id);
        let response = match owner {
            crate::extension_registry::ExtensionOwner::Plugin => {
                let ctx = uptrakit_plugin_infrastructure_registry::ExtensionActionContext {
                    db: self.state.db(),
                    tenant_id: self.service_tenant_id,
                    caller_user_id: None,
                };
                match self
                    .state
                    .plugin_ops
                    .handle_extension_action(
                        &ctx,
                        &payload.extension_id,
                        &payload.action_id,
                        payload.params,
                    )
                    .await
                {
                    Ok(data) => uptrakit_internal_wire::extension::ExtensionResponsePayload {
                        request_id,
                        success: true,
                        data,
                        error: None,
                    },
                    Err(msg) => uptrakit_internal_wire::extension::ExtensionResponsePayload {
                        request_id,
                        success: false,
                        data: serde_json::Value::Null,
                        error: Some(msg),
                    },
                }
            }
            _ => uptrakit_internal_wire::extension::ExtensionResponsePayload {
                request_id,
                success: false,
                data: serde_json::Value::Null,
                error: Some("extension not found or not plugin-backed".to_string()),
            },
        };

        ProcessorResponse::reply(ControllerMessage::ExtensionResponse(response))
    }
}

// ---------------------------------------------------------------------------
// AuthenticatedSessionState
// ---------------------------------------------------------------------------

/// All state produced during authenticated session setup that the main loop
/// and cleanup phases need.
struct AuthenticatedSessionState {
    service_id: uuid::Uuid,
    is_system: bool,
    is_mqtt: bool,
    has_software_discovery: bool,
    has_ui_extensions: bool,
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
                svc.service_app_name,
                None,
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
/// Returns `(push_rx, cancel_token)`.
async fn register_connection(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    capabilities: &BTreeSet<Capability>,
    service_app_name: Option<String>,
) -> (
    tokio::sync::mpsc::Receiver<ControllerMessage>,
    tokio_util::sync::CancellationToken,
) {
    let (push_rx, cancel_token) = state
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

    (push_rx, cancel_token)
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

/// Stage 5: Deliver pending updates to services with the `UpdateHooks` capability.
///
/// Errors are logged but do not abort setup — the connection is still usable.
async fn deliver_pending_updates_on_connect(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    has_update_hooks: bool,
    out_seq: &mut OutgoingSeq,
) {
    if has_update_hooks
        && let Err(e) = deliver_pending_updates(state, service_id, sink, out_seq).await
    {
        tracing::error!(error = %e, %service_id, "failed to deliver pending updates on reconnect");
    }
}

/// Output of [`spawn_message_processor`]: channels for communicating with the
/// background task.
struct ProcessorChannels {
    msg_tx: tokio::sync::mpsc::Sender<ProcessorMessage>,
    resp_rx: tokio::sync::mpsc::Receiver<ProcessorResponse>,
    processor_cancel: tokio_util::sync::CancellationToken,
    processor_handle: tokio::task::JoinHandle<()>,
}

/// Stage 6: Spawn the background [`MessageProcessor`] and return the channels
/// the main loop needs to exchange messages with it.
fn spawn_message_processor(processor: MessageProcessor) -> ProcessorChannels {
    let (msg_tx, msg_rx) =
        tokio::sync::mpsc::channel::<ProcessorMessage>(PROCESSOR_CHANNEL_CAPACITY);
    let (resp_tx, resp_rx) =
        tokio::sync::mpsc::channel::<ProcessorResponse>(RESPONSE_CHANNEL_CAPACITY);

    let processor_cancel = tokio_util::sync::CancellationToken::new();
    let proc_cancel_clone = processor_cancel.clone();
    let processor_handle = tokio::spawn(async move {
        tokio::select! {
            () = processor.run(msg_rx, resp_tx) => {}
            () = proc_cancel_clone.cancelled() => {}
        }
    });

    ProcessorChannels {
        msg_tx,
        resp_rx,
        processor_cancel,
        processor_handle,
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
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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
    // capability flags (is_mqtt, has_software_discovery, etc.) come from the
    // Register handshake in Stage 3 so they are correct on first connect even
    // when the DB row has no stored capabilities yet.
    let (db_capabilities, service_app_name, service_tenant_id) =
        load_service_capabilities(state, service_id, is_system).await;

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Stage 2: Deliver credentials to services that have credential capabilities.
    deliver_service_credentials(sink, state, &db_capabilities, service_id, out_seq).await?;

    // Stage 2.5: Deliver stored service config entries to services with a known app name.
    if let Some(ref app_name) = service_app_name {
        service_config::deliver_service_config(sink, state, app_name, out_seq).await?;
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

    let session_capabilities = register_payload.capabilities.clone();
    let is_mqtt = session_capabilities.contains(&Capability::UpdateTracking);
    let has_software_discovery = session_capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = session_capabilities.contains(&Capability::UpdateHooks);
    let has_ui_extensions = session_capabilities.contains(&Capability::UiExtensions);

    // Persist the session capabilities to the DB so that subsequent reconnects
    // (and other controller instances) see the up-to-date capability set.
    upgrade_service_capabilities(
        state.db(),
        service_id,
        is_system,
        register_payload.capabilities,
        &mut { has_ui_extensions },
    )
    .await;

    // Stage 4: Register the connection and notify embedded services.
    let (push_rx, cancel_token) = register_connection(
        state,
        service_id,
        &session_capabilities,
        service_app_name.clone(),
    )
    .await;

    // Stage 5: Load linked host IDs shared between the main loop and the processor.
    let linked_host_ids = load_session_host_ids(state, service_id, has_software_discovery).await;

    // Stage 6: Deliver pending updates to services with the `UpdateHooks` capability.
    deliver_pending_updates_on_connect(sink, state, service_id, has_update_hooks, out_seq).await;

    // Stage 7: Spawn the background message processor.
    let processor = MessageProcessor {
        state: Arc::clone(state),
        service_id,
        cert: cert.clone(),
        is_system,
        is_mqtt,
        has_software_discovery,
        has_update_hooks,
        has_ui_extensions,
        service_app_name,
        service_tenant_id,
        linked_host_ids: Arc::clone(&linked_host_ids),
        report_tracker: ReportTracker::new(),
    };
    let channels = spawn_message_processor(processor);

    Some(AuthenticatedSessionState {
        service_id,
        is_system,
        is_mqtt,
        has_software_discovery,
        has_ui_extensions,
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
        is_mqtt,
        has_software_discovery,
        has_ui_extensions,
        service_tenant_id,
        linked_host_ids,
        processor_cancel,
        processor_handle,
        ..
    } = session;

    // Cancel the processor task and wait for it to finish.
    processor_cancel.cancel();
    let _ = processor_handle.await;

    // Unregister UI extensions before connection teardown.
    if has_ui_extensions {
        state.extension_registry.unregister_service(&service_id);
    }

    // Notify services that this agent's hosts are now offline.
    if !is_system
        && !is_mqtt
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
#[allow(clippy::too_many_arguments)]
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
                session.processor_cancel.cancel();
                let _ = session.processor_handle.await;
                // Genuine supersession: the new connection owns the registry
                // entry so cleanup is skipped. Force-disconnect: the entry was
                // already removed by force_disconnect(), so we must notify
                // embedded services that the yield condition may have cleared.
                if !state.service_connections.is_connected(&service_id).await
                    && let Some(ref notifier) = state.embedded_service_notifier
                {
                    notifier.on_external_disconnected(&service_id);
                }
                return;
            }
        }
    }

    // ------------------------------------------------------------------
    // Cleanup
    // ------------------------------------------------------------------
    cleanup_authenticated_session(state, session).await;
}

// ---------------------------------------------------------------------------
// upgrade_service_capabilities
// ---------------------------------------------------------------------------

/// Persist the service's current capability set to the database and refresh
/// in-session gating flags.
async fn upgrade_service_capabilities(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    is_system: bool,
    capabilities: std::collections::BTreeSet<Capability>,
    has_ui_extensions: &mut bool,
) {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_internal_wire::service_profile::serialize_capabilities;

    let new_caps_json = serialize_capabilities(&capabilities);
    let had_ui_extensions = *has_ui_extensions;
    *has_ui_extensions = capabilities.contains(&Capability::UiExtensions);

    if had_ui_extensions != *has_ui_extensions {
        tracing::info!(
            %service_id,
            ui_extensions = *has_ui_extensions,
            "service UiExtensions capability changed in-session",
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
    // Fetch service to derive capabilities for registration.
    let capabilities: BTreeSet<Capability> = if is_system {
        match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => parse_capabilities(&svc.capabilities),
            _ => BTreeSet::new(),
        }
    } else {
        match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(svc)) => parse_capabilities(&svc.capabilities),
            _ => BTreeSet::new(),
        }
    };

    // Register in service_connections.
    let (push_rx, cancel_token) = state
        .service_connections
        .register(service_id, capabilities.clone(), None, None, None)
        .await;

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
