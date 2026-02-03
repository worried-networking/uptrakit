use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use tokio::sync::mpsc;
use uptrakit_internal_wire::{
    AgentMessage, AgentSettingsPayload, ApprovedPayload, CertificatePayload, ControllerMessage,
    EnrolledPayload, ErrorPayload, ExecuteUpdatePayload, PingPayload, PongPayload, RejectedPayload,
    UpdateFinalStatus, UpdateProviderType, now_millis,
};
use uptrakit_shared_db::entity::{
    agent_host, host_software_item, provider_config, software_item, update_history,
};

/// Minimum agent version required for connection.
const MIN_AGENT_VERSION: &str = "0.0.1";

use crate::AppState;
use crate::extract::{AgentIdentity, ClientIp};
use crate::routes::agents::{
    AgentRouteError, AgentStatus, do_enroll, do_lookup_by_secret, do_sign_csr,
    find_or_create_host_and_link, revoke_certificate,
};

/// Serialize a [`ControllerMessage`] to JSON, logging on failure.
fn serialize_msg(msg: &ControllerMessage) -> Option<String> {
    match serde_json::to_string(msg) {
        Ok(json) => Some(json),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize controller message");
            None
        }
    }
}

/// Connection type determined at WebSocket upgrade time.
enum ConnectionType {
    /// mTLS client cert present → authenticated agent
    Authenticated {
        agent_id: uuid::Uuid,
        cert_serial: String,
    },
    /// Authorization: Bearer <secret> → reconnecting enrolled agent
    Enrolled(uuid::Uuid),
    /// No auth → expects Enroll message
    Anonymous,
}

pub async fn agent_ws(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<AgentIdentity>>,
    client_ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Determine connection type at upgrade time
    let conn_type = if let Some(Extension(ref id)) = identity {
        tracing::info!(agent_id = %id.agent_id, "authenticated agent WS upgrade (mTLS)");
        ConnectionType::Authenticated {
            agent_id: id.agent_id,
            cert_serial: id.cert_serial.clone(),
        }
    } else if let Some(secret) = extract_bearer(&headers) {
        match do_lookup_by_secret(&state.db, &secret).await {
            Ok(agent) => {
                tracing::info!(agent_id = %agent.id, "enrolled agent WS upgrade (bearer)");
                ConnectionType::Enrolled(agent.id)
            }
            Err(e) => {
                let ctx = e.current_context();
                let status = ctx.status_code();
                let msg = ctx.to_string();
                tracing::warn!(status = %status, "bearer auth failed: {msg}");
                return (status, msg).into_response();
            }
        }
    } else {
        tracing::info!("anonymous WS upgrade");
        ConnectionType::Anonymous
    };

    let ip = client_ip.map(|Extension(ClientIp(ip))| ip);

    ws.on_upgrade(move |socket| handle_connection(socket, state, conn_type, ip))
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

async fn handle_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    conn_type: ConnectionType,
    client_ip: Option<std::net::IpAddr>,
) {
    match conn_type {
        ConnectionType::Authenticated {
            agent_id,
            cert_serial,
        } => {
            handle_authenticated(socket, state, agent_id, cert_serial).await;
        }
        ConnectionType::Enrolled(agent_id) => {
            handle_enrolled(socket, state, agent_id).await;
        }
        ConnectionType::Anonymous => {
            handle_anonymous(socket, state, client_ip).await;
        }
    }
}

/// Authenticated path: mTLS agent, Ping/Pong keepalive loop.
async fn handle_authenticated(
    socket: WebSocket,
    state: Arc<AppState>,
    agent_id: uuid::Uuid,
    cert_serial: String,
) {
    tracing::debug!(%agent_id, "authenticated agent connected");

    let (mut sink, mut stream) = socket.split();

    // 1. Certificate validation check
    // If cert_serial is empty (proxy-forwarded without serial), use agent-id-only
    // lookup: find any non-revoked cert for this agent.
    let cert_record = if cert_serial.is_empty() {
        match uptrakit_shared_db::entity::prelude::AgentCertificate::find()
            .filter(uptrakit_shared_db::entity::agent_certificate::Column::AgentId.eq(agent_id))
            .filter(uptrakit_shared_db::entity::agent_certificate::Column::RevokedAt.is_null())
            .order_by_desc(uptrakit_shared_db::entity::agent_certificate::Column::CreatedAt)
            .one(&state.db)
            .await
        {
            Ok(Some(record)) => {
                tracing::warn!(
                    %agent_id,
                    "agent connected via proxy without cert serial, using agent-id-only lookup"
                );
                record
            }
            Ok(None) => {
                tracing::warn!(
                    %agent_id,
                    "rejected connection: no non-revoked certificate found for agent"
                );
                let _ = close_with_reason(&mut sink, "no valid certificate").await;
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "certificate validation check failed");
                let _ = close_with_reason(&mut sink, "internal error").await;
                return;
            }
        }
    } else {
        // Standard lookup: query by (agent_id, serial)
        match uptrakit_shared_db::entity::prelude::AgentCertificate::find()
            .filter(
                uptrakit_shared_db::entity::agent_certificate::Column::SerialNumber
                    .eq(cert_serial.clone()),
            )
            .filter(uptrakit_shared_db::entity::agent_certificate::Column::AgentId.eq(agent_id))
            .one(&state.db)
            .await
        {
            Ok(Some(record)) => {
                if record.revoked_at.is_some() {
                    tracing::warn!(
                        %agent_id,
                        serial_number = %cert_serial,
                        "rejected connection: certificate is revoked"
                    );
                    let _ = close_with_reason(&mut sink, "certificate revoked").await;
                    return;
                }
                record
            }
            Ok(None) => {
                tracing::warn!(
                    %agent_id,
                    serial_number = %cert_serial,
                    "rejected connection: certificate not recognized"
                );
                let _ = close_with_reason(&mut sink, "certificate not recognized").await;
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "certificate validation check failed");
                let _ = close_with_reason(&mut sink, "internal error").await;
                return;
            }
        }
    };

    // 2. Agent status check
    match uptrakit_shared_db::entity::prelude::Agent::find_by_id(agent_id)
        .one(&state.db)
        .await
    {
        Ok(Some(agent)) => {
            if agent.deactivated_at.is_some() {
                tracing::warn!(%agent_id, "deactivated agent connected with valid certificate");
                let _ = close_with_reason(&mut sink, "agent deactivated").await;
                return;
            }

            if agent.status != AgentStatus::Approved.as_str() {
                tracing::warn!(%agent_id, "rejected connection: agent not approved");
                let _ = close_with_reason(&mut sink, "agent not approved").await;
                return;
            }
        }
        Ok(None) => {
            tracing::warn!(%agent_id, "rejected connection: agent not found");
            let _ = close_with_reason(&mut sink, "agent not found").await;
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "agent status check failed");
            let _ = close_with_reason(&mut sink, "internal error").await;
            return;
        }
    }

    // Save CA fingerprint before moving cert_record
    let cert_ca_fingerprint = cert_record.ca_fingerprint.clone();

    // Record certificate usage
    let mut active: uptrakit_shared_db::entity::agent_certificate::ActiveModel = cert_record.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    if let Err(e) = active.update(&state.db).await {
        tracing::error!(error = %e, "failed to update certificate last_seen_at");
    }

    // Send AgentSettings on connect
    let renewal_window_hours = state.settings.renewal_window_hours().await;
    let ca_bundle_hash = state.ca_snapshot.borrow().bundle_hash.clone();
    let settings_msg = ControllerMessage::AgentSettings(AgentSettingsPayload {
        renewal_window_hours,
        ca_bundle_hash,
    });
    let Some(json) = serialize_msg(&settings_msg) else {
        return;
    };
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    // Deliver pending updates for hosts linked to this agent
    if let Err(e) = deliver_pending_updates(&state, agent_id, &mut sink).await {
        tracing::error!(error = %e, %agent_id, "failed to deliver pending updates on reconnect");
    }

    let mut push_rx = state.agent_connections.register(agent_id).await;

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
                        let agent_msg: AgentMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match agent_msg {
                            AgentMessage::Ping(PingPayload { agent_ts }) => {
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
                            AgentMessage::ReportHostInfo(payload) => {
                                // Check agent version
                                let agent_ver = match semver::Version::parse(&payload.agent_version) {
                                    Ok(v) => v,
                                    Err(_) => {
                                        tracing::warn!(
                                            %agent_id,
                                            version = %payload.agent_version,
                                            "agent sent invalid version string"
                                        );
                                        // Treat invalid version as too old
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
                                        let _ = close_with_reason(&mut sink, "agent version too old").await;
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
                                    let _ = close_with_reason(&mut sink, "agent version too old").await;
                                    break;
                                }

                                // Look up agent hostname from DB for host linking
                                let agent_model = match uptrakit_shared_db::entity::prelude::Agent::find_by_id(agent_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(a)) => a,
                                    _ => continue,
                                };

                                // Update agent_version in database
                                let mut active: uptrakit_shared_db::entity::agent::ActiveModel = agent_model.clone().into();
                                active.agent_version = Set(payload.agent_version.clone());
                                active.updated_at = Set(time::OffsetDateTime::now_utc());
                                if let Err(e) = active.update(&state.db).await {
                                    tracing::error!(error = %e, "failed to update agent_version");
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
                            AgentMessage::RenewCertificate(payload) => {
                                // Re-fetch agent from DB, verify still approved
                                let agent = match uptrakit_shared_db::entity::prelude::Agent::find_by_id(agent_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(a)) if a.status == AgentStatus::Approved.as_str() && a.deactivated_at.is_none() => a,
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
                                        let _ = close_with_reason(&mut sink, "certificate rotated").await;
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
                            AgentMessage::VersionCheckResults(payload) => {
                                tracing::debug!(%agent_id, count = payload.results.len(), "received VersionCheckResults");

                                // Look up hosts linked to this agent
                                let host_ids: Vec<uuid::Uuid> = match uptrakit_shared_db::entity::prelude::AgentHost::find()
                                    .filter(uptrakit_shared_db::entity::agent_host::Column::AgentId.eq(agent_id))
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
                                    // Skip results with errors
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

                                    // Parse software_item_id as UUID
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

                                    // Update host_software_items for each linked host
                                    for &host_id in &host_ids {
                                        // Check if record exists
                                        match uptrakit_shared_db::entity::prelude::HostSoftwareItem::find_by_id((host_id, software_item_id))
                                            .one(&state.db)
                                            .await
                                        {
                                            Ok(Some(existing)) => {
                                                // Update existing record
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
                                                // No record exists - skip (don't create unlinked records)
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
                            AgentMessage::UpdateStarted(payload) => {
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    from_version = ?payload.from_version,
                                    "update started"
                                );
                                // Update status to InProgress and from_version
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
                            AgentMessage::UpdateOutput(payload) => {
                                tracing::trace!(
                                    update_id = %payload.update_history_id,
                                    stream = ?payload.stream,
                                    "update output"
                                );
                                // Append output to update_history.output
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
                            AgentMessage::UpdateResult(payload) => {
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
                                    // Append final output
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

                                    // On success, update host_software_item.installed_version
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

    state.agent_connections.unregister(&agent_id).await;
    tracing::debug!(%agent_id, "authenticated agent disconnected");
}

/// Enrolled path: agent reconnecting with Bearer secret, waiting for approval.
async fn handle_enrolled(socket: WebSocket, state: Arc<AppState>, agent_id: uuid::Uuid) {
    tracing::debug!(%agent_id, "enrolled agent connected (bearer)");
    let mut push_rx = state.agent_connections.register(agent_id).await;

    // Check current status — if already approved/rejected, push immediately
    let agent = match uptrakit_shared_db::entity::prelude::Agent::find_by_id(agent_id)
        .one(&state.db)
        .await
    {
        Ok(Some(a)) => a,
        Ok(None) => {
            tracing::warn!(%agent_id, "agent not found in DB");
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "DB lookup failed");
            return;
        }
    };

    let status = AgentStatus::from_str(&agent.status);

    let (mut sink, mut stream) = socket.split();

    // If already approved/rejected, push immediately
    match status {
        Some(AgentStatus::Approved) => {
            let msg = ControllerMessage::Approved(ApprovedPayload {
                agent_id: agent_id.to_string(),
            });
            let Some(json) = serialize_msg(&msg) else {
                state.agent_connections.unregister(&agent_id).await;
                return;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                state.agent_connections.unregister(&agent_id).await;
                return;
            }
        }
        Some(AgentStatus::Rejected) => {
            let msg = ControllerMessage::Rejected(RejectedPayload {
                agent_id: agent_id.to_string(),
            });
            if let Some(json) = serialize_msg(&msg) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            state.agent_connections.unregister(&agent_id).await;
            return;
        }
        _ => {
            // Pending — wait for push
        }
    }

    // Enter enrolled loop
    run_enrolled_loop(&mut sink, &mut stream, &mut push_rx, &state, agent_id).await;

    state.agent_connections.unregister(&agent_id).await;
    tracing::debug!(%agent_id, "enrolled agent disconnected");
}

/// Anonymous path: expects Enroll message, then promotes in-place.
async fn handle_anonymous(
    socket: WebSocket,
    state: Arc<AppState>,
    client_ip: Option<std::net::IpAddr>,
) {
    tracing::debug!("anonymous agent connected");

    let (mut sink, mut stream) = socket.split();

    // Wait for first message — must be Enroll
    let agent_id = loop {
        let msg = match stream.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                tracing::debug!(error = %e, "websocket receive error");
                return;
            }
            None => return,
        };

        match msg {
            Message::Text(text) => {
                let agent_msg: AgentMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: format!("invalid message: {e}"),
                        });
                        if let Some(json) = serialize_msg(&err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        return;
                    }
                };

                match agent_msg {
                    AgentMessage::Enroll(payload) => {
                        let result = do_enroll(
                            &state.db,
                            &state.settings,
                            state.default_tenant_id,
                            &payload.client_id,
                            &payload.hostname,
                            &payload.friendly_name,
                            payload.enrollment_token.as_deref(),
                            client_ip,
                            Some(&payload.host_info),
                        )
                        .await;

                        match result {
                            Ok(enroll_result) => {
                                let agent_id = enroll_result.agent.id;
                                let enrolled_msg = ControllerMessage::Enrolled(EnrolledPayload {
                                    agent_id: agent_id.to_string(),
                                    enrollment_secret: enroll_result.enrollment_secret,
                                    status: enroll_result.status.as_str().to_string(),
                                });
                                let Some(json) = serialize_msg(&enrolled_msg) else {
                                    return;
                                };
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    return;
                                }

                                tracing::info!(
                                    %agent_id,
                                    status = enroll_result.status.as_str(),
                                    "agent enrolled via WS"
                                );

                                // If auto-approved (valid enrollment token), push Approved
                                if enroll_result.status == AgentStatus::Approved {
                                    let approved_msg =
                                        ControllerMessage::Approved(ApprovedPayload {
                                            agent_id: agent_id.to_string(),
                                        });
                                    let Some(json) = serialize_msg(&approved_msg) else {
                                        return;
                                    };
                                    if sink.send(Message::Text(json.into())).await.is_err() {
                                        return;
                                    }
                                }

                                break agent_id;
                            }
                            Err(e) => {
                                let (code, message) = match e.current_context() {
                                    AgentRouteError::ClientIdCollision => (
                                        "client_id_collision".to_string(),
                                        "client_id already exists".to_string(),
                                    ),
                                    other => ("enrollment_failed".to_string(), other.to_string()),
                                };
                                let err = ControllerMessage::Error(ErrorPayload { code, message });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                                return;
                            }
                        }
                    }
                    _ => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: "expected enroll message".to_string(),
                        });
                        if let Some(json) = serialize_msg(&err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        return;
                    }
                }
            }
            Message::Close(_) => return,
            _ => {}
        }
    };

    // Connection promoted: register in connection registry
    let mut push_rx = state.agent_connections.register(agent_id).await;

    // Enter enrolled loop
    run_enrolled_loop(&mut sink, &mut stream, &mut push_rx, &state, agent_id).await;

    state.agent_connections.unregister(&agent_id).await;
    tracing::debug!(%agent_id, "anonymous->enrolled agent disconnected");
}

/// Shared enrolled loop: handles Ping, RequestCertificate, and push messages.
async fn run_enrolled_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    push_rx: &mut mpsc::Receiver<ControllerMessage>,
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
) {
    let mut approved = false;

    // Check current status to set initial approved flag
    if let Ok(Some(agent)) = uptrakit_shared_db::entity::prelude::Agent::find_by_id(agent_id)
        .one(&state.db)
        .await
        && agent.status == AgentStatus::Approved.as_str()
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
                        let agent_msg: AgentMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match agent_msg {
                            AgentMessage::Ping(PingPayload { agent_ts }) => {
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
                            AgentMessage::RequestCertificate(payload) => {
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
                                let agent = match uptrakit_shared_db::entity::prelude::Agent::find_by_id(agent_id)
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
                            AgentMessage::ReportHostInfo(_) => {
                                // Host linking happens at enrollment; ignore during enrolled loop
                            }
                            AgentMessage::Enroll(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "already enrolled".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            AgentMessage::RenewCertificate(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            AgentMessage::VersionCheckResults(_) => {
                                // Version checks not supported during enrollment
                            }
                            // Update messages are only valid for authenticated connections
                            AgentMessage::UpdateStarted(_)
                            | AgentMessage::UpdateOutput(_)
                            | AgentMessage::UpdateResult(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "update messages not available during enrollment".to_string(),
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

async fn close_with_reason(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    reason: &str,
) -> Result<(), axum::Error> {
    sink.send(Message::Close(Some(CloseFrame {
        code: axum::extract::ws::close_code::POLICY,
        reason: reason.into(),
    })))
    .await
}

/// Deliver pending updates for hosts linked to this agent.
///
/// On agent reconnect, we check for any `update_history` records with `status = Pending`
/// for hosts linked to this agent and send them to the agent.
async fn deliver_pending_updates(
    state: &Arc<AppState>,
    agent_id: uuid::Uuid,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<(), String> {
    // 1. Find host_ids linked to this agent
    let host_links = agent_host::Entity::find()
        .filter(agent_host::Column::AgentId.eq(agent_id))
        .all(&state.db)
        .await
        .map_err(|e| format!("failed to find agent hosts: {e}"))?;

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
        .map_err(|e| format!("failed to find pending updates: {e}"))?;

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
        // Load software item
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

        // Load provider config
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

        // Convert provider type
        let provider_type = match provider_cfg.provider_type.as_str() {
            "github_releases" => UpdateProviderType::GithubReleases,
            "proxmox_helper_scripts" => UpdateProviderType::ProxmoxHelperScripts,
            "docker_registry" => UpdateProviderType::DockerRegistry,
            other => {
                tracing::warn!(
                    update_id = %update_record.id,
                    provider_type = other,
                    "unknown provider type, skipping pending update"
                );
                continue;
            }
        };

        // Merge hooks and config
        let (pre_update_commands, post_update_commands) =
            crate::update_hooks::merge_hooks(&provider_cfg.config, item.config_override.as_ref());
        let merged_config =
            crate::update_hooks::merge_config(&provider_cfg.config, item.config_override.as_ref());

        // Build payload
        let execute_payload = ExecuteUpdatePayload {
            update_history_id: update_record.id.to_string(),
            software_item_id: item.id.to_string(),
            software_item_name: item.name.clone(),
            package_identifier: item.package_identifier.clone(),
            to_version: update_record.to_version.clone(),
            provider_type,
            provider_config: merged_config,
            pre_update_commands,
            post_update_commands,
            release_info: None, // Not stored in update_history
            timeout_seconds: 300,
        };

        // Send to agent
        let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
        let Some(json) = serialize_msg(&msg) else {
            continue;
        };

        if sink.send(Message::Text(json.into())).await.is_err() {
            return Err("websocket send failed".to_string());
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
