//! Unified capability-gated WebSocket handler for all service types.
//!
//! This module replaces the three separate handlers (`agent_ws`, `mqtt_ws`,
//! `ssh_agent_ws`) with a single pair of handler functions that dispatch
//! messages based on the service's persisted capability set.
//!
//! # Public API
//!
//! - [`handle_authenticated_loop`] -- post-certificate operational loop.
//! - [`handle_enrolled_loop`] -- pre-certificate enrollment loop.
//! - [`trigger_discovery_for_agent_host`] -- send `DiscoverSoftware` to an
//!   agent for a specific host (also used by `hosts.rs`).

mod discovery;
mod messages;
mod mqtt;
mod renewal;
mod updates;

pub(crate) use discovery::trigger_discovery_for_agent_host;
use mqtt::handle_mqtt_register_phase;
use updates::{deliver_pending_updates, load_linked_host_ids};

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use thiserror::Error;

use rootcause::prelude::*;
use sea_orm::EntityTrait;

use uptrakit_internal_wire::{
    ApprovedPayload, Capability, CertificatePayload, CloseReason, ControllerMessage, ErrorCode,
    ErrorPayload, IncomingSeq, MqttRegisteredPayload, MqttTenantAssignmentsPayload, OutgoingSeq,
    PingPayload, RejectedPayload, ServiceCredentialsPayload, ServiceMessage,
};
use uptrakit_shared_db::entity::service;
use uptrakit_shared_macros::impl_report_conversion;

use super::protocol::{
    AuthenticatedContext, MessageRateLimiter, WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW,
    close_with_reason, deserialize_service_msg, record_service_activity, send_pong,
    serialize_controller_msg,
};
use crate::AppState;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;
use crate::routes::agents::do_sign_csr;
use uptrakit_internal_wire::service_profile::parse_capabilities;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum size of the `update_history.output` column (1 MB).
///
/// Once the output reaches this limit, further `UpdateOutput` messages are
/// silently dropped to prevent unbounded DB growth.
const MAX_UPDATE_OUTPUT_BYTES: usize = 1_048_576;

/// Interval between approval-status DB polls in enrolled loops.
const APPROVAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

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
// handle_authenticated_loop
// ---------------------------------------------------------------------------

/// Unified authenticated handler for all service types.
///
/// Called by [`super::service_ws`] after certificate validation, service status
/// check, and sending `ServiceSettings`. Dispatches incoming messages based on
/// the service's capability set.
pub(crate) async fn handle_authenticated_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    ctx: AuthenticatedContext<'_>,
) {
    let AuthenticatedContext {
        service_id,
        cert,
        out_seq,
        in_seq,
    } = ctx;

    // Load service from DB, derive capabilities.
    let capabilities: BTreeSet<Capability> = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => parse_capabilities(&svc.capabilities),
        _ => BTreeSet::new(),
    };

    let is_mqtt = capabilities.contains(&Capability::MqttBridge);
    let has_software_discovery = capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = capabilities.contains(&Capability::UpdateHooks);

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
                return;
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
    // MQTT pre-loop phase: wait for Register, set up leases
    // ------------------------------------------------------------------
    let mqtt_context = if is_mqtt {
        match handle_mqtt_register_phase(
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
            Some(ctx) => Some(ctx),
            None => return, // registration failed or connection closed
        }
    } else {
        None
    };

    // ------------------------------------------------------------------
    // Register in service_connections
    // ------------------------------------------------------------------
    let (mut push_rx, cancel_token) = if let Some(ref mctx) = mqtt_context {
        state
            .service_connections
            .register(
                service_id,
                capabilities.clone(),
                Some(mctx.instance_id.clone()),
                Some(mctx.max_tenants),
            )
            .await
    } else {
        state
            .service_connections
            .register(service_id, capabilities.clone(), None, None)
            .await
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
            return;
        };
        if sink.send(Message::Text(json.into())).await.is_err() {
            state.service_connections.unregister(&service_id).await;
            return;
        }

        // Send initial tenant assignments.
        if !mctx.tenant_configs.is_empty() {
            let assignments_msg =
                ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload {
                    tenants: mctx.tenant_configs.clone(),
                });
            let Some(json) = serialize_controller_msg(out_seq, assignments_msg) else {
                state.service_connections.unregister(&service_id).await;
                return;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                state.service_connections.unregister(&service_id).await;
                return;
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

        tracing::info!(
            %service_id,
            instance_id = %mctx.instance_id,
            "MQTT service registered"
        );
    }

    // ------------------------------------------------------------------
    // SoftwareDiscovery: load linked host IDs
    // ------------------------------------------------------------------
    let mut linked_host_ids: HashSet<uuid::Uuid> = if has_software_discovery {
        load_linked_host_ids(state.db(), service_id)
            .await
            .unwrap_or_default()
    } else {
        HashSet::new()
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
    // Main operational loop
    // ------------------------------------------------------------------
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
                if !rate_limiter.allow() {
                    let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                    break;
                }
                match msg {
                    Message::Text(text) => {
                        let service_msg: ServiceMessage =
                            match deserialize_service_msg(in_seq, &text) {
                                Ok(Some(m)) => m,
                                Ok(None) => continue,
                                Err(e) => {
                                    tracing::debug!(error = %e, "deserialize error");
                                    break;
                                }
                            };

                        match service_msg {
                            // -------------------------------------------------
                            // Ping (all capabilities)
                            // -------------------------------------------------
                            ServiceMessage::Ping(PingPayload { service_ts }) => {
                                if messages::handle_ping(sink, out_seq, state, service_id, service_ts, lease_coordinator.as_ref()).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // RenewCertificate (all capabilities)
                            // -------------------------------------------------
                            ServiceMessage::RenewCertificate(payload) => {
                                if messages::handle_renew_certificate(sink, out_seq, state, service_id, &cert, &payload).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // ReportHosts (requires SoftwareDiscovery)
                            // -------------------------------------------------
                            ServiceMessage::ReportHosts(payload) if has_software_discovery => {
                                if messages::handle_report_hosts(sink, out_seq, state, service_id, &payload, &mut linked_host_ids).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // VersionCheckResults (SoftwareDiscovery AND NOT MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::VersionCheckResults(payload)
                                if has_software_discovery && !is_mqtt =>
                            {
                                if messages::handle_version_check_results(state, service_id, &payload).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // UpdateStarted (requires UpdateHooks)
                            // -------------------------------------------------
                            ServiceMessage::UpdateStarted(payload) if has_update_hooks => {
                                if updates::handle_update_started(state, service_id, &payload, &linked_host_ids).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // UpdateOutput (requires UpdateHooks)
                            // -------------------------------------------------
                            ServiceMessage::UpdateOutput(payload) if has_update_hooks => {
                                if updates::handle_update_output(state, service_id, &payload, &linked_host_ids).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // UpdateResult (requires UpdateHooks)
                            // -------------------------------------------------
                            ServiceMessage::UpdateResult(payload) if has_update_hooks => {
                                if updates::handle_update_result(state, service_id, payload, &linked_host_ids).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // DiscoveryResults (requires SoftwareDiscovery)
                            // -------------------------------------------------
                            ServiceMessage::DiscoveryResults(payload)
                                if has_software_discovery =>
                            {
                                if messages::handle_discovery_results(state, service_id, payload).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // ReleaseTenants (requires MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::ReleaseTenants(payload) if is_mqtt => {
                                if mqtt::handle_release_tenants(state, service_id, &payload, lease_coordinator.as_ref()).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // MqttClientStatus (requires MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::MqttClientStatus(payload) if is_mqtt => {
                                if mqtt::handle_mqtt_client_status(state, &payload).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // MqttTriggerUpdate (requires MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::MqttTriggerUpdate(payload) if is_mqtt => {
                                if mqtt::handle_mqtt_trigger_update(sink, out_seq, state, &payload, mqtt_context.as_ref()).await.is_break() {
                                    break;
                                }
                            }

                            // -------------------------------------------------
                            // Disconnecting (all capabilities)
                            // -------------------------------------------------
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "service disconnecting gracefully"
                                );
                                break;
                            }

                            // -------------------------------------------------
                            // Wildcard: message not supported for this capability
                            // -------------------------------------------------
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message:
                                        "message not supported for this service capability"
                                            .to_string(),
                                });
                                if let Some(json) =
                                    serialize_controller_msg(out_seq, err)
                                {
                                    let _ =
                                        sink.send(Message::Text(json.into())).await;
                                }
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            push = push_rx.recv() => {
                let Some(msg) = push else { break };
                let Some(json) = serialize_controller_msg(out_seq, msg) else { break };
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            _ = cancel_token.cancelled() => {
                tracing::info!(%service_id, "connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Do NOT unregister -- the new connection owns the registry entry.
                // Release MQTT leases if applicable (new connection will re-reconcile).
                if let Some(ref lc) = lease_coordinator
                    && let Err(e) = lc.release_all_for_service(&service_id).await
                {
                    tracing::error!(error = %e, "failed to release leases on superseded disconnect");
                }
                return;
            }
        }
    }

    // ------------------------------------------------------------------
    // Cleanup
    // ------------------------------------------------------------------
    if let Some(ref lc) = lease_coordinator
        && let Err(e) = lc.release_all_for_service(&service_id).await
    {
        tracing::error!(error = %e, "failed to release leases on disconnect");
    }

    state.service_connections.unregister(&service_id).await;
    tracing::debug!(%service_id, "authenticated service disconnected");
}

// ---------------------------------------------------------------------------
// handle_enrolled_loop
// ---------------------------------------------------------------------------

/// Unified enrolled handler for all service types.
///
/// Handles Ping, RequestCertificate, and polls for approval changes at a
/// fixed interval (decoupled from client-controlled ping frequency).
pub(crate) async fn handle_enrolled_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    // Fetch service to derive capabilities for registration.
    let capabilities: BTreeSet<Capability> = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => parse_capabilities(&svc.capabilities),
        _ => BTreeSet::new(),
    };

    // Register in service_connections.
    let (mut push_rx, cancel_token) = state
        .service_connections
        .register(service_id, capabilities, None, None)
        .await;

    // Check current status to set initial approved flag.
    let mut approved = false;
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        && svc.status == service::ServiceStatus::Approved
    {
        approved = true;
    }

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Dedicated interval for polling approval status from the DB.
    let mut approval_poll = tokio::time::interval(APPROVAL_POLL_INTERVAL);
    approval_poll.tick().await; // skip immediate first tick

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
                if !rate_limiter.allow() {
                    let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                    break;
                }

                match msg {
                    Message::Text(text) => {
                        let service_msg: ServiceMessage =
                            match deserialize_service_msg(in_seq, &text) {
                                Ok(Some(m)) => m,
                                Ok(None) => continue,
                                Err(e) => {
                                    tracing::debug!(error = %e, "deserialize error");
                                    break;
                                }
                            };

                        match service_msg {
                            ServiceMessage::Ping(PingPayload { service_ts }) => {
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
                                if let Err(e) =
                                    record_service_activity(state.db(), service_id, None).await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        %service_id,
                                        "failed to record service activity"
                                    );
                                }
                            }
                            ServiceMessage::RequestCertificate(payload) => {
                                if !approved {
                                    let err = ControllerMessage::Error(ErrorPayload {
                                        code: ErrorCode::NotApproved,
                                        message: "service is not yet approved".to_string(),
                                    });
                                    if let Some(json) =
                                        serialize_controller_msg(out_seq, err)
                                    {
                                        let _ =
                                            sink.send(Message::Text(json.into())).await;
                                    }
                                    continue;
                                }

                                // Re-fetch service from DB.
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
                                        if let Some(json) =
                                            serialize_controller_msg(out_seq, err)
                                        {
                                            let _ =
                                                sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                };

                                // Use do_sign_csr (invalidates enrollment secret -- correct
                                // for initial certificate issuance during enrollment).
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
                                        let cert_msg = ControllerMessage::Certificate(
                                            CertificatePayload {
                                                cert_pem: bundle.cert_pem,
                                                not_after: bundle.not_after,
                                            },
                                        );
                                        if let Some(json) =
                                            serialize_controller_msg(out_seq, cert_msg)
                                        {
                                            let _ =
                                                sink.send(Message::Text(json.into())).await;
                                        }
                                        tracing::info!(
                                            %service_id,
                                            "certificate issued via WS"
                                        );
                                        break; // close connection after certificate issuance
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::CertificateError,
                                            message: e.current_context().to_string(),
                                        });
                                        if let Some(json) =
                                            serialize_controller_msg(out_seq, err)
                                        {
                                            let _ =
                                                sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
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
            push = push_rx.recv() => {
                let Some(msg) = push else { break };

                // Track state transitions; handle Rejected specially (send + break).
                let is_rejected = matches!(&msg, ControllerMessage::Rejected(_));
                if matches!(&msg, ControllerMessage::Approved(_)) {
                    approved = true;
                }

                let Some(json) = serialize_controller_msg(out_seq, msg) else { break };
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
                if is_rejected {
                    break;
                }
            }
            // Dedicated approval poll at a fixed interval.
            _ = approval_poll.tick(), if !approved => {
                if let Ok(Some(s)) = service::Entity::find_by_id(service_id)
                    .one(state.db())
                    .await
                {
                    match s.status {
                        service::ServiceStatus::Approved => {
                            approved = true;
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
            _ = cancel_token.cancelled() => {
                tracing::info!(%service_id, "enrolled connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Do NOT unregister -- the new connection owns the registry entry.
                return;
            }
        }
    }

    // Cleanup: unregister unless superseded.
    if !cancel_token.is_cancelled() {
        state.service_connections.unregister(&service_id).await;
    }
    tracing::debug!(%service_id, "enrolled service disconnected");
}
