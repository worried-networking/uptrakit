use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tokio::sync::mpsc;
use uptrakit_internal_wire::{
    ApprovedPayload, CertificatePayload, CloseReason, ControllerMessage, DiscoverSoftwarePayload,
    DiscoveryProviderAssignment, ErrorCode, ErrorPayload, ExecuteUpdatePayload, IncomingSeq,
    OutgoingSeq, PingPayload, ProviderType, RejectedPayload, ServiceMessage, UpdateFinalStatus,
};
use uptrakit_shared_db::entity::{
    available_version, host, host_software_item, provider_config, service_host, software_item,
    update_history,
};

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

use super::service_ws::{
    MessageRateLimiter, WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW, close_with_reason,
    deserialize_service_msg, record_service_activity, send_pong, serialize_controller_msg,
};
use crate::AppState;
use crate::routes::agents::{do_sign_csr, find_or_create_host_and_link, revoke_certificate};
use sea_orm::sea_query::{Expr, ExprTrait};
use uptrakit_shared_db::entity::update_output_line;
use uptrakit_shared_db::entity::update_output_line::Entity as UpdateOutputLine;

#[derive(Debug, Error)]
enum AgentWsError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("websocket send failed")]
    WebSocketSend,
}

type AgentWsResult<T> = std::result::Result<T, Report<AgentWsError>>;

impl_report_conversion!(sea_orm::DbErr => AgentWsError::Database);

/// Maximum size of the `update_history.output` column (1 MB).
///
/// Once the output reaches this limit, further `UpdateOutput` messages are
/// silently dropped to prevent unbounded DB growth.
const MAX_UPDATE_OUTPUT_BYTES: usize = 1_048_576;

// ---------------------------------------------------------------------------
// Authenticated agent handler (called from service_ws after shared auth)
// ---------------------------------------------------------------------------

/// Service-type-specific handler for an authenticated agent connection.
///
/// Called by [`super::service_ws`] after certificate validation, service status
/// check, and sending `ServiceSettings`. Owns the agent-specific message loop
/// (ReportHosts, VersionCheckResults, Update*, RenewCertificate).
pub(crate) async fn handle_agent_authenticated(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    ctx: super::service_ws::AuthenticatedContext<'_>,
) {
    let super::service_ws::AuthenticatedContext {
        service_id: agent_id,
        cert,
        last_seen_at,
        out_seq,
        in_seq,
    } = ctx;
    // Register first so concurrent outbox events can reach us via push_rx.
    let (mut push_rx, cancel_token) = state.service_connections.register_agent(agent_id).await;

    // Deliver pending updates for hosts linked to this agent.
    // Any concurrent outbox events that arrive between registration and
    // this query are buffered in push_rx (not lost).
    if let Err(e) = deliver_pending_updates(state, agent_id, sink, out_seq).await {
        tracing::error!(error = %e, %agent_id, "failed to deliver pending updates on reconnect");
    }

    let delivered = state
        .notification_service
        .deliver_backlog_for_authenticated_service(
            agent_id,
            uptrakit_shared_db::entity::service::ServiceType::Agent,
            last_seen_at,
        )
        .await;
    if delivered > 0 {
        tracing::info!(
            %agent_id,
            delivered,
            "delivered outbox backlog to agent"
        );
    }

    // Cache host IDs linked to this agent for update ownership validation.
    // Refreshed on ReportHosts (which may link new hosts).
    let mut linked_host_ids: HashSet<uuid::Uuid> = load_linked_host_ids(state.db(), agent_id)
        .await
        .unwrap_or_default();

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

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
                        let agent_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                            Ok(Some(m)) => m,
                            Ok(None) => continue,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match agent_msg {
                            ServiceMessage::Ping(PingPayload { service_ts }) => {
                                let Ok(controller_ts) = send_pong(sink, out_seq, service_ts).await else { break };
                                tracing::trace!(service_ts, controller_ts, "ping/pong");
                                if let Err(e) = record_service_activity(state.db(), agent_id, None).await {
                                    tracing::warn!(error = %e, %agent_id, "failed to record service activity");
                                }
                            }
                            ServiceMessage::ReportHosts(payload) => {
                                tracing::debug!(
                                    %agent_id,
                                    capabilities = ?payload.capabilities,
                                    "received ReportHosts"
                                );

                                // Look up agent hostname from DB for host linking
                                let agent_model = match uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                    .one(state.db())
                                    .await
                                {
                                    Ok(Some(a)) => a,
                                    _ => continue,
                                };

                                // Update client_version in database
                                let mut active: uptrakit_shared_db::entity::service::ActiveModel = agent_model.clone().into();
                                active.client_version = Set(Some(payload.agent_version.clone()));
                                active.updated_at = Set(time::OffsetDateTime::now_utc());
                                if let Err(e) = active.update(state.db()).await {
                                    tracing::error!(error = %e, "failed to update client_version");
                                }

                                for host_info in &payload.hosts {
                                    let host_hostname = host_info.hostname.as_deref().unwrap_or(&agent_model.hostname);
                                    let host_ip = host_info.ip_address.as_deref().or(agent_model.ip_address.as_deref());
                                    match find_or_create_host_and_link(
                                        state.db(),
                                        agent_model.tenant_id,
                                        agent_id,
                                        host_info,
                                        host_hostname,
                                        host_ip,
                                    ).await {
                                        Ok(Some((_host_id, true))) => {
                                            // New host was registered — trigger autodiscovery.
                                            trigger_discovery_for_agent_host(
                                                state,
                                                agent_id,
                                                agent_model.tenant_id,
                                                &host_info.machine_id,
                                            )
                                            .await;
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            tracing::warn!(error = %e, machine_id = %host_info.machine_id, "failed to link host");
                                        }
                                    }
                                }

                                // Refresh cached host IDs (may have linked a new host).
                                if let Ok(ids) = load_linked_host_ids(state.db(), agent_id).await {
                                    linked_host_ids = ids;
                                }
                            }
                            ServiceMessage::RenewCertificate(payload) => {
                                // Re-fetch agent from DB, verify still approved
                                let agent = match uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                    .one(state.db())
                                    .await
                                {
                                    Ok(Some(a)) if a.status == uptrakit_shared_db::entity::service::ServiceStatus::Approved && a.deactivated_at.is_none() => a,
                                    _ => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::Forbidden,
                                            message: "agent is not approved".to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                };

                                // Sign new certificate from agent's CSR
                                match do_sign_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    state.db(),
                                    agent,
                                    &payload.csr_pem,
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }

                                        // Revoke old cert
                                        if let Err(e) = revoke_certificate(state.db(), &cert.serial, &cert.ca_fingerprint, uptrakit_shared_db::entity::prelude::RevocationReason::CertificateRenewed).await {
                                            tracing::error!(error = %e, "failed to revoke old certificate");
                                        }

                                        if let Err(e) = crate::settings_store::bump_revocation_version(state.db(), state.default_tenant_id).await {
                                            tracing::warn!(error = ?e, "failed to bump revocation version counter");
                                        }
                                        state.revocation_notify.notify_one();
                                        tracing::info!(%agent_id, old_serial = %cert.serial, "certificate renewed, old cert revoked");
                                        let _ = close_with_reason(sink, CloseReason::CertificateRotated).await;
                                        break;
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::CertificateError,
                                            message: e.current_context().to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                }
                            }
                            ServiceMessage::VersionCheckResults(payload) => {
                                tracing::debug!(%agent_id, count = payload.results.len(), "received VersionCheckResults");

                                // Look up hosts linked to this agent
                                let host_ids: Vec<uuid::Uuid> = match uptrakit_shared_db::entity::prelude::ServiceHost::find()
                                    .filter(uptrakit_shared_db::entity::service_host::Column::ServiceId.eq(agent_id))
                                    .all(state.db())
                                    .await
                                {
                                    Ok(links) => links.into_iter().map(|l| l.host_id).collect(),
                                    Err(e) => {
                                        tracing::warn!(error = %e, "failed to look up agent hosts");
                                        continue;
                                    }
                                };

                                if host_ids.is_empty() {
                                    tracing::debug!(%agent_id, "no hosts linked to agent, skipping version updates");
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

                                    // Update installed version on host_software_item records
                                    if let Some(ref installed_version) = result.installed_version {
                                        for &host_id in &host_ids {
                                            match uptrakit_shared_db::entity::prelude::HostSoftwareItem::find_by_id((host_id, software_item_id))
                                                .one(state.db())
                                                .await
                                            {
                                                Ok(Some(existing)) => {
                                                    let mut active: uptrakit_shared_db::entity::host_software_item::ActiveModel = existing.into();
                                                    active.installed_version = Set(Some(installed_version.clone()));
                                                    active.installed_version_detected_at = Set(Some(now));
                                                    if let Err(e) = active.update(state.db()).await {
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

                                    // If the agent reported a latest_version (agent-side
                                    // resolution, e.g. Homebrew), upsert an available_version
                                    // record for this software item.
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

                                // Batch-update last_checked_at for all software items that
                                // had at least one successful result (no error). This is a
                                // single UPDATE rather than per-item to avoid N DB round-trips.
                                let checked_ids: Vec<uuid::Uuid> = payload
                                    .results
                                    .iter()
                                    .filter(|r| r.error.is_none())
                                    .map(|r| r.software_item_id)
                                    .collect::<std::collections::HashSet<_>>()
                                    .into_iter()
                                    .collect();

                                if !checked_ids.is_empty()
                                    && let Err(e) = software_item::Entity::update_many()
                                        .filter(software_item::Column::Id.is_in(checked_ids))
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

                                // Push updated software states to MQTT services for this tenant.
                                if let Ok(Some(agent_svc)) =
                                    uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                        .one(state.db())
                                        .await
                                {
                                    state
                                        .notification_service
                                        .push_software_states_for_tenant(agent_svc.tenant_id)
                                        .await;
                                }
                            }
                            ServiceMessage::UpdateStarted(payload) => {
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    from_version = ?payload.from_version,
                                    "update started"
                                );
                                let record = match validate_update_ownership(
                                    state.db(), agent_id, payload.update_history_id, &linked_host_ids,
                                ).await {
                                    Ok(r) => r,
                                    Err(_) => continue,
                                };
                                let mut active: update_history::ActiveModel = record.into();
                                active.status = Set(update_history::UpdateStatus::InProgress);
                                active.started_at = Set(time::OffsetDateTime::now_utc());
                                if payload.from_version.is_some() {
                                    active.from_version = Set(payload.from_version);
                                }
                                active.output = Set(String::new());
                                active.output_bytes = Set(0);
                                if let Err(e) = active.update(state.db()).await {
                                    tracing::warn!(error = %e, "failed to update update_history status");
                                }
                                if let Err(e) = UpdateOutputLine::delete_many()
                                    .filter(
                                        update_output_line::Column::UpdateHistoryId
                                            .eq(payload.update_history_id),
                                    )
                                    .exec(state.db())
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to clear update output lines");
                                }
                            }
                            ServiceMessage::UpdateOutput(payload) => {
                                tracing::trace!(
                                    update_id = %payload.update_history_id,
                                    stream = ?payload.stream,
                                    "update output"
                                );
                                if validate_update_ownership(
                                    state.db(), agent_id, payload.update_history_id, &linked_host_ids,
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
                                        Expr::col(update_history::Column::OutputBytes).add(line_len),
                                    )
                                    .filter(update_history::Column::Id.eq(payload.update_history_id))
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
                                if let Err(e) = UpdateOutputLine::insert(line).exec(state.db()).await {
                                    tracing::warn!(error = %e, "failed to insert update output line");
                                }
                            }
                            ServiceMessage::UpdateResult(payload) => {
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    status = ?payload.status,
                                    error = ?payload.error,
                                    "update result"
                                );
                                let record = match validate_update_ownership(
                                    state.db(), agent_id, payload.update_history_id, &linked_host_ids,
                                ).await {
                                    Ok(r) => r,
                                    Err(_) => continue,
                                };
                                let mut active: update_history::ActiveModel = record.clone().into();
                                active.status = Set(match payload.status {
                                    UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
                                    UpdateFinalStatus::Failed => update_history::UpdateStatus::Failed,
                                    _ => update_history::UpdateStatus::Failed,
                                });
                                active.completed_at = Set(Some(time::OffsetDateTime::now_utc()));
                                let capped_output = if payload.output.len() > MAX_UPDATE_OUTPUT_BYTES {
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
                                    tracing::warn!(error = %e, "failed to update update_history result");
                                }

                                if let Err(e) = UpdateOutputLine::delete_many()
                                    .filter(
                                        update_output_line::Column::UpdateHistoryId
                                            .eq(payload.update_history_id),
                                    )
                                    .exec(state.db())
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to clear update output lines");
                                }

                                if payload.status == UpdateFinalStatus::Completed
                                    && let Some(ref to_version) = payload.to_version
                                    && let Ok(Some(link)) = uptrakit_shared_db::entity::prelude::HostSoftwareItem::find_by_id((record.host_id, record.software_item_id))
                                        .one(state.db())
                                        .await
                                {
                                    let mut link_active: host_software_item::ActiveModel = link.into();
                                    link_active.installed_version = Set(Some(to_version.clone()));
                                    link_active.installed_version_detected_at = Set(Some(time::OffsetDateTime::now_utc()));
                                    link_active.last_updated_at = Set(Some(time::OffsetDateTime::now_utc()));
                                    if let Err(e) = link_active.update(state.db()).await {
                                        tracing::warn!(error = %e, "failed to update host_software_item installed_version");
                                    }
                                }

                                // Push updated software states to MQTT services.
                                if let Ok(Some(agent_svc)) =
                                    uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                        .one(state.db())
                                        .await
                                {
                                    state
                                        .notification_service
                                        .push_software_states_for_tenant(agent_svc.tenant_id)
                                        .await;
                                }
                            }
                            ServiceMessage::DiscoveryResults(payload) => {
                                tracing::debug!(
                                    %agent_id,
                                    host_machine_id = %payload.host_machine_id,
                                    results = payload.results.len(),
                                    "received DiscoveryResults"
                                );

                                // Find the host this discovery result targets.
                                // Find all service-host links for this agent.
                                // We resolve machine_id below by joining through the host entity.
                                let links_opt = uptrakit_shared_db::entity::prelude::ServiceHost::find()
                                    .filter(service_host::Column::ServiceId.eq(agent_id))
                                    .all(state.db())
                                    .await
                                    .ok();

                                if let Some(links) = links_opt {
                                    let mut host_id_opt: Option<uuid::Uuid> = None;
                                    for link in &links {
                                        if let Ok(Some(h)) = host::Entity::find_by_id(link.host_id)
                                            .filter(host::Column::MachineId.eq(&payload.host_machine_id))
                                            .filter(host::Column::DeactivatedAt.is_null())
                                            .one(state.db())
                                            .await
                                        {
                                            host_id_opt = Some(h.id);
                                            break;
                                        }
                                    }

                                    if let Some(host_id) = host_id_opt {
                                        if let Ok(Some(agent_svc)) = uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                            .one(state.db())
                                            .await
                                            && let Err(e) = crate::queries::autodiscovery::process_discovery_results(
                                                state.db(),
                                                agent_id,
                                                agent_svc.tenant_id,
                                                host_id,
                                                payload,
                                            ).await
                                        {
                                            tracing::warn!(
                                                error = %e,
                                                %agent_id,
                                                "failed to process discovery results"
                                            );
                                        }
                                    } else {
                                        tracing::warn!(
                                            %agent_id,
                                            host_machine_id = %payload.host_machine_id,
                                            "received DiscoveryResults for unknown host machine_id"
                                        );
                                    }
                                }
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %agent_id,
                                    reason = ?payload.reason,
                                    "agent disconnecting gracefully"
                                );
                                break;
                            }
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "unexpected message for authenticated connection".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
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
                tracing::info!(%agent_id, "connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Do NOT unregister — the new connection owns the registry entry.
                return;
            }
        }
    }

    state.service_connections.unregister(&agent_id).await;
    tracing::debug!(%agent_id, "authenticated agent disconnected");
}

// ---------------------------------------------------------------------------
// Enrolled agent handler (called from service_ws after shared enrolled setup)
// ---------------------------------------------------------------------------

/// Service-type-specific enrolled handler for an agent connection.
///
/// Called by [`super::service_ws`] for enrolled agents. Registers in the
/// connection registry and runs the enrolled loop.
pub(crate) async fn handle_agent_enrolled(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    let (mut push_rx, cancel_token) = state.service_connections.register_agent(agent_id).await;
    run_agent_enrolled_loop(
        sink,
        stream,
        (&mut push_rx, &cancel_token),
        state,
        agent_id,
        out_seq,
        in_seq,
    )
    .await;
    if !cancel_token.is_cancelled() {
        state.service_connections.unregister(&agent_id).await;
    }
}

// ---------------------------------------------------------------------------
// Shared agent enrolled loop
// ---------------------------------------------------------------------------

/// Interval between approval-status DB polls in enrolled loops.
const APPROVAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Shared enrolled loop for agents: handles Ping, RequestCertificate, and
/// push messages (Approved / Rejected).
///
/// The `connection` tuple contains the push-message receiver and cancellation
/// token returned by `ServiceConnectionRegistry::register_agent()`.
pub(crate) async fn run_agent_enrolled_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    connection: (
        &mut mpsc::Receiver<ControllerMessage>,
        &tokio_util::sync::CancellationToken,
    ),
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    let (push_rx, cancel_token) = connection;
    let mut approved = false;
    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Check current status to set initial approved flag.
    if let Ok(Some(agent)) = uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
        .one(state.db())
        .await
        && agent.status == uptrakit_shared_db::entity::service::ServiceStatus::Approved
    {
        approved = true;
    }

    // Dedicated interval for polling approval status from the DB, decoupled
    // from client-controlled ping frequency.
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
                        let agent_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                            Ok(Some(m)) => m,
                            Ok(None) => continue,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match agent_msg {
                            ServiceMessage::Ping(PingPayload { service_ts }) => {
                                let Ok(controller_ts) = send_pong(sink, out_seq, service_ts).await else { break };
                                tracing::trace!(service_ts, controller_ts, "ping/pong (enrolled)");
                                if let Err(e) = record_service_activity(state.db(), agent_id, None).await {
                                    tracing::warn!(error = %e, %agent_id, "failed to record service activity");
                                }
                            }
                            ServiceMessage::RequestCertificate(payload) => {
                                if !approved {
                                    let err = ControllerMessage::Error(ErrorPayload {
                                        code: ErrorCode::NotApproved,
                                        message: "agent is not yet approved".to_string(),
                                    });
                                    if let Some(json) = serialize_controller_msg(out_seq, err) {
                                        let _ = sink.send(Message::Text(json.into())).await;
                                    }
                                    continue;
                                }

                                // Re-fetch agent from DB
                                let agent = match uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                    .one(state.db())
                                    .await
                                {
                                    Ok(Some(a)) => a,
                                    _ => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::InternalError,
                                            message: "agent not found".to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                };

                                match do_sign_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    state.db(),
                                    agent,
                                    &payload.csr_pem,
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        tracing::info!(%agent_id, "certificate issued via WS");
                                        break; // close connection after certificate issuance
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::CertificateError,
                                            message: e.current_context().to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                }
                            }
                            ServiceMessage::ReportHosts(_) => {
                                // Host linking not supported during enrolled loop
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
                            ServiceMessage::RenewCertificate(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            ServiceMessage::VersionCheckResults(_)
                            | ServiceMessage::DiscoveryResults(_) => {
                                // These are not supported during enrollment
                            }
                            ServiceMessage::UpdateStarted(_)
                            | ServiceMessage::UpdateOutput(_)
                            | ServiceMessage::UpdateResult(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "update messages not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %agent_id,
                                    reason = ?payload.reason,
                                    "agent disconnecting gracefully during enrollment"
                                );
                                break;
                            }
                            // MQTT-specific variants are not valid on an agent connection
                            ServiceMessage::Register(_)
                            | ServiceMessage::ReleaseTenants(_)
                            | ServiceMessage::MqttClientStatus(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "message type not supported on agent connections".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "unknown message type".to_string(),
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
            // Dedicated approval poll at a fixed interval, decoupled from
            // client-controlled ping frequency.
            _ = approval_poll.tick(), if !approved => {
                if let Ok(Some(s)) = uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                    .one(state.db())
                    .await
                {
                    match s.status {
                        uptrakit_shared_db::entity::service::ServiceStatus::Approved => {
                            approved = true;
                            let msg = ControllerMessage::Approved(ApprovedPayload { service_id: agent_id });
                            if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                        }
                        uptrakit_shared_db::entity::service::ServiceStatus::Rejected => {
                            let msg = ControllerMessage::Rejected(RejectedPayload { service_id: agent_id });
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
                tracing::info!(%agent_id, "enrolled connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pending updates delivery (agent-specific)
// ---------------------------------------------------------------------------

/// Deliver pending updates for hosts linked to this agent.
///
/// On agent reconnect, we check for any `update_history` records with
/// `status = Pending` for hosts linked to this agent and send them.
async fn deliver_pending_updates(
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
) -> AgentWsResult<()> {
    // 1. Find host_ids linked to this agent
    let host_links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(agent_id))
        .all(state.db())
        .await
        .context_to::<AgentWsError>()?;

    if host_links.is_empty() {
        return Ok(());
    }

    let host_ids: Vec<uuid::Uuid> = host_links.iter().map(|l| l.host_id).collect();

    // 2. Query pending update_history records for those hosts
    let pending_updates = update_history::Entity::find()
        .filter(update_history::Column::HostId.is_in(host_ids))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .all(state.db())
        .await
        .context_to::<AgentWsError>()?;

    if pending_updates.is_empty() {
        return Ok(());
    }

    tracing::info!(
        %agent_id,
        count = pending_updates.len(),
        "delivering pending updates on reconnect"
    );

    // 3. Build ExecuteUpdatePayload for each and send
    for update_record in pending_updates {
        let item = match software_item::Entity::find_by_id(update_record.software_item_id)
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(state.db())
            .await
        {
            Ok(Some(i)) => i,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    software_item_id = %update_record.software_item_id,
                    "software item not found or deactivated, skipping pending update"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load software item for pending update");
                continue;
            }
        };

        // Load per-host provider info from the host_software_item link.
        let link = match host_software_item::Entity::find_by_id((update_record.host_id, item.id))
            .one(state.db())
            .await
        {
            Ok(Some(l)) => l,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    host_id = %update_record.host_id,
                    software_item_id = %item.id,
                    "host-software-item link not found, skipping pending update"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load host-software-item link for pending update");
                continue;
            }
        };

        let provider_cfg = match provider_config::Entity::find_by_id(link.provider_config_id)
            .filter(provider_config::Column::DeactivatedAt.is_null())
            .one(state.db())
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    provider_config_id = %link.provider_config_id,
                    "provider config not found or deactivated, skipping pending update"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load provider config for pending update");
                continue;
            }
        };

        let provider_type: ProviderType = match serde_json::from_value(serde_json::Value::String(
            provider_cfg.provider_type.clone(),
        )) {
            Ok(pt) => pt,
            Err(_) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    provider_type = %provider_cfg.provider_type,
                    "unknown provider type, skipping pending update"
                );
                continue;
            }
        };

        let resolved_hooks =
            crate::update_hooks::resolve_hooks(&provider_cfg.config, link.config_override.as_ref());
        let merged_config =
            crate::update_hooks::merge_config(&provider_cfg.config, link.config_override.as_ref());

        // Look up the host's machine_id so the agent can route correctly.
        let host_machine_id = match host::Entity::find_by_id(update_record.host_id)
            .one(state.db())
            .await
        {
            Ok(Some(h)) => h.machine_id,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    host_id = %update_record.host_id,
                    "host not found for pending update, skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load host for pending update");
                continue;
            }
        };

        let execute_payload = ExecuteUpdatePayload {
            host_machine_id,
            update_history_id: update_record.id,
            software_item_id: item.id,
            software_item_name: item.name.clone(),
            package_identifier: link.package_identifier.clone(),
            to_version: update_record.to_version.clone(),
            provider_type,
            provider_config: merged_config,
            pre_update_hooks: resolved_hooks.pre_update_hooks,
            post_update_hooks: resolved_hooks.post_update_hooks,
            release_info: None,
            timeout_seconds: 300,
        };

        let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
        let Some(json) = serialize_controller_msg(out_seq, msg) else {
            continue;
        };

        if sink.send(Message::Text(json.into())).await.is_err() {
            bail!(AgentWsError::WebSocketSend);
        }

        tracing::info!(
            update_id = %update_record.id,
            %agent_id,
            software = %item.name,
            "delivered pending update on reconnect"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Update ownership validation (FP-17)
// ---------------------------------------------------------------------------

/// Load the set of host IDs linked to the given agent.
async fn load_linked_host_ids(
    db: &sea_orm::DatabaseConnection,
    agent_id: uuid::Uuid,
) -> AgentWsResult<HashSet<uuid::Uuid>> {
    let links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(agent_id))
        .all(db)
        .await
        .context_to::<AgentWsError>()?;

    Ok(links.into_iter().map(|l| l.host_id).collect())
}

/// Validate that an `update_history` record belongs to a host linked to the
/// current agent. Returns the record on success, logs a warning and returns
/// an error if the agent does not own the record.
async fn validate_update_ownership(
    db: &sea_orm::DatabaseConnection,
    agent_id: uuid::Uuid,
    update_history_id: uuid::Uuid,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> AgentWsResult<update_history::Model> {
    let record = uptrakit_shared_db::entity::prelude::UpdateHistory::find_by_id(update_history_id)
        .one(db)
        .await
        .context_to::<AgentWsError>()?
        .ok_or_else(|| {
            tracing::warn!(
                %agent_id,
                update_id = %update_history_id,
                "update_history record not found"
            );
            report!(AgentWsError::WebSocketSend)
        })?;

    if !linked_host_ids.contains(&record.host_id) {
        tracing::warn!(
            %agent_id,
            update_id = %update_history_id,
            host_id = %record.host_id,
            "agent attempted to update record for unlinked host"
        );
        bail!(AgentWsError::WebSocketSend);
    }

    Ok(record)
}

/// Upsert an `available_version` record for a software item.
///
/// If an existing record with the same version already exists for this software
/// item, its `updated_at` timestamp is refreshed. Otherwise, old records for
/// this software item are deleted and a new one is inserted.
async fn upsert_available_version(
    db: &sea_orm::DatabaseConnection,
    software_item_id: uuid::Uuid,
    version: &str,
    now: time::OffsetDateTime,
) {
    // Check if a record with this version already exists.
    let existing = available_version::Entity::find()
        .filter(available_version::Column::SoftwareItemId.eq(software_item_id))
        .filter(available_version::Column::Version.eq(version))
        .one(db)
        .await;

    match existing {
        Ok(Some(record)) => {
            // Version already recorded — just refresh the timestamp.
            let mut active: available_version::ActiveModel = record.into();
            active.updated_at = Set(now);
            if let Err(e) = active.update(db).await {
                tracing::warn!(
                    error = %e,
                    software_item_id = %software_item_id,
                    version,
                    "failed to update available_version timestamp"
                );
            }
        }
        Ok(None) => {
            // Delete any previous available_version records for this item
            // and insert the new one.
            if let Err(e) = available_version::Entity::delete_many()
                .filter(available_version::Column::SoftwareItemId.eq(software_item_id))
                .exec(db)
                .await
            {
                tracing::warn!(
                    error = %e,
                    software_item_id = %software_item_id,
                    "failed to delete old available_version records"
                );
            }

            let record = available_version::ActiveModel {
                id: Set(uuid::Uuid::now_v7()),
                software_item_id: Set(software_item_id),
                version: Set(Some(version.to_string())),
                release_date: Set(None),
                release_notes: Set(None),
                extra: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            };
            if let Err(e) = available_version::Entity::insert(record).exec(db).await {
                tracing::warn!(
                    error = %e,
                    software_item_id = %software_item_id,
                    version,
                    "failed to insert available_version"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                software_item_id = %software_item_id,
                "failed to query available_version"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Autodiscovery helper
// ---------------------------------------------------------------------------

/// Send `DiscoverSoftware` to the given agent for the given host.
///
/// Queries all active provider configs for discovery-capable provider types.
/// If no configs exist for a type, sends a single default (empty-config)
/// assignment so the agent can still discover software.
pub(crate) async fn trigger_discovery_for_agent_host(
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    host_machine_id: &str,
) {
    let discovery_types = state.provider_ops.discovery_provider_types();

    let mut providers: Vec<DiscoveryProviderAssignment> = Vec::new();

    for provider_type in discovery_types {
        let type_str = provider_type.to_string();

        let configs = match provider_config::Entity::find()
            .filter(provider_config::Column::TenantId.eq(tenant_id))
            .filter(provider_config::Column::ProviderType.eq(&type_str))
            .filter(provider_config::Column::Enabled.eq(true))
            .filter(provider_config::Column::DeactivatedAt.is_null())
            .all(state.db())
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %provider_type,
                    "failed to query provider configs for discovery trigger"
                );
                continue;
            }
        };

        if configs.is_empty() {
            // No configs for this type — send a default assignment.
            providers.push(DiscoveryProviderAssignment {
                provider_config_id: None,
                provider_type: provider_type.clone(),
                config: serde_json::Value::Object(Default::default()),
            });
        } else {
            for cfg in configs {
                providers.push(DiscoveryProviderAssignment {
                    provider_config_id: Some(cfg.id),
                    provider_type: provider_type.clone(),
                    config: cfg.config,
                });
            }
        }
    }

    if providers.is_empty() {
        tracing::debug!(%agent_id, "no discovery-capable providers configured; skipping discovery trigger");
        return;
    }

    let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
        host_machine_id: host_machine_id.to_string(),
        providers,
    });

    tracing::info!(
        %agent_id,
        %host_machine_id,
        "triggering autodiscovery for newly registered host"
    );
    state.notification_service.send(&agent_id, msg).await;
}
