use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tokio::sync::mpsc;
use uptrakit_internal_wire::{
    CertificatePayload, ControllerMessage, ErrorPayload, ExecuteUpdatePayload, PingPayload,
    PongPayload, ProviderType, ServiceMessage, UpdateFinalStatus, now_millis,
};
use uptrakit_shared_db::entity::{
    host_software_item, provider_config, service_host as agent_host, software_item, update_history,
};

use rootcause::prelude::*;
use thiserror::Error;

use super::service_ws::{close_with_reason, serialize_msg};
use crate::AppState;
use crate::routes::agents::{do_sign_csr, find_or_create_host_and_link, revoke_certificate};

#[derive(Debug, Error)]
enum AgentWsError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("websocket send failed")]
    WebSocketSend,
}

type AgentWsResult<T> = std::result::Result<T, Report<AgentWsError>>;

/// Minimum agent version required for connection.
const MIN_AGENT_VERSION: &str = "0.0.1";

// ---------------------------------------------------------------------------
// Authenticated agent handler (called from service_ws after shared auth)
// ---------------------------------------------------------------------------

/// Service-type-specific handler for an authenticated agent connection.
///
/// Called by [`super::service_ws`] after certificate validation, service status
/// check, and sending `ServiceSettings`. Owns the agent-specific message loop
/// (ReportHostInfo, VersionCheckResults, Update*, RenewCertificate).
pub(crate) async fn handle_agent_authenticated(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
    cert_serial: String,
    cert_ca_fingerprint: String,
) {
    // Deliver pending updates for hosts linked to this agent.
    if let Err(e) = deliver_pending_updates(state, agent_id, sink).await {
        tracing::error!(error = %e, %agent_id, "failed to deliver pending updates on reconnect");
    }

    let mut push_rx = state.service_connections.register_agent(agent_id).await;

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
                match msg {
                    Message::Text(text) => {
                        let agent_msg: ServiceMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match agent_msg {
                            ServiceMessage::Ping(PingPayload { agent_ts }) => {
                                let controller_ts = now_millis();
                                tracing::trace!(agent_ts, controller_ts, "ping/pong");
                                let response = ControllerMessage::Pong(PongPayload {
                                    agent_ts,
                                    controller_ts,
                                });
                                let Some(json) = serialize_msg(&response) else { break };
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            }
                            ServiceMessage::ReportHostInfo(payload) => {
                                // Check agent version
                                let agent_ver = match semver::Version::parse(&payload.agent_version) {
                                    Ok(v) => v,
                                    Err(_) => {
                                        tracing::warn!(
                                            %agent_id,
                                            version = %payload.agent_version,
                                            "agent sent invalid version string"
                                        );
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "agent_version_too_old".to_string(),
                                            message: format!(
                                                "invalid agent version '{}', minimum required: {MIN_AGENT_VERSION}",
                                                payload.agent_version
                                            ),
                                        });
                                        if let Some(json) = serialize_msg(&err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        let _ = close_with_reason(sink, "agent version too old").await;
                                        break;
                                    }
                                };

                                let min_ver = semver::Version::parse(MIN_AGENT_VERSION)
                                    .expect("MIN_AGENT_VERSION must be valid semver");

                                if agent_ver < min_ver {
                                    tracing::warn!(
                                        %agent_id,
                                        version = %payload.agent_version,
                                        min_version = MIN_AGENT_VERSION,
                                        "agent version too old"
                                    );
                                    let err = ControllerMessage::Error(ErrorPayload {
                                        code: "agent_version_too_old".to_string(),
                                        message: format!(
                                            "agent version {} is too old, minimum required: {MIN_AGENT_VERSION}",
                                            payload.agent_version
                                        ),
                                    });
                                    if let Some(json) = serialize_msg(&err) {
                                        let _ = sink.send(Message::Text(json.into())).await;
                                    }
                                    let _ = close_with_reason(sink, "agent version too old").await;
                                    break;
                                }

                                // Look up agent hostname from DB for host linking
                                let agent_model = match uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(a)) => a,
                                    _ => continue,
                                };

                                // Update client_version in database
                                let mut active: uptrakit_shared_db::entity::service::ActiveModel = agent_model.clone().into();
                                active.client_version = Set(Some(payload.agent_version.clone()));
                                active.updated_at = Set(time::OffsetDateTime::now_utc());
                                if let Err(e) = active.update(&state.db).await {
                                    tracing::error!(error = %e, "failed to update client_version");
                                }

                                if let Err(e) = find_or_create_host_and_link(
                                    &state.db,
                                    agent_model.tenant_id,
                                    agent_id,
                                    &payload.host_info,
                                    &agent_model.hostname,
                                    agent_model.ip_address.as_deref(),
                                ).await {
                                    tracing::warn!(error = %e, "failed to link host on ReportHostInfo");
                                }
                            }
                            ServiceMessage::RenewCertificate(payload) => {
                                // Re-fetch agent from DB, verify still approved
                                let agent = match uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(a)) if a.status == uptrakit_shared_db::entity::service::ServiceStatus::Approved && a.deactivated_at.is_none() => a,
                                    _ => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "forbidden".to_string(),
                                            message: "agent is not approved".to_string(),
                                        });
                                        if let Some(json) = serialize_msg(&err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                };

                                // Sign new certificate from agent's CSR
                                match do_sign_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    &state.db,
                                    agent,
                                    &payload.csr_pem,
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_msg(&cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }

                                        // Revoke old cert
                                        if let Err(e) = revoke_certificate(&state.db, &cert_serial, &cert_ca_fingerprint, uptrakit_shared_db::entity::prelude::RevocationReason::CertificateRenewed).await {
                                            tracing::error!(error = %e, "failed to revoke old certificate");
                                        }

                                        state.revocation_notify.notify_one();
                                        tracing::info!(%agent_id, old_serial = %cert_serial, "certificate renewed, old cert revoked");
                                        let _ = close_with_reason(sink, "certificate rotated").await;
                                        break;
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "certificate_error".to_string(),
                                            message: e.current_context().to_string(),
                                        });
                                        if let Some(json) = serialize_msg(&err) {
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
                                    .all(&state.db)
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

                                    let Some(ref installed_version) = result.installed_version else {
                                        continue;
                                    };

                                    let software_item_id = match uuid::Uuid::parse_str(&result.software_item_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            tracing::warn!(
                                                software_item_id = %result.software_item_id,
                                                "invalid software_item_id UUID"
                                            );
                                            continue;
                                        }
                                    };

                                    for &host_id in &host_ids {
                                        match uptrakit_shared_db::entity::prelude::HostSoftwareItem::find_by_id((host_id, software_item_id))
                                            .one(&state.db)
                                            .await
                                        {
                                            Ok(Some(existing)) => {
                                                let mut active: uptrakit_shared_db::entity::host_software_item::ActiveModel = existing.into();
                                                active.installed_version = Set(Some(installed_version.clone()));
                                                active.installed_version_detected_at = Set(Some(now));
                                                if let Err(e) = active.update(&state.db).await {
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
                            }
                            ServiceMessage::UpdateStarted(payload) => {
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    from_version = ?payload.from_version,
                                    "update started"
                                );
                                if let Ok(update_id) = uuid::Uuid::parse_str(&payload.update_history_id)
                                    && let Ok(Some(record)) = uptrakit_shared_db::entity::prelude::UpdateHistory::find_by_id(update_id)
                                        .one(&state.db)
                                        .await
                                {
                                    let mut active: update_history::ActiveModel = record.into();
                                    active.status = Set(update_history::UpdateStatus::InProgress);
                                    active.started_at = Set(time::OffsetDateTime::now_utc());
                                    if payload.from_version.is_some() {
                                        active.from_version = Set(payload.from_version);
                                    }
                                    if let Err(e) = active.update(&state.db).await {
                                        tracing::warn!(error = %e, "failed to update update_history status");
                                    }
                                }
                            }
                            ServiceMessage::UpdateOutput(payload) => {
                                tracing::trace!(
                                    update_id = %payload.update_history_id,
                                    stream = ?payload.stream,
                                    "update output"
                                );
                                if let Ok(update_id) = uuid::Uuid::parse_str(&payload.update_history_id)
                                    && let Ok(Some(record)) = uptrakit_shared_db::entity::prelude::UpdateHistory::find_by_id(update_id)
                                        .one(&state.db)
                                        .await
                                {
                                    let mut active: update_history::ActiveModel = record.clone().into();
                                    let new_output = format!("{}{}\n", record.output, payload.output);
                                    active.output = Set(new_output);
                                    if let Err(e) = active.update(&state.db).await {
                                        tracing::warn!(error = %e, "failed to append update output");
                                    }
                                }
                            }
                            ServiceMessage::UpdateResult(payload) => {
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    status = ?payload.status,
                                    error = ?payload.error,
                                    "update result"
                                );
                                if let Ok(update_id) = uuid::Uuid::parse_str(&payload.update_history_id)
                                    && let Ok(Some(record)) = uptrakit_shared_db::entity::prelude::UpdateHistory::find_by_id(update_id)
                                        .one(&state.db)
                                        .await
                                {
                                    let mut active: update_history::ActiveModel = record.clone().into();
                                    active.status = Set(match payload.status {
                                        UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
                                        UpdateFinalStatus::Failed => update_history::UpdateStatus::Failed,
                                    });
                                    active.completed_at = Set(Some(time::OffsetDateTime::now_utc()));
                                    let final_output = if payload.output.is_empty() {
                                        record.output.clone()
                                    } else {
                                        format!("{}{}", record.output, payload.output)
                                    };
                                    active.output = Set(final_output);
                                    if payload.from_version.is_some() {
                                        active.from_version = Set(payload.from_version);
                                    }
                                    if let Err(e) = active.update(&state.db).await {
                                        tracing::warn!(error = %e, "failed to update update_history result");
                                    }

                                    if payload.status == UpdateFinalStatus::Completed
                                        && let Some(ref to_version) = payload.to_version
                                        && let Ok(Some(link)) = uptrakit_shared_db::entity::prelude::HostSoftwareItem::find_by_id((record.host_id, record.software_item_id))
                                            .one(&state.db)
                                            .await
                                    {
                                        let mut link_active: host_software_item::ActiveModel = link.into();
                                        link_active.installed_version = Set(Some(to_version.clone()));
                                        link_active.installed_version_detected_at = Set(Some(time::OffsetDateTime::now_utc()));
                                        link_active.last_updated_at = Set(Some(time::OffsetDateTime::now_utc()));
                                        if let Err(e) = link_active.update(&state.db).await {
                                            tracing::warn!(error = %e, "failed to update host_software_item installed_version");
                                        }
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
                                    code: "bad_request".to_string(),
                                    message: "unexpected message for authenticated connection".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
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
                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(_) => break,
                };
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
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
) {
    let mut push_rx = state.service_connections.register_agent(agent_id).await;
    run_agent_enrolled_loop(sink, stream, &mut push_rx, state, agent_id).await;
    state.service_connections.unregister(&agent_id).await;
}

// ---------------------------------------------------------------------------
// Shared agent enrolled loop
// ---------------------------------------------------------------------------

/// Shared enrolled loop for agents: handles Ping, RequestCertificate, and
/// push messages (Approved / Rejected).
pub(crate) async fn run_agent_enrolled_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    push_rx: &mut mpsc::Receiver<ControllerMessage>,
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
) {
    let mut approved = false;

    // Check current status to set initial approved flag.
    if let Ok(Some(agent)) = uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
        .one(&state.db)
        .await
        && agent.status == uptrakit_shared_db::entity::service::ServiceStatus::Approved
    {
        approved = true;
    }

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

                match msg {
                    Message::Text(text) => {
                        let agent_msg: ServiceMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match agent_msg {
                            ServiceMessage::Ping(PingPayload { agent_ts }) => {
                                let controller_ts = now_millis();
                                let response = ControllerMessage::Pong(PongPayload {
                                    agent_ts,
                                    controller_ts,
                                });
                                let Some(json) = serialize_msg(&response) else { break };
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                                tracing::trace!(agent_ts, controller_ts, "ping/pong (enrolled)");
                            }
                            ServiceMessage::RequestCertificate(payload) => {
                                if !approved {
                                    let err = ControllerMessage::Error(ErrorPayload {
                                        code: "not_approved".to_string(),
                                        message: "agent is not yet approved".to_string(),
                                    });
                                    if let Some(json) = serialize_msg(&err) {
                                        let _ = sink.send(Message::Text(json.into())).await;
                                    }
                                    continue;
                                }

                                // Re-fetch agent from DB
                                let agent = match uptrakit_shared_db::entity::prelude::Service::find_by_id(agent_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(a)) => a,
                                    _ => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "internal_error".to_string(),
                                            message: "agent not found".to_string(),
                                        });
                                        if let Some(json) = serialize_msg(&err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                };

                                match do_sign_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    &state.db,
                                    agent,
                                    &payload.csr_pem,
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_msg(&cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        tracing::info!(%agent_id, "certificate issued via WS");
                                        break; // close connection after certificate issuance
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "certificate_error".to_string(),
                                            message: e.current_context().to_string(),
                                        });
                                        if let Some(json) = serialize_msg(&err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                }
                            }
                            ServiceMessage::ReportHostInfo(_) => {
                                // Host linking happens at enrollment; ignore during enrolled loop
                            }
                            ServiceMessage::Enroll(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "already enrolled".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            ServiceMessage::RenewCertificate(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            ServiceMessage::VersionCheckResults(_) => {
                                // Version checks not supported during enrollment
                            }
                            ServiceMessage::UpdateStarted(_)
                            | ServiceMessage::UpdateOutput(_)
                            | ServiceMessage::UpdateResult(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "update messages not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
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
                            | ServiceMessage::ReleaseTenants(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "message type not supported on agent connections".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
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

                // Track state transitions
                match &msg {
                    ControllerMessage::Approved(_) => {
                        approved = true;
                    }
                    ControllerMessage::Rejected(_) => {
                        // Forward rejection and close
                        if let Some(json) = serialize_msg(&msg) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        break;
                    }
                    _ => {}
                }

                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(_) => break,
                };
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
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
) -> AgentWsResult<()> {
    // 1. Find host_ids linked to this agent
    let host_links = agent_host::Entity::find()
        .filter(agent_host::Column::ServiceId.eq(agent_id))
        .all(&state.db)
        .await
        .map_err(|e| report!(AgentWsError::Database(e)))?;

    if host_links.is_empty() {
        return Ok(());
    }

    let host_ids: Vec<uuid::Uuid> = host_links.iter().map(|l| l.host_id).collect();

    // 2. Query pending update_history records for those hosts
    let pending_updates = update_history::Entity::find()
        .filter(update_history::Column::HostId.is_in(host_ids))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .all(&state.db)
        .await
        .map_err(|e| report!(AgentWsError::Database(e)))?;

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
            .one(&state.db)
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

        let provider_cfg = match provider_config::Entity::find_by_id(item.provider_config_id)
            .filter(provider_config::Column::DeactivatedAt.is_null())
            .one(&state.db)
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    provider_config_id = %item.provider_config_id,
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
            crate::update_hooks::resolve_hooks(&provider_cfg.config, item.config_override.as_ref());
        let merged_config =
            crate::update_hooks::merge_config(&provider_cfg.config, item.config_override.as_ref());

        let shell = if !resolved_hooks.pre_update_commands.is_empty() {
            Some(resolved_hooks.pre_update_shell.as_str().to_string())
        } else if !resolved_hooks.post_update_commands.is_empty() {
            Some(resolved_hooks.post_update_shell.as_str().to_string())
        } else {
            None
        };

        let execute_payload = ExecuteUpdatePayload {
            update_history_id: update_record.id.to_string(),
            software_item_id: item.id.to_string(),
            software_item_name: item.name.clone(),
            package_identifier: item.package_identifier.clone(),
            to_version: update_record.to_version.clone(),
            provider_type,
            provider_config: merged_config,
            pre_update_commands: resolved_hooks.pre_update_commands,
            post_update_commands: resolved_hooks.post_update_commands,
            release_info: None,
            timeout_seconds: 300,
            shell,
        };

        let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
        let Some(json) = serialize_msg(&msg) else {
            continue;
        };

        if sink.send(Message::Text(json.into())).await.is_err() {
            return Err(report!(AgentWsError::WebSocketSend));
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
