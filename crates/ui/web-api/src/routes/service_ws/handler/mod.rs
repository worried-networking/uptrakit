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
//! sends [`ProcessorResponse`](messages::ProcessorResponse) values back
//! to the main loop, which serializes and writes replies to the WebSocket
//! sink with `out_seq` staying in the main loop.
//!
//! # Public API
//!
//! - [`handle_authenticated_loop`] -- post-certificate operational loop.
//! - [`handle_enrolled_loop`] -- pre-certificate enrollment loop.
//! - [`trigger_discovery_for_agent_host`] -- send `DiscoverSoftware` to an
//!   agent for a specific host (also used by `hosts.rs`).

mod discovery;
pub(super) mod messages;
mod mqtt;
mod renewal;
mod updates;

pub(crate) use discovery::trigger_discovery_for_agent_host;
use messages::{ProcessorAction, ProcessorResponse};
use mqtt::{complete_mqtt_registration, handle_mqtt_register_handshake};
use updates::{deliver_pending_updates, load_linked_host_ids};

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use thiserror::Error;

use rootcause::prelude::*;
use sea_orm::EntityTrait;

use uptrakit_internal_wire::limits::WireValidate;
use uptrakit_internal_wire::payloads::RequestCertificatePayload;
use uptrakit_internal_wire::report_tracker::ReportTracker;
use uptrakit_internal_wire::{
    ApprovedPayload, Capability, CertificatePayload, CloseReason, ControllerMessage, ErrorCode,
    ErrorPayload, HostConnectivityUpdate, IncomingSeq, MqttRegisteredPayload,
    MqttTenantAssignmentsPayload, OutgoingSeq, PingPayload, RejectedPayload, ReportPagination,
    ServiceCredentialsPayload, ServiceMessage, UpdateCapabilitiesPayload,
};
use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_shared_macros::impl_report_conversion;

use super::protocol::{
    AuthenticatedContext, CertIdentity, MessageRateLimiter, WS_MESSAGE_RATE_LIMIT,
    WS_MESSAGE_RATE_WINDOW, close_with_reason, deserialize_service_msg, record_service_activity,
    record_system_service_activity, send_pong, serialize_controller_msg,
};
use crate::AppState;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;
use crate::routes::agents::{do_sign_csr, do_sign_csr_for_system_service};
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
    mqtt_context: Option<mqtt::MqttContext>,
    lease_coordinator: Option<MqttLeaseCoordinator>,
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
    async fn dispatch(
        &mut self,
        service_msg: ServiceMessage,
        pagination: Option<ReportPagination>,
    ) -> ProcessorResponse {
        match service_msg {
            // -------------------------------------------------
            // RenewCertificate (all capabilities)
            // -------------------------------------------------
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

            // -------------------------------------------------
            // ReportHosts (requires SoftwareDiscovery)
            // -------------------------------------------------
            ServiceMessage::ReportHosts(payload) if self.has_software_discovery => {
                messages::handle_report_hosts(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }

            // -------------------------------------------------
            // VersionCheckResults (SoftwareDiscovery AND NOT MqttBridge)
            // -------------------------------------------------
            ServiceMessage::VersionCheckResults(payload)
                if self.has_software_discovery && !self.is_mqtt =>
            {
                messages::handle_version_check_results(&self.state, self.service_id, &payload).await
            }

            // -------------------------------------------------
            // UpdateStarted (requires UpdateHooks)
            // -------------------------------------------------
            ServiceMessage::UpdateStarted(payload) if self.has_update_hooks => {
                updates::handle_update_started(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }

            // -------------------------------------------------
            // UpdateOutput (requires UpdateHooks)
            // -------------------------------------------------
            ServiceMessage::UpdateOutput(payload) if self.has_update_hooks => {
                updates::handle_update_output(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }

            // -------------------------------------------------
            // UpdateResult (requires UpdateHooks)
            // -------------------------------------------------
            ServiceMessage::UpdateResult(payload) if self.has_update_hooks => {
                updates::handle_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                )
                .await
            }

            // -------------------------------------------------
            // BatchUpdateResult (requires UpdateHooks)
            // -------------------------------------------------
            ServiceMessage::BatchUpdateResult(payload) if self.has_update_hooks => {
                updates::handle_batch_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                )
                .await
            }

            // -------------------------------------------------
            // StdinAttention (requires UpdateHooks)
            // -------------------------------------------------
            ServiceMessage::StdinAttention(payload) if self.has_update_hooks => {
                updates::handle_stdin_attention(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                )
                .await
            }

            // -------------------------------------------------
            // DiscoveryResults (requires SoftwareDiscovery)
            // -------------------------------------------------
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

            // -------------------------------------------------
            // ReportPluginConfig (all capabilities)
            // -------------------------------------------------
            ServiceMessage::ReportPluginConfig(payload) => {
                messages::handle_report_plugin_config(&self.state, self.service_id, &payload).await
            }

            // -------------------------------------------------
            // ReleaseTenants (requires MqttBridge)
            // -------------------------------------------------
            ServiceMessage::ReleaseTenants(payload) if self.is_mqtt => {
                mqtt::handle_release_tenants(
                    &self.state,
                    self.service_id,
                    &payload,
                    self.lease_coordinator.as_ref(),
                )
                .await
            }

            // -------------------------------------------------
            // MqttClientStatus (requires MqttBridge)
            // -------------------------------------------------
            ServiceMessage::MqttClientStatus(payload) if self.is_mqtt => {
                mqtt::handle_mqtt_client_status(&self.state, &payload).await
            }

            // -------------------------------------------------
            // MqttTriggerUpdate (requires MqttBridge)
            // -------------------------------------------------
            ServiceMessage::MqttTriggerUpdate(payload) if self.is_mqtt => {
                mqtt::handle_mqtt_trigger_update(&self.state, &payload, self.mqtt_context.as_ref())
                    .await
            }

            // -------------------------------------------------
            // MqttTriggerHostBatchUpdate (requires MqttBridge)
            // -------------------------------------------------
            ServiceMessage::MqttTriggerHostBatchUpdate(payload) if self.is_mqtt => {
                mqtt::handle_mqtt_trigger_host_batch_update(
                    &self.state,
                    &payload,
                    self.mqtt_context.as_ref(),
                )
                .await
            }

            // -------------------------------------------------
            // UpdateCapabilities (all capabilities)
            // -------------------------------------------------
            ServiceMessage::UpdateCapabilities(payload) => {
                upgrade_service_capabilities(
                    self.state.db(),
                    self.service_id,
                    self.is_system,
                    payload,
                    &mut self.has_ui_extensions,
                )
                .await;
                ProcessorResponse::cont()
            }

            // -------------------------------------------------
            // ExtensionRegister (requires UiExtensions)
            // -------------------------------------------------
            ServiceMessage::ExtensionRegister(payload) if self.has_ui_extensions => {
                if let Err(e) = payload.wire_validate() {
                    tracing::warn!(
                        service_id = %self.service_id,
                        error = %e,
                        "invalid ExtensionRegister payload"
                    );
                    ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                        code: ErrorCode::BadRequest,
                        message: format!("invalid extension manifests: {e}"),
                    }))
                } else {
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
            }

            // -------------------------------------------------
            // ExtensionActionsRegister (requires UiExtensions)
            // -------------------------------------------------
            ServiceMessage::ExtensionActionsRegister(payload) if self.has_ui_extensions => {
                if let Err(e) = payload.wire_validate() {
                    tracing::warn!(
                        service_id = %self.service_id,
                        error = %e,
                        "invalid ExtensionActionsRegister payload"
                    );
                    ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                        code: ErrorCode::BadRequest,
                        message: format!("invalid extension actions: {e}"),
                    }))
                } else {
                    let app_name = self.service_app_name.as_deref().unwrap_or("unknown");
                    self.state.extension_registry.register_actions(
                        self.service_id,
                        app_name,
                        payload.actions,
                    );
                    tracing::info!(
                        service_id = %self.service_id,
                        app_name,
                        "registered extension actions"
                    );
                    ProcessorResponse::cont()
                }
            }

            // -------------------------------------------------
            // ExtensionResponse (requires UiExtensions)
            // -------------------------------------------------
            ServiceMessage::ExtensionResponse(payload) if self.has_ui_extensions => {
                let request_id = payload.request_id.clone();
                self.state.extension_proxy.complete(&request_id, payload);
                ProcessorResponse::cont()
            }

            // -------------------------------------------------
            // ExtensionRequest: service-initiated extension invocation
            // -------------------------------------------------
            ServiceMessage::ExtensionRequest(payload) if self.has_ui_extensions => {
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
                            Ok(data) => {
                                uptrakit_internal_wire::extension::ExtensionResponsePayload {
                                    request_id,
                                    success: true,
                                    data,
                                    error: None,
                                }
                            }
                            Err(msg) => {
                                uptrakit_internal_wire::extension::ExtensionResponsePayload {
                                    request_id,
                                    success: false,
                                    data: serde_json::Value::Null,
                                    error: Some(msg),
                                }
                            }
                        }
                    }
                    _ => {
                        // Service-to-service proxying not supported for service-initiated requests.
                        uptrakit_internal_wire::extension::ExtensionResponsePayload {
                            request_id,
                            success: false,
                            data: serde_json::Value::Null,
                            error: Some("extension not found or not plugin-backed".to_string()),
                        }
                    }
                };

                ProcessorResponse::reply(ControllerMessage::ExtensionResponse(response))
            }

            // -------------------------------------------------
            // Wildcard: message not supported for this capability
            // -------------------------------------------------
            _ => ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: "message not supported for this service capability".to_string(),
            })),
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
    is_system: bool,
    is_mqtt: bool,
    has_software_discovery: bool,
    has_ui_extensions: bool,
    is_external_scheduler: bool,
    service_tenant_id: Option<uuid::Uuid>,
    linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    lease_coordinator: Option<MqttLeaseCoordinator>,
    push_rx: tokio::sync::mpsc::Receiver<ControllerMessage>,
    cancel_token: tokio_util::sync::CancellationToken,
    msg_tx: tokio::sync::mpsc::Sender<ProcessorMessage>,
    resp_rx: tokio::sync::mpsc::Receiver<ProcessorResponse>,
    processor_cancel: tokio_util::sync::CancellationToken,
    processor_handle: tokio::task::JoinHandle<()>,
    rate_limiter: MessageRateLimiter,
}

// ---------------------------------------------------------------------------
// setup_authenticated_session
// ---------------------------------------------------------------------------

/// Perform all pre-loop setup for the authenticated handler.
///
/// Loads the service from the DB, delivers credentials, runs the MQTT
/// handshake (if applicable), registers the connection, spawns the background
/// processor, and delivers pending updates.
///
/// Returns `None` if the connection must be closed early (e.g. failed MQTT
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
    // Load service from DB, derive capabilities, app name, and tenant ID.
    let (capabilities, service_app_name, service_tenant_id): (
        BTreeSet<Capability>,
        Option<String>,
        Option<uuid::Uuid>,
    ) = if is_system {
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
    };

    let is_mqtt = capabilities.contains(&Capability::MqttBridge);
    let has_software_discovery = capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = capabilities.contains(&Capability::UpdateHooks);
    let has_ui_extensions = capabilities.contains(&Capability::UiExtensions);

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // ------------------------------------------------------------------
    // Credential delivery for services with credential capabilities
    // ------------------------------------------------------------------
    {
        let has_db_access = capabilities.contains(&Capability::DatabaseAccess);
        let has_nats_access = capabilities.contains(&Capability::NatsAccess);
        let has_master_key_access = capabilities.contains(&Capability::MasterKeyAccess);

        if has_db_access || has_nats_access || has_master_key_access {
            let sources = &state.credential_sources;
            let payload = ServiceCredentialsPayload {
                db_url: if has_db_access {
                    sources
                        .db_url
                        .as_ref()
                        .map(|u| uptrakit_internal_wire::SecretString::new(u.clone()))
                } else {
                    None
                },
                nats_url: if has_nats_access {
                    sources.nats_url.clone()
                } else {
                    None
                },
                master_key_hex: if has_master_key_access {
                    sources.master_key_hex.clone()
                } else {
                    None
                },
            };
            let cred_msg = ControllerMessage::ServiceCredentials(payload);
            if let Some(json) = serialize_controller_msg(out_seq, cred_msg)
                && sink.send(Message::Text(json.into())).await.is_err()
            {
                return None;
            }
            tracing::info!(
                %service_id,
                db = has_db_access,
                nats = has_nats_access,
                master_key = has_master_key_access,
                "delivered service credentials"
            );
        }
    }

    // ------------------------------------------------------------------
    // MQTT pre-loop phase: wait for Register message (handshake only)
    // ------------------------------------------------------------------
    let mqtt_handshake = if is_mqtt {
        match handle_mqtt_register_handshake(
            sink,
            stream,
            state,
            service_id,
            out_seq,
            in_seq,
            &mut rate_limiter,
        )
        .await
        {
            Some(h) => Some(h),
            None => return None, // connection closed before Register
        }
    } else {
        None
    };

    // ------------------------------------------------------------------
    // Register in service_connections (must happen before lease assignment)
    // ------------------------------------------------------------------
    let (push_rx, cancel_token) = if let Some(ref h) = mqtt_handshake {
        state
            .service_connections
            .register(
                service_id,
                capabilities.clone(),
                Some(h.instance_id.clone()),
                Some(h.max_tenants),
            )
            .await
    } else {
        state
            .service_connections
            .register(service_id, capabilities.clone(), None, None)
            .await
    };

    // ------------------------------------------------------------------
    // External scheduler detection
    // ------------------------------------------------------------------
    let is_external_scheduler = capabilities.contains(&Capability::Scheduler);
    if is_external_scheduler {
        state
            .external_scheduler_connected
            .store(true, Ordering::Relaxed);
        tracing::info!(
            %service_id,
            "external scheduler connected; embedded scheduler deferring external tasks"
        );
    }

    // ------------------------------------------------------------------
    // MQTT post-registration: assign/reconcile leases now that the service
    // entry exists in the registry
    // ------------------------------------------------------------------
    let mqtt_context = if let Some(h) = mqtt_handshake {
        Some(complete_mqtt_registration(state, service_id, h).await)
    } else {
        None
    };

    // ------------------------------------------------------------------
    // MQTT post-registration: send Registered, TenantAssignments, push states
    // ------------------------------------------------------------------
    if let Some(ref mctx) = mqtt_context {
        // Send Registered acknowledgment.
        let registered_msg = ControllerMessage::Registered(MqttRegisteredPayload {
            instance_id: mctx.instance_id.clone(),
        });
        let Some(json) = serialize_controller_msg(out_seq, registered_msg) else {
            state.service_connections.unregister(&service_id).await;
            return None;
        };
        if sink.send(Message::Text(json.into())).await.is_err() {
            state.service_connections.unregister(&service_id).await;
            return None;
        }

        // Send initial tenant assignments.
        if !mctx.tenant_configs.is_empty() {
            let assignments_msg =
                ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload {
                    tenants: mctx.tenant_configs.clone(),
                });
            let Some(json) = serialize_controller_msg(out_seq, assignments_msg) else {
                state.service_connections.unregister(&service_id).await;
                return None;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                state.service_connections.unregister(&service_id).await;
                return None;
            }
        }

        // Push current software states for each newly assigned tenant.
        {
            let mut seen_tenants = HashSet::new();
            for cfg in &mctx.tenant_configs {
                if seen_tenants.insert(cfg.tenant_id) {
                    state
                        .notification_service
                        .push_software_states_for_tenant(state.db(), cfg.tenant_id)
                        .await;
                }
            }
        }

        // Push connectivity state for agents that are already connected.
        {
            let mut seen_tenants_conn = HashSet::new();
            for cfg in &mctx.tenant_configs {
                if seen_tenants_conn.insert(cfg.tenant_id) {
                    state
                        .notification_service
                        .push_connected_agent_states_for_tenant(state.db(), cfg.tenant_id)
                        .await;
                }
            }
        }

        tracing::info!(
            %service_id,
            instance_id = %mctx.instance_id,
            "MQTT service registered"
        );
    }

    // ------------------------------------------------------------------
    // SoftwareDiscovery: load linked host IDs (shared with processor)
    // ------------------------------------------------------------------
    let linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>> = if has_software_discovery {
        Arc::new(parking_lot::Mutex::new(
            load_linked_host_ids(state.db(), service_id)
                .await
                .unwrap_or_default(),
        ))
    } else {
        Arc::new(parking_lot::Mutex::new(HashSet::new()))
    };

    // ------------------------------------------------------------------
    // UpdateHooks: deliver pending updates (non-MQTT only)
    // ------------------------------------------------------------------
    if has_update_hooks
        && !is_mqtt
        && let Err(e) = deliver_pending_updates(state, service_id, sink, out_seq).await
    {
        tracing::error!(error = %e, %service_id, "failed to deliver pending updates on reconnect");
    }

    // ------------------------------------------------------------------
    // Create lease coordinator if MQTT
    // ------------------------------------------------------------------
    let lease_coordinator = if is_mqtt {
        Some(MqttLeaseCoordinator::new(
            state.db().clone(),
            state.service_connections.clone(),
        ))
    } else {
        None
    };

    // ------------------------------------------------------------------
    // Spawn background message processor
    // ------------------------------------------------------------------
    let (msg_tx, msg_rx) =
        tokio::sync::mpsc::channel::<ProcessorMessage>(PROCESSOR_CHANNEL_CAPACITY);
    let (resp_tx, resp_rx) =
        tokio::sync::mpsc::channel::<ProcessorResponse>(RESPONSE_CHANNEL_CAPACITY);

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
        mqtt_context,
        lease_coordinator: lease_coordinator.clone(),
        report_tracker: ReportTracker::new(),
    };

    let processor_cancel = tokio_util::sync::CancellationToken::new();
    let proc_cancel_clone = processor_cancel.clone();
    let processor_handle = tokio::spawn(async move {
        tokio::select! {
            () = processor.run(msg_rx, resp_tx) => {}
            () = proc_cancel_clone.cancelled() => {}
        }
    });

    Some(AuthenticatedSessionState {
        service_id,
        is_system,
        is_mqtt,
        has_software_discovery,
        has_ui_extensions,
        is_external_scheduler,
        service_tenant_id,
        linked_host_ids,
        lease_coordinator,
        push_rx,
        cancel_token,
        msg_tx,
        resp_rx,
        processor_cancel,
        processor_handle,
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
        is_external_scheduler,
        service_tenant_id,
        linked_host_ids,
        lease_coordinator,
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

    if let Some(ref lc) = lease_coordinator
        && let Err(e) = lc.release_all_for_service(&service_id).await
    {
        tracing::error!(error = %e, "failed to release leases on disconnect");
    }

    // Notify MQTT services that this agent's hosts are now offline.
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

    if is_external_scheduler {
        // Unregister first so has_capability_connected excludes this service.
        state.service_connections.unregister(&service_id).await;
        if !state
            .service_connections
            .has_capability_connected(&Capability::Scheduler)
            .await
        {
            state
                .external_scheduler_connected
                .store(false, Ordering::Relaxed);
            tracing::info!(
                %service_id,
                "external scheduler disconnected; embedded scheduler resuming all tasks"
            );
        }
    } else {
        state.service_connections.unregister(&service_id).await;
    }
    tracing::debug!(%service_id, "authenticated service disconnected");
}

// ---------------------------------------------------------------------------
// handle_authenticated_loop
// ---------------------------------------------------------------------------

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
                        let deserialized =
                            match deserialize_service_msg(in_seq, &text) {
                                Ok(Some(m)) => m,
                                Ok(None) => continue,
                                Err(e) => {
                                    tracing::debug!(error = %e, "deserialize error");
                                    break;
                                }
                            };
                        let pagination = deserialized.pagination;
                        let service_msg = deserialized.message;

                        // -- Inline fast-path messages --
                        match &service_msg {
                            ServiceMessage::Ping(PingPayload { service_ts, .. }) => {
                                if messages::handle_ping(sink, out_seq, state, service_id, *service_ts, session.lease_coordinator.as_ref(), is_system).await.is_break() {
                                    break;
                                }
                                consecutive_unknown = 0;
                                continue;
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "service disconnecting gracefully"
                                );
                                break;
                            }
                            ServiceMessage::Unknown => {
                                consecutive_unknown += 1;
                                tracing::warn!(
                                    %service_id,
                                    consecutive_unknown,
                                    "received unknown service message type; \
                                     ignoring for forward compatibility"
                                );
                                if consecutive_unknown >= MAX_CONSECUTIVE_UNKNOWN_MESSAGES {
                                    tracing::warn!(
                                        %service_id,
                                        "closing connection: {MAX_CONSECUTIVE_UNKNOWN_MESSAGES} \
                                         consecutive unknown messages"
                                    );
                                    let _ = close_with_reason(
                                        sink,
                                        CloseReason::RateLimitExceeded,
                                    )
                                    .await;
                                    break;
                                }
                                continue;
                            }
                            _ => {}
                        }

                        // Reset unknown counter -- any known message breaks the streak.
                        consecutive_unknown = 0;

                        // Forward to processor
                        if session.msg_tx.send(ProcessorMessage { message: service_msg, pagination }).await.is_err() {
                            tracing::debug!("processor channel closed, breaking main loop");
                            break;
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
                match tokio::time::timeout(
                    WS_WRITE_TIMEOUT,
                    sink.send(Message::Text(json.into())),
                ).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => {
                        tracing::warn!(
                            %service_id,
                            "WebSocket write timed out after {}s, dropping connection",
                            WS_WRITE_TIMEOUT.as_secs(),
                        );
                        break;
                    }
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
                    match tokio::time::timeout(
                        WS_WRITE_TIMEOUT,
                        sink.send(Message::Text(json.into())),
                    ).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => { write_failed = true; break; }
                        Err(_) => {
                            tracing::warn!(
                                %service_id,
                                "WebSocket write timed out after {}s on processor reply",
                                WS_WRITE_TIMEOUT.as_secs(),
                            );
                            write_failed = true;
                            break;
                        }
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

            // 4. Connection superseded
            _ = session.cancel_token.cancelled() => {
                tracing::info!(%service_id, "connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Do NOT unregister -- the new connection owns the registry entry.
                // Release MQTT leases if applicable (new connection will re-reconcile).
                if let Some(ref lc) = session.lease_coordinator
                    && let Err(e) = lc.release_all_for_service(&service_id).await
                {
                    tracing::error!(error = %e, "failed to release leases on superseded disconnect");
                }
                session.processor_cancel.cancel();
                let _ = session.processor_handle.await;
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
    payload: UpdateCapabilitiesPayload,
    has_ui_extensions: &mut bool,
) {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_internal_wire::service_profile::serialize_capabilities;

    let new_caps_json = serialize_capabilities(&payload.capabilities);
    let had_ui_extensions = *has_ui_extensions;
    *has_ui_extensions = payload.capabilities.contains(&Capability::UiExtensions);

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
    is_external_scheduler: bool,
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
        .register(service_id, capabilities.clone(), None, None)
        .await;

    // External scheduler detection.
    let is_external_scheduler = capabilities.contains(&Capability::Scheduler);
    if is_external_scheduler {
        state
            .external_scheduler_connected
            .store(true, Ordering::Relaxed);
        tracing::info!(
            %service_id,
            "external scheduler connected (enrolled); embedded scheduler deferring external tasks"
        );
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
        is_external_scheduler,
        push_rx,
        cancel_token,
        approved,
        rate_limiter,
        approval_poll,
    }
}

// ---------------------------------------------------------------------------
// CertificateResult
// ---------------------------------------------------------------------------

/// Result of processing a `RequestCertificate` message.
enum CertificateResult {
    /// Certificate issued (or error sent); break out of the main loop.
    Break,
    /// Service not yet approved; already sent error reply, continue looping.
    NotApproved,
}

// ---------------------------------------------------------------------------
// handle_request_certificate
// ---------------------------------------------------------------------------

/// Handle a `RequestCertificate` message during the enrolled loop.
///
/// Signs the CSR for either a system service or a regular service, sends the
/// certificate (or error) back over the WebSocket, and returns whether the
/// loop should break or continue.
async fn handle_request_certificate(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    approved: bool,
    out_seq: &mut OutgoingSeq,
    payload: &RequestCertificatePayload,
) -> CertificateResult {
    if !approved {
        let err = ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::NotApproved,
            message: "service is not yet approved".to_string(),
        });
        if let Some(json) = serialize_controller_msg(out_seq, err) {
            let _ = sink.send(Message::Text(json.into())).await;
        }
        return CertificateResult::NotApproved;
    }

    if is_system {
        let sys_svc = match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(s)) => s,
            _ => {
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::InternalError,
                    message: "system service not found".to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                return CertificateResult::Break;
            }
        };

        match do_sign_csr_for_system_service(
            state.cert_signer.as_ref(),
            &state.settings,
            state.db(),
            sys_svc,
            &payload.csr_pem,
        )
        .await
        {
            Ok(bundle) => {
                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                tracing::info!(%service_id, "system service certificate issued via WS");
            }
            Err(e) => {
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.current_context().to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
            }
        }
    } else {
        let svc = match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(s)) => s,
            _ => {
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::InternalError,
                    message: "service not found".to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                return CertificateResult::Break;
            }
        };

        match do_sign_csr(
            state.cert_signer.as_ref(),
            &state.settings,
            state.db(),
            svc,
            &payload.csr_pem,
        )
        .await
        {
            Ok(bundle) => {
                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                tracing::info!(%service_id, "certificate issued via WS");
            }
            Err(e) => {
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.current_context().to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
            }
        }
    }

    CertificateResult::Break
}

// ---------------------------------------------------------------------------
// handle_enrolled_loop
// ---------------------------------------------------------------------------

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
                if is_system {
                    if let Ok(Some(s)) = sys_svc_entity::Entity::find_by_id(service_id)
                        .one(state.db())
                        .await
                    {
                        match s.status {
                            sys_svc_entity::SystemServiceStatus::Approved => {
                                session.approved = true;
                                let msg = ControllerMessage::Approved(ApprovedPayload {
                                    service_id,
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            sys_svc_entity::SystemServiceStatus::Rejected => {
                                let msg = ControllerMessage::Rejected(RejectedPayload {
                                    service_id,
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                } else if let Ok(Some(s)) = service::Entity::find_by_id(service_id)
                    .one(state.db())
                    .await
                {
                    match s.status {
                        service::ServiceStatus::Approved => {
                            session.approved = true;
                            let msg = ControllerMessage::Approved(ApprovedPayload {
                                service_id,
                            });
                            if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                        }
                        service::ServiceStatus::Rejected => {
                            let msg = ControllerMessage::Rejected(RejectedPayload {
                                service_id,
                            });
                            if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            _ = session.cancel_token.cancelled() => {
                tracing::info!(%service_id, "enrolled connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Do NOT unregister -- the new connection owns the registry entry.
                return;
            }
        }
    }

    // Cleanup: unregister unless superseded.
    if !session.cancel_token.is_cancelled() {
        if session.is_external_scheduler {
            // Unregister first so has_capability_connected excludes this service.
            state.service_connections.unregister(&service_id).await;
            if !state
                .service_connections
                .has_capability_connected(&Capability::Scheduler)
                .await
            {
                state
                    .external_scheduler_connected
                    .store(false, Ordering::Relaxed);
                tracing::info!(
                    %service_id,
                    "external scheduler disconnected (enrolled); embedded scheduler resuming all tasks"
                );
            }
        } else {
            state.service_connections.unregister(&service_id).await;
        }
    }
    tracing::debug!(%service_id, "enrolled service disconnected");
}
