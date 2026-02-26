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
mod mqtt;
mod renewal;
mod updates;

pub(crate) use discovery::trigger_discovery_for_agent_host;
use mqtt::handle_mqtt_register_phase;
use renewal::sign_renewal_csr;
use updates::{
    deliver_pending_updates, load_linked_host_ids, upsert_available_version,
    validate_update_ownership,
};

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use sea_orm::sea_query::{Expr, ExprTrait};
use thiserror::Error;

use rootcause::prelude::*;
use uptrakit_internal_wire::{
    ApprovedPayload, Capability, CertificatePayload, CloseReason, ControllerMessage,
    ErrorCode, ErrorPayload, IncomingSeq,
    MqttClientConnectionStatus as WireMqttClientConnectionStatus,
    MqttRegisteredPayload, MqttTenantAssignmentsPayload, OutgoingSeq,
    PingPayload, RejectedPayload, ServiceMessage, UpdateFinalStatus,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, service, service_host, software_item, update_history,
    update_output_line,
};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::settings_mqtt::MqttClientConnectionStatus as ApiMqttClientConnectionStatus;

use super::protocol::{
    AuthenticatedContext, MessageRateLimiter, WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW,
    close_with_reason, deserialize_service_msg, record_service_activity, send_pong,
    serialize_controller_msg,
};
use crate::AppState;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;
use crate::routes::agents::{do_sign_csr, find_or_create_host_and_link, revoke_certificate};
use crate::service_profile::parse_capabilities;

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
        last_seen_at,
        out_seq,
        in_seq,
    } = ctx;

    // Load service from DB, derive capabilities.
    let capabilities: BTreeSet<Capability> =
        match service::Entity::find_by_id(service_id).one(state.db()).await {
            Ok(Some(svc)) => parse_capabilities(&svc.capabilities),
            _ => BTreeSet::new(),
        };

    let is_mqtt = capabilities.contains(&Capability::MqttBridge);
    let has_software_discovery = capabilities.contains(&Capability::SoftwareDiscovery);
    let has_update_hooks = capabilities.contains(&Capability::UpdateHooks);

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

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
                        .push_software_states_for_tenant(cfg.tenant_id)
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
    // Deliver notification backlog
    // ------------------------------------------------------------------
    let delivered = state
        .notification_service
        .deliver_backlog_for_authenticated_service(service_id, &capabilities, last_seen_at)
        .await;
    if delivered > 0 {
        tracing::info!(%service_id, delivered, "delivered outbox backlog to service");
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
    if has_update_hooks && !is_mqtt
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
                                let Ok(controller_ts) =
                                    send_pong(sink, out_seq, service_ts).await
                                else {
                                    break;
                                };
                                tracing::trace!(service_ts, controller_ts, "ping/pong");
                                if let Err(e) =
                                    record_service_activity(state.db(), service_id, None).await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        %service_id,
                                        "failed to record service activity"
                                    );
                                }
                                // MQTT heartbeat
                                if let Some(ref lc) = lease_coordinator
                                    && let Err(e) = lc.record_heartbeat(&service_id).await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to record heartbeat"
                                    );
                                }
                            }

                            // -------------------------------------------------
                            // RenewCertificate (all capabilities)
                            // -------------------------------------------------
                            ServiceMessage::RenewCertificate(payload) => {
                                // Re-fetch service, verify still approved.
                                let svc = match service::Entity::find_by_id(service_id)
                                    .one(state.db())
                                    .await
                                {
                                    Ok(Some(s))
                                        if s.status == service::ServiceStatus::Approved
                                            && s.deactivated_at.is_none() =>
                                    {
                                        s
                                    }
                                    _ => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::Forbidden,
                                            message: "service is not approved".to_string(),
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

                                match sign_renewal_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    state.db(),
                                    svc,
                                    &payload.csr_pem,
                                )
                                .await
                                {
                                    Ok(bundle) => {
                                        let cert_msg =
                                            ControllerMessage::Certificate(CertificatePayload {
                                                cert_pem: bundle.cert_pem,
                                                not_after: bundle.not_after,
                                            });
                                        if let Some(json) =
                                            serialize_controller_msg(out_seq, cert_msg)
                                        {
                                            let _ =
                                                sink.send(Message::Text(json.into())).await;
                                        }

                                        // Revoke old certificate.
                                        if let Err(e) = revoke_certificate(
                                            state.db(),
                                            &cert.serial,
                                            &cert.ca_fingerprint,
                                            uptrakit_shared_db::entity::prelude::RevocationReason::CertificateRenewed,
                                        )
                                        .await
                                        {
                                            tracing::error!(
                                                error = %e,
                                                "failed to revoke old certificate"
                                            );
                                        }

                                        if let Err(e) =
                                            crate::settings_store::bump_revocation_version(
                                                state.db(),
                                                state.default_tenant_id,
                                            )
                                            .await
                                        {
                                            tracing::warn!(
                                                error = ?e,
                                                "failed to bump revocation version counter"
                                            );
                                        }
                                        state.revocation_notify.notify_one();
                                        tracing::info!(
                                            %service_id,
                                            old_serial = %cert.serial,
                                            "certificate renewed, old cert revoked"
                                        );
                                        let _ = close_with_reason(
                                            sink,
                                            CloseReason::CertificateRotated,
                                        )
                                        .await;
                                        break;
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::CertificateError,
                                            message: e.to_string(),
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

                            // -------------------------------------------------
                            // ReportHosts (requires SoftwareDiscovery)
                            // -------------------------------------------------
                            ServiceMessage::ReportHosts(payload) if has_software_discovery => {
                                tracing::debug!(
                                    %service_id,
                                    capabilities = ?payload.capabilities,
                                    "received ReportHosts"
                                );

                                let service_model =
                                    match service::Entity::find_by_id(service_id)
                                        .one(state.db())
                                        .await
                                    {
                                        Ok(Some(s)) => s,
                                        _ => continue,
                                    };

                                // Update client_version.
                                let mut active: service::ActiveModel =
                                    service_model.clone().into();
                                active.client_version =
                                    Set(Some(payload.agent_version.clone()));
                                active.updated_at =
                                    Set(time::OffsetDateTime::now_utc());
                                if let Err(e) = active.update(state.db()).await {
                                    tracing::error!(
                                        error = %e,
                                        "failed to update client_version"
                                    );
                                }

                                for host_info in &payload.hosts {
                                    let host_hostname = host_info
                                        .hostname
                                        .as_deref()
                                        .unwrap_or(&service_model.hostname);
                                    let host_ip = host_info
                                        .ip_address
                                        .as_deref()
                                        .or(service_model.ip_address.as_deref());
                                    match find_or_create_host_and_link(
                                        state.db(),
                                        service_model.tenant_id,
                                        service_id,
                                        host_info,
                                        host_hostname,
                                        host_ip,
                                    )
                                    .await
                                    {
                                        Ok(Some((_host_id, true))) => {
                                            // New host -- trigger autodiscovery.
                                            trigger_discovery_for_agent_host(
                                                state,
                                                service_id,
                                                service_model.tenant_id,
                                                &host_info.machine_id,
                                            )
                                            .await;
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                machine_id = %host_info.machine_id,
                                                "failed to link host"
                                            );
                                        }
                                    }
                                }

                                // Refresh cached host IDs.
                                if let Ok(ids) =
                                    load_linked_host_ids(state.db(), service_id).await
                                {
                                    linked_host_ids = ids;
                                }
                            }

                            // -------------------------------------------------
                            // VersionCheckResults (SoftwareDiscovery AND NOT MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::VersionCheckResults(payload)
                                if has_software_discovery && !is_mqtt =>
                            {
                                tracing::debug!(
                                    %service_id,
                                    count = payload.results.len(),
                                    "received VersionCheckResults"
                                );

                                let host_ids: Vec<uuid::Uuid> =
                                    match service_host::Entity::find()
                                        .filter(
                                            service_host::Column::ServiceId.eq(service_id),
                                        )
                                        .all(state.db())
                                        .await
                                    {
                                        Ok(links) => {
                                            links.into_iter().map(|l| l.host_id).collect()
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "failed to look up service hosts"
                                            );
                                            continue;
                                        }
                                    };

                                if host_ids.is_empty() {
                                    tracing::debug!(
                                        %service_id,
                                        "no hosts linked, skipping version updates"
                                    );
                                    continue;
                                }

                                let now = time::OffsetDateTime::now_utc();

                                for result in &payload.results {
                                    if result.error.is_some() {
                                        tracing::debug!(
                                            software_item_id = %result.software_item_id,
                                            error = ?result.error,
                                            "skipping version result with error"
                                        );
                                        continue;
                                    }

                                    let software_item_id = result.software_item_id;

                                    // Update installed version on host_software_item records.
                                    if let Some(ref installed_version) =
                                        result.installed_version
                                    {
                                        for &host_id in &host_ids {
                                            match host_software_item::Entity::find_by_id((
                                                host_id,
                                                software_item_id,
                                            ))
                                            .one(state.db())
                                            .await
                                            {
                                                Ok(Some(existing)) => {
                                                    let mut active: host_software_item::ActiveModel = existing.into();
                                                    active.installed_version = Set(Some(
                                                        installed_version.clone(),
                                                    ));
                                                    active
                                                        .installed_version_detected_at =
                                                        Set(Some(now));
                                                    if let Err(e) =
                                                        active.update(state.db()).await
                                                    {
                                                        tracing::warn!(
                                                            error = %e,
                                                            host_id = %host_id,
                                                            software_item_id = %software_item_id,
                                                            "failed to update host_software_item"
                                                        );
                                                    }
                                                }
                                                Ok(None) => {
                                                    tracing::debug!(
                                                        host_id = %host_id,
                                                        software_item_id = %software_item_id,
                                                        "no host_software_item record found, skipping"
                                                    );
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        error = %e,
                                                        host_id = %host_id,
                                                        software_item_id = %software_item_id,
                                                        "failed to look up host_software_item"
                                                    );
                                                }
                                            }
                                        }
                                    }

                                    // Upsert available_version if agent reported latest.
                                    if let Some(ref latest_version) = result.latest_version {
                                        upsert_available_version(
                                            state.db(),
                                            software_item_id,
                                            latest_version,
                                            now,
                                        )
                                        .await;
                                    }
                                }

                                // Batch-update last_checked_at for successful results.
                                let checked_ids: Vec<uuid::Uuid> = payload
                                    .results
                                    .iter()
                                    .filter(|r| r.error.is_none())
                                    .map(|r| r.software_item_id)
                                    .collect::<HashSet<_>>()
                                    .into_iter()
                                    .collect();

                                if !checked_ids.is_empty()
                                    && let Err(e) = software_item::Entity::update_many()
                                        .filter(
                                            software_item::Column::Id.is_in(checked_ids),
                                        )
                                        .col_expr(
                                            software_item::Column::LastCheckedAt,
                                            Expr::value(now),
                                        )
                                        .exec(state.db())
                                        .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to batch-update software_item last_checked_at"
                                    );
                                }

                                // Push updated software states to MQTT services.
                                if let Ok(Some(svc)) =
                                    service::Entity::find_by_id(service_id)
                                        .one(state.db())
                                        .await
                                {
                                    state
                                        .notification_service
                                        .push_software_states_for_tenant(svc.tenant_id)
                                        .await;
                                }
                            }

                            // -------------------------------------------------
                            // UpdateStarted (requires UpdateHooks)
                            // -------------------------------------------------
                            ServiceMessage::UpdateStarted(payload) if has_update_hooks => {
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    from_version = ?payload.from_version,
                                    "update started"
                                );
                                let record = match validate_update_ownership(
                                    state.db(),
                                    service_id,
                                    payload.update_history_id,
                                    &linked_host_ids,
                                )
                                .await
                                {
                                    Ok(r) => r,
                                    Err(_) => continue,
                                };
                                let mut active: update_history::ActiveModel = record.into();
                                active.status =
                                    Set(update_history::UpdateStatus::InProgress);
                                active.started_at =
                                    Set(time::OffsetDateTime::now_utc());
                                if payload.from_version.is_some() {
                                    active.from_version = Set(payload.from_version);
                                }
                                active.output = Set(String::new());
                                active.output_bytes = Set(0);
                                if let Err(e) = active.update(state.db()).await {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to update update_history status"
                                    );
                                }
                                if let Err(e) =
                                    update_output_line::Entity::delete_many()
                                        .filter(
                                            update_output_line::Column::UpdateHistoryId
                                                .eq(payload.update_history_id),
                                        )
                                        .exec(state.db())
                                        .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to clear update output lines"
                                    );
                                }
                            }

                            // -------------------------------------------------
                            // UpdateOutput (requires UpdateHooks)
                            // -------------------------------------------------
                            ServiceMessage::UpdateOutput(payload) if has_update_hooks => {
                                tracing::trace!(
                                    update_id = %payload.update_history_id,
                                    stream = ?payload.stream,
                                    "update output"
                                );
                                if validate_update_ownership(
                                    state.db(),
                                    service_id,
                                    payload.update_history_id,
                                    &linked_host_ids,
                                )
                                .await
                                .is_err()
                                {
                                    continue;
                                }

                                let output_line = format!("{}\n", payload.output);
                                let line_len = output_line.len() as i64;
                                let updated = update_history::Entity::update_many()
                                    .col_expr(
                                        update_history::Column::OutputBytes,
                                        Expr::col(update_history::Column::OutputBytes)
                                            .add(line_len),
                                    )
                                    .filter(
                                        update_history::Column::Id
                                            .eq(payload.update_history_id),
                                    )
                                    .filter(
                                        update_history::Column::OutputBytes
                                            .lt(MAX_UPDATE_OUTPUT_BYTES as i64),
                                    )
                                    .exec(state.db())
                                    .await;

                                let Ok(updated) = updated else {
                                    tracing::warn!(
                                        update_id = %payload.update_history_id,
                                        "failed to update output bytes"
                                    );
                                    continue;
                                };

                                if updated.rows_affected == 0 {
                                    tracing::debug!(
                                        update_id = %payload.update_history_id,
                                        "update output exceeded {MAX_UPDATE_OUTPUT_BYTES} byte cap, dropping"
                                    );
                                    continue;
                                }

                                let line = update_output_line::ActiveModel {
                                    id: Set(uuid::Uuid::now_v7()),
                                    update_history_id: Set(payload.update_history_id),
                                    stream: Set(payload.stream),
                                    output: Set(output_line),
                                    created_at: Set(time::OffsetDateTime::now_utc()),
                                };
                                if let Err(e) = update_output_line::Entity::insert(line)
                                    .exec(state.db())
                                    .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to insert update output line"
                                    );
                                }
                            }

                            // -------------------------------------------------
                            // UpdateResult (requires UpdateHooks)
                            // -------------------------------------------------
                            ServiceMessage::UpdateResult(payload) if has_update_hooks => {
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    status = ?payload.status,
                                    error = ?payload.error,
                                    "update result"
                                );
                                let record = match validate_update_ownership(
                                    state.db(),
                                    service_id,
                                    payload.update_history_id,
                                    &linked_host_ids,
                                )
                                .await
                                {
                                    Ok(r) => r,
                                    Err(_) => continue,
                                };
                                let mut active: update_history::ActiveModel =
                                    record.clone().into();
                                active.status = Set(match payload.status {
                                    UpdateFinalStatus::Completed => {
                                        update_history::UpdateStatus::Completed
                                    }
                                    UpdateFinalStatus::Failed => {
                                        update_history::UpdateStatus::Failed
                                    }
                                    _ => update_history::UpdateStatus::Failed,
                                });
                                active.completed_at =
                                    Set(Some(time::OffsetDateTime::now_utc()));
                                let capped_output =
                                    if payload.output.len() > MAX_UPDATE_OUTPUT_BYTES {
                                        payload.output[..MAX_UPDATE_OUTPUT_BYTES].to_string()
                                    } else {
                                        payload.output
                                    };
                                active.output = Set(capped_output.clone());
                                active.output_bytes = Set(capped_output.len() as i64);
                                if payload.from_version.is_some() {
                                    active.from_version = Set(payload.from_version);
                                }
                                if let Err(e) = active.update(state.db()).await {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to update update_history result"
                                    );
                                }

                                if let Err(e) = update_output_line::Entity::delete_many()
                                    .filter(
                                        update_output_line::Column::UpdateHistoryId
                                            .eq(payload.update_history_id),
                                    )
                                    .exec(state.db())
                                    .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to clear update output lines"
                                    );
                                }

                                if payload.status == UpdateFinalStatus::Completed
                                    && let Some(ref to_version) = payload.to_version
                                    && let Ok(Some(link)) = host_software_item::Entity::find_by_id((
                                        record.host_id,
                                        record.software_item_id,
                                    ))
                                    .one(state.db())
                                    .await
                                {
                                    let mut link_active: host_software_item::ActiveModel =
                                        link.into();
                                    link_active.installed_version =
                                        Set(Some(to_version.clone()));
                                    link_active.installed_version_detected_at =
                                        Set(Some(time::OffsetDateTime::now_utc()));
                                    link_active.last_updated_at =
                                        Set(Some(time::OffsetDateTime::now_utc()));
                                    if let Err(e) =
                                        link_active.update(state.db()).await
                                    {
                                        tracing::warn!(
                                            error = %e,
                                            "failed to update host_software_item installed_version"
                                        );
                                    }
                                }

                                // Push updated software states to MQTT services.
                                if let Ok(Some(svc)) =
                                    service::Entity::find_by_id(service_id)
                                        .one(state.db())
                                        .await
                                {
                                    state
                                        .notification_service
                                        .push_software_states_for_tenant(svc.tenant_id)
                                        .await;
                                }
                            }

                            // -------------------------------------------------
                            // DiscoveryResults (requires SoftwareDiscovery)
                            // -------------------------------------------------
                            ServiceMessage::DiscoveryResults(payload)
                                if has_software_discovery =>
                            {
                                tracing::debug!(
                                    %service_id,
                                    host_machine_id = %payload.host_machine_id,
                                    results = payload.results.len(),
                                    "received DiscoveryResults"
                                );

                                let links = service_host::Entity::find()
                                    .filter(
                                        service_host::Column::ServiceId.eq(service_id),
                                    )
                                    .all(state.db())
                                    .await
                                    .unwrap_or_default();

                                let mut host_id_opt: Option<uuid::Uuid> = None;
                                for link in &links {
                                    if let Ok(Some(h)) =
                                        host::Entity::find_by_id(link.host_id)
                                            .filter(
                                                host::Column::MachineId
                                                    .eq(&payload.host_machine_id),
                                            )
                                            .filter(
                                                host::Column::DeactivatedAt.is_null(),
                                            )
                                            .one(state.db())
                                            .await
                                    {
                                        host_id_opt = Some(h.id);
                                        break;
                                    }
                                }

                                if let Some(host_id) = host_id_opt {
                                    if let Ok(Some(svc)) =
                                        service::Entity::find_by_id(service_id)
                                            .one(state.db())
                                            .await
                                        && let Err(e) = crate::queries::autodiscovery::process_discovery_results(
                                            state.db(),
                                            service_id,
                                            svc.tenant_id,
                                            host_id,
                                            payload,
                                        )
                                        .await
                                    {
                                        tracing::warn!(
                                            error = %e,
                                            %service_id,
                                            "failed to process discovery results"
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        %service_id,
                                        host_machine_id = %payload.host_machine_id,
                                        "received DiscoveryResults for unknown host machine_id"
                                    );
                                }
                            }

                            // -------------------------------------------------
                            // ReleaseTenants (requires MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::ReleaseTenants(payload) if is_mqtt => {
                                if let Some(ref lc) = lease_coordinator
                                    && let Err(e) = lc
                                        .release_mqtt_clients(
                                            &service_id,
                                            &payload.mqtt_client_ids,
                                        )
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
                            }

                            // -------------------------------------------------
                            // MqttClientStatus (requires MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::MqttClientStatus(payload) if is_mqtt => {
                                let status = match payload.status {
                                    WireMqttClientConnectionStatus::Online => {
                                        ApiMqttClientConnectionStatus::Online
                                    }
                                    WireMqttClientConnectionStatus::Offline => {
                                        ApiMqttClientConnectionStatus::Offline
                                    }
                                    WireMqttClientConnectionStatus::Connecting => {
                                        ApiMqttClientConnectionStatus::Connecting
                                    }
                                };

                                if let Err(e) =
                                    crate::mqtt_client_store::update_mqtt_client_status(
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
                            }

                            // -------------------------------------------------
                            // MqttTriggerUpdate (requires MqttBridge)
                            // -------------------------------------------------
                            ServiceMessage::MqttTriggerUpdate(payload) if is_mqtt => {
                                // Validate tenant is assigned to this MQTT service.
                                let tenant_assigned = mqtt_context
                                    .as_ref()
                                    .map(|mctx| {
                                        mctx.tenant_configs
                                            .iter()
                                            .any(|c| c.tenant_id == payload.tenant_id)
                                    })
                                    .unwrap_or(false);

                                if !tenant_assigned {
                                    let err_msg = ControllerMessage::Error(ErrorPayload {
                                        code: ErrorCode::BadRequest,
                                        message:
                                            "tenant not assigned to this MQTT service"
                                                .to_string(),
                                    });
                                    if let Some(json) =
                                        serialize_controller_msg(out_seq, err_msg)
                                    {
                                        let _ =
                                            sink.send(Message::Text(json.into())).await;
                                    }
                                    continue;
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
                                        let err_msg =
                                            ControllerMessage::Error(ErrorPayload {
                                                code: ErrorCode::BadRequest,
                                                message: err.to_string(),
                                            });
                                        if let Some(json) =
                                            serialize_controller_msg(out_seq, err_msg)
                                        {
                                            let _ = sink
                                                .send(Message::Text(json.into()))
                                                .await;
                                        }
                                    }
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
    let capabilities: BTreeSet<Capability> =
        match service::Entity::find_by_id(service_id).one(state.db()).await {
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

