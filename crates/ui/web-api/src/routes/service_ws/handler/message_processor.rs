//! Background message processor and channel setup for the WebSocket handler.
//!
//! Extracted from `mod.rs` to keep the main handler file focused on the
//! session lifecycle (setup, loop, cleanup) rather than dispatch logic.

#![expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]
#![expect(
    clippy::unreachable,
    reason = "branch is statically unreachable given surrounding protocol invariants"
)]

use std::collections::HashSet;
use std::sync::Arc;

use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::limits::WireValidate;
use uptrakit_wire::report_tracker::ReportTracker;
use uptrakit_wire::{ControllerMessage, ErrorCode, ErrorPayload, ReportPagination, ServiceMessage};

use super::super::protocol::CertIdentity;
use super::session_enrolled::upgrade_service_capabilities;
use super::shared_types::{
    self, ProcessorAction, ProcessorResponse, is_valid_service_config_scope,
};
use super::{
    audit_service, audit_surface, messages, service_config, surface_wire, update_tracking, updates,
    workload,
};
use crate::AppState;

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
// ProcessorMessage
// ---------------------------------------------------------------------------

/// A deserialized service message forwarded to the background processor.
pub(super) struct ProcessorMessage {
    pub(super) message: ServiceMessage,
    pub(super) pagination: Option<ReportPagination>,
}

// ---------------------------------------------------------------------------
// MessageProcessor
// ---------------------------------------------------------------------------

/// Background message processor spawned per WebSocket connection.
///
/// Receives deserialized [`ServiceMessage`] values via an MPSC channel,
/// dispatches them to the appropriate handler, and sends
/// [`ProcessorResponse`] values back to the main loop.
pub(super) struct MessageProcessor {
    pub(super) state: Arc<AppState>,
    pub(super) service_id: uuid::Uuid,
    pub(super) cert: Option<CertIdentity>,
    pub(super) is_system: bool,
    pub(super) has_update_tracking: bool,
    pub(super) has_software_discovery: bool,
    pub(super) has_update_hooks: bool,
    pub(super) has_ui_surfaces: bool,
    pub(super) has_workload_claims: bool,
    pub(super) runtime_instance_id: Option<uuid::Uuid>,
    pub(super) service_app_name: Option<String>,
    pub(super) service_tenant_id: Option<uuid::Uuid>,
    pub(super) linked_host_ids: Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    pub(super) report_tracker: ReportTracker,
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
    pub(super) async fn dispatch(
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
                if self.has_software_discovery && !self.has_update_tracking =>
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
                if self.has_update_tracking =>
            {
                self.dispatch_update_tracking(msg).await
            }

            // -- Shared surfaces runtime (parallel migration path) --
            msg @ (ServiceMessage::SurfaceRegistration(_)
            | ServiceMessage::SurfaceActionResponse(_)
            | ServiceMessage::SurfaceActionRequest(_))
                if self.has_ui_surfaces =>
            {
                self.dispatch_surfaces(msg).await
            }

            // -- WorkloadClaims capability --
            ServiceMessage::WorkloadClaim(payload) if self.has_workload_claims => {
                workload::handle_workload_claim(&self.state, self.service_id, payload).await
            }
            ServiceMessage::WorkloadRelease(payload) if self.has_workload_claims => {
                workload::handle_workload_release(&self.state, self.service_id, payload).await
            }

            // -- Universal messages (all capabilities) --
            ServiceMessage::AuditEvent(payload) => {
                let _ = audit_service::ingest_service_audit_event(
                    &self.state,
                    self.service_id,
                    self.is_system,
                    self.service_tenant_id,
                    self.service_app_name.as_deref(),
                    payload,
                )
                .await;
                ProcessorResponse::cont()
            }
            ServiceMessage::RenewCertificate(payload) => {
                if let Some(ref cert) = self.cert {
                    messages::handle_renew_certificate(
                        &self.state,
                        self.service_id,
                        cert,
                        &payload,
                        self.is_system,
                    )
                    .await
                } else {
                    // Embedded services do not use certificates.
                    ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                        code: ErrorCode::BadRequest,
                        message: "certificate renewal not supported for embedded services"
                            .to_string(),
                    }))
                }
            }
            ServiceMessage::TestPluginConfigResult(payload) => {
                let request_id = payload.request_id.clone();
                self.state.config_test_proxy.complete(&request_id, payload);
                ProcessorResponse::cont()
            }
            ServiceMessage::ReportPluginConfig(payload) => {
                messages::handle_report_plugin_config(&self.state, self.service_id, &payload).await
            }
            ServiceMessage::StoreServiceConfig(payload) => {
                if !is_valid_service_config_scope(self.service_tenant_id, payload.tenant_id) {
                    service_config::emit_service_config_scope_denied_audit_event(
                        service_config::ServiceConfigAuditCtx {
                            state: &self.state,
                            action_type: uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_STORE,
                            service_id: self.service_id,
                            service_app_name: self.service_app_name.as_deref().unwrap_or(""),
                        },
                        self.service_tenant_id
                            .expect("service config scope denial requires tenant binding"),
                        payload.tenant_id,
                        &payload.key,
                        &payload.request_id,
                        "outside_tenant_binding",
                    );
                    return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                        uptrakit_wire::ServiceConfigAckPayload::error(
                            payload.request_id,
                            "service cannot write config outside its tenant binding".to_string(),
                        ),
                    ));
                }
                service_config::handle_store_service_config(
                    &self.state,
                    self.service_app_name.as_deref().unwrap_or(""),
                    self.service_id,
                    payload,
                )
                .await
            }
            ServiceMessage::DeleteServiceConfig(payload) => {
                if !is_valid_service_config_scope(self.service_tenant_id, payload.tenant_id) {
                    service_config::emit_service_config_scope_denied_audit_event(
                        service_config::ServiceConfigAuditCtx {
                            state: &self.state,
                            action_type: uptrakit_audit_log::AuditActionType::SERVICE_CONFIG_DELETE,
                            service_id: self.service_id,
                            service_app_name: self.service_app_name.as_deref().unwrap_or(""),
                        },
                        self.service_tenant_id
                            .expect("service config scope denial requires tenant binding"),
                        payload.tenant_id,
                        &payload.key,
                        &payload.request_id,
                        "outside_tenant_binding",
                    );
                    return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                        uptrakit_wire::ServiceConfigAckPayload::error(
                            payload.request_id,
                            "service cannot delete config outside its tenant binding".to_string(),
                        ),
                    ));
                }
                service_config::handle_delete_service_config(
                    &self.state,
                    self.service_app_name.as_deref().unwrap_or(""),
                    self.service_id,
                    payload,
                )
                .await
            }

            // -- Register: embedded services send this on startup to declare capabilities --
            ServiceMessage::Register(payload) => {
                self.runtime_instance_id = payload.runtime_instance_id;
                upgrade_service_capabilities(
                    self.state.db(),
                    self.service_id,
                    self.is_system,
                    payload.capabilities,
                    &mut self.has_ui_surfaces,
                )
                .await;

                if let Err(error) = updates::recover_owned_updates_on_connect_with_dispatch_mode(
                    &self.state,
                    self.service_id,
                    self.runtime_instance_id,
                    updates::ReconnectSuccessorDispatchMode::Immediate,
                )
                .await
                {
                    tracing::warn!(
                        error = %error,
                        %self.service_id,
                        "embedded reconnect recovery failed"
                    );
                }

                ProcessorResponse::cont()
            }

            // -- Disconnecting: embedded services send this during shutdown --
            ServiceMessage::Disconnecting(_) => {
                tracing::debug!(
                    service_id = %self.service_id,
                    "embedded service sent Disconnecting"
                );
                ProcessorResponse {
                    replies: Vec::new(),
                    action: ProcessorAction::Break,
                }
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
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::UpdateOutput(payload) => {
                updates::handle_update_output(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::UpdateResult(payload) => {
                updates::handle_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::BatchUpdateResult(payload) => {
                updates::handle_batch_update_result(
                    &self.state,
                    self.service_id,
                    payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            ServiceMessage::StdinAttention(payload) => {
                updates::handle_stdin_attention(
                    &self.state,
                    self.service_id,
                    &payload,
                    &self.linked_host_ids,
                    self.runtime_instance_id,
                )
                .await
            }
            _ => unreachable!("dispatch_update_hooks called with non-update message"),
        }
    }

    /// Dispatch update-tracking messages (ServiceTriggerUpdate, etc.).
    async fn dispatch_update_tracking(&self, msg: ServiceMessage) -> ProcessorResponse {
        let service_app_name = self.service_app_name.as_deref().unwrap_or("unknown");
        match msg {
            ServiceMessage::ServiceTriggerUpdate(payload) => {
                update_tracking::handle_service_trigger_update(
                    &self.state,
                    service_app_name,
                    &payload,
                )
                .await
            }
            ServiceMessage::ServiceTriggerHostBatchUpdate(payload) => {
                update_tracking::handle_service_trigger_host_batch_update(
                    &self.state,
                    service_app_name,
                    &payload,
                )
                .await
            }
            _ => unreachable!("dispatch_update_tracking called with non-update-tracking message"),
        }
    }

    /// Dispatch surface runtime messages (SurfaceRegistration, SurfaceActionResponse, etc.).
    async fn dispatch_surfaces(&mut self, msg: ServiceMessage) -> ProcessorResponse {
        match msg {
            ServiceMessage::SurfaceRegistration(payload) => {
                self.handle_surface_registration(payload).await
            }
            ServiceMessage::SurfaceActionResponse(payload) => {
                self.state
                    .surface_proxy_deps
                    .proxy
                    .complete(payload.request_id, payload);
                ProcessorResponse::cont()
            }
            ServiceMessage::SurfaceActionRequest(payload) => {
                self.handle_surface_action_request(payload).await
            }
            _ => unreachable!("dispatch_surfaces called with non-surface message"),
        }
    }

    /// Handle a `SurfaceRegistration` message: validate and register provider surfaces.
    pub(super) async fn handle_surface_registration(
        &self,
        payload: uptrakit_wire::surfaces::SurfaceRegistration,
    ) -> ProcessorResponse {
        if let Err(e) = payload.wire_validate() {
            audit_surface::emit_surface_registration_audit_event(
                &shared_types::ServiceAuditCtx {
                    state: &self.state,
                    service_id: self.service_id,
                    service_app_name: self.service_app_name.as_deref(),
                },
                self.is_system,
                self.service_tenant_id,
                &payload,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                Some(audit_surface::classify_surface_registration_validation_error(&e)),
            );
            tracing::warn!(
                service_id = %self.service_id,
                error = %e,
                "invalid SurfaceRegistration payload"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: format!("invalid surface registration: {e}"),
            }));
        }

        let app_name = self.service_app_name.as_deref().unwrap_or("unknown");
        if let Err(e) = surface_wire::register_surface_provider(
            self.state.surface_proxy_deps.registry.as_ref(),
            self.state.surface_proxy_deps.proxy.as_ref(),
            self.service_id,
            app_name,
            self.service_tenant_id,
            payload.clone(),
        ) {
            audit_surface::emit_surface_registration_audit_event(
                &shared_types::ServiceAuditCtx {
                    state: &self.state,
                    service_id: self.service_id,
                    service_app_name: self.service_app_name.as_deref(),
                },
                self.is_system,
                self.service_tenant_id,
                &payload,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some(audit_surface::classify_surface_registration_error_for_audit(&e)),
            );
            tracing::warn!(
                service_id = %self.service_id,
                app_name,
                error = %e,
                "surface registration rejected"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: surface_wire::surface_registration_error_message(&e),
            }));
        }

        tracing::info!(
            service_id = %self.service_id,
            app_name,
            "registered service surfaces"
        );
        audit_surface::emit_surface_registration_audit_event(
            &shared_types::ServiceAuditCtx {
                state: &self.state,
                service_id: self.service_id,
                service_app_name: self.service_app_name.as_deref(),
            },
            self.is_system,
            self.service_tenant_id,
            &payload,
            uptrakit_audit_log::AuditOutcome::Success,
            None,
        );
        if let Some(tenant_id) = self.service_tenant_id {
            self.state
                .notification
                .event_broadcaster
                .send(tenant_id, AdminEvent::SurfacesChanged)
                .await;
        }
        ProcessorResponse::cont()
    }

    /// Handle a `SurfaceActionRequest` message: service-initiated surface action invocation.
    pub(super) async fn handle_surface_action_request(
        &self,
        payload: uptrakit_wire::surfaces::SurfaceActionRequest,
    ) -> ProcessorResponse {
        let request_id = payload.request_id;

        if let Err(e) = payload.wire_validate() {
            tracing::warn!(
                service_id = %self.service_id,
                error = %e,
                "invalid SurfaceActionRequest payload"
            );
            if let Some(tenant_id) = audit_surface::resolve_surface_action_audit_tenant_id(
                self.service_tenant_id,
                &payload,
            ) {
                audit_surface::emit_surface_action_invoke_audit_event(
                    &shared_types::ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    tenant_id,
                    &payload,
                    None,
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    Some(audit_surface::classify_surface_action_request_validation_error(&e)),
                );
            }
            return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                uptrakit_wire::surfaces::SurfaceActionResponse {
                    request_id,
                    success: false,
                    result: None,
                    error: Some(uptrakit_wire::surfaces::SurfaceActionError {
                        code: uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
                        message: format!("invalid surface action request: {e}"),
                        details: None,
                    }),
                },
            ));
        }

        let request_tenant_id = match uuid::Uuid::parse_str(&payload.tenant_id) {
            Ok(tenant_id) => tenant_id,
            Err(error) => {
                if let Some(tenant_id) = self.service_tenant_id {
                    audit_surface::emit_surface_action_invoke_audit_event(
                        &shared_types::ServiceAuditCtx {
                            state: &self.state,
                            service_id: self.service_id,
                            service_app_name: self.service_app_name.as_deref(),
                        },
                        tenant_id,
                        &payload,
                        None,
                        uptrakit_audit_log::AuditOutcome::ValidationFailed,
                        Some("invalid_tenant_id"),
                    );
                }
                return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                    uptrakit_wire::surfaces::SurfaceActionResponse {
                        request_id,
                        success: false,
                        result: None,
                        error: Some(uptrakit_wire::surfaces::SurfaceActionError {
                            code: uptrakit_wire::surfaces::SurfaceActionErrorCode::InvalidRequest,
                            message: format!("invalid tenant_id: {error}"),
                            details: None,
                        }),
                    },
                ));
            }
        };

        if let Some(service_tenant_id) = self.service_tenant_id
            && service_tenant_id != request_tenant_id
        {
            audit_surface::emit_surface_action_scope_denied_audit_event(
                &self.state,
                self.service_id,
                self.service_app_name.as_deref(),
                service_tenant_id,
                &payload,
            )
            .await;
            return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                uptrakit_wire::surfaces::SurfaceActionResponse {
                    request_id,
                    success: false,
                    result: None,
                    error: Some(uptrakit_wire::surfaces::SurfaceActionError {
                        code: uptrakit_wire::surfaces::SurfaceActionErrorCode::PermissionDenied,
                        message: "service cannot invoke actions outside its tenant".to_string(),
                        details: None,
                    }),
                },
            ));
        }

        let invoke_request = crate::surface_proxy::SurfaceInvokeRequest::new(
            request_tenant_id,
            payload.surface_id.to_string(),
            payload.interaction_id.to_string(),
            payload.idempotency_key.clone(),
            payload.target_provider_id.clone(),
            crate::surface_proxy::SurfaceCallerOrigin::Provider {
                service_id: self.service_id,
            },
            payload.params.clone(),
            payload.encrypted_sensitive_params.clone(),
        );
        let resolved = match self
            .state
            .surface_proxy_deps
            .registry
            .resolve_surface_action(
                request_tenant_id,
                payload.surface_id.as_str(),
                payload.interaction_id.as_str(),
                payload.target_provider_id.as_deref(),
            ) {
            Ok(resolved) => Some(resolved),
            Err(error) => {
                let (outcome, reason_code) =
                    audit_surface::classify_surface_lookup_error_for_audit(&error);
                audit_surface::emit_surface_action_invoke_audit_event(
                    &shared_types::ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    request_tenant_id,
                    &payload,
                    None,
                    outcome,
                    Some(reason_code),
                );
                return ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(
                    uptrakit_wire::surfaces::SurfaceActionResponse {
                        request_id,
                        success: false,
                        result: None,
                        error: Some(surface_wire::surface_registry_lookup_error_to_wire(error)),
                    },
                ));
            }
        };

        let response = match self
            .state
            .surface_proxy_deps
            .proxy
            .invoke(
                &self.state.service_connections,
                &self.state.surface_proxy_deps.registry,
                invoke_request,
                None,
            )
            .await
        {
            Ok(mut response) => {
                let (outcome, reason_code) =
                    audit_surface::classify_surface_action_response_for_audit(&response);
                audit_surface::emit_surface_action_invoke_audit_event(
                    &shared_types::ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    request_tenant_id,
                    &payload,
                    resolved.as_ref(),
                    outcome,
                    reason_code,
                );
                response.request_id = request_id;
                response
            }
            Err(error) => {
                let (outcome, reason_code) =
                    audit_surface::classify_surface_proxy_error_for_audit(&error);
                audit_surface::emit_surface_action_invoke_audit_event(
                    &shared_types::ServiceAuditCtx {
                        state: &self.state,
                        service_id: self.service_id,
                        service_app_name: self.service_app_name.as_deref(),
                    },
                    request_tenant_id,
                    &payload,
                    resolved.as_ref(),
                    outcome,
                    Some(reason_code),
                );
                uptrakit_wire::surfaces::SurfaceActionResponse {
                    request_id,
                    success: false,
                    result: None,
                    error: Some(surface_wire::surface_proxy_error_to_wire(error)),
                }
            }
        };

        ProcessorResponse::reply(ControllerMessage::SurfaceActionResponse(response))
    }
}

// ---------------------------------------------------------------------------
// ProcessorChannels + spawn_message_processor
// ---------------------------------------------------------------------------

/// Output of [`spawn_message_processor`]: channels for communicating with the
/// background task.
pub(super) struct ProcessorChannels {
    pub(super) msg_tx: tokio::sync::mpsc::Sender<ProcessorMessage>,
    pub(super) resp_rx: tokio::sync::mpsc::Receiver<ProcessorResponse>,
    pub(super) processor_cancel: tokio_util::sync::CancellationToken,
    pub(super) processor_handle: tokio::task::JoinHandle<()>,
}

/// Stage 6: Spawn the background [`MessageProcessor`] and return the channels
/// the main loop needs to exchange messages with it.
pub(super) fn spawn_message_processor(processor: MessageProcessor) -> ProcessorChannels {
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
