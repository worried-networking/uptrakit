use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tokio::sync::mpsc;
use uptrakit_internal_wire::{
    AgentMessage, AgentSettingsPayload, ApprovedPayload, CertificatePayload, ControllerMessage,
    EnrolledPayload, ErrorPayload, PingPayload, PongPayload, RejectedPayload, now_millis,
};

use crate::AppState;
use crate::extract::{AgentIdentity, ClientIp};
use crate::routes::agents::{
    AgentStatus, do_enroll, do_lookup_by_secret, do_sign_certificate, revoke_certificate,
};

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
            Err((status, msg)) => {
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

    // 1. Certificate validation check — query by (agent_id, serial) since
    // the composite PK is (ca_fingerprint, serial_number) and we don't know
    // the CA fingerprint at this point.
    let cert_record = match uptrakit_shared_db::entity::prelude::AgentCertificate::find()
        .filter(uptrakit_shared_db::entity::agent_certificate::Column::SerialNumber.eq(cert_serial.clone()))
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
    let json = serde_json::to_string(&settings_msg).unwrap();
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
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
                                let json = serde_json::to_string(&response).unwrap();
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            }
                            AgentMessage::RenewCertificate(_) => {
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
                                        let json = serde_json::to_string(&err).unwrap();
                                        let _ = sink.send(Message::Text(json.into())).await;
                                        break;
                                    }
                                };

                                // Sign new certificate
                                match do_sign_certificate(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    &state.db,
                                    agent,
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            key_pem: bundle.key_pem,
                                            not_after: bundle.not_after,
                                        });
                                        let json = serde_json::to_string(&cert_msg).unwrap();
                                        let _ = sink.send(Message::Text(json.into())).await;

                                        // Revoke old cert
                                        if let Err(e) = revoke_certificate(&state.db, &cert_serial, &cert_ca_fingerprint, uptrakit_shared_db::entity::prelude::RevocationReason::CertificateRenewed).await {
                                            tracing::error!(error = %e, "failed to revoke old certificate");
                                        }

                                        state.revocation_notify.notify_one();
                                        tracing::info!(%agent_id, old_serial = %cert_serial, "certificate renewed, old cert revoked");
                                        let _ = close_with_reason(&mut sink, "certificate rotated").await;
                                        break;
                                    }
                                    Err((_status, msg)) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "certificate_error".to_string(),
                                            message: msg.to_string(),
                                        });
                                        let json = serde_json::to_string(&err).unwrap();
                                        let _ = sink.send(Message::Text(json.into())).await;
                                        break;
                                    }
                                }
                            }
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "unexpected message for authenticated connection".to_string(),
                                });
                                let json = serde_json::to_string(&err).unwrap();
                                let _ = sink.send(Message::Text(json.into())).await;
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
            let json = serde_json::to_string(&msg).unwrap();
            if sink.send(Message::Text(json.into())).await.is_err() {
                state.agent_connections.unregister(&agent_id).await;
                return;
            }
        }
        Some(AgentStatus::Rejected) => {
            let msg = ControllerMessage::Rejected(RejectedPayload {
                agent_id: agent_id.to_string(),
            });
            let json = serde_json::to_string(&msg).unwrap();
            let _ = sink.send(Message::Text(json.into())).await;
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
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&err).unwrap().into()))
                            .await;
                        return;
                    }
                };

                match agent_msg {
                    AgentMessage::Enroll(payload) => {
                        let result = do_enroll(
                            &state.db,
                            &state.settings,
                            &payload.hostname,
                            &payload.friendly_name,
                            payload.enrollment_token.as_deref(),
                            client_ip,
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
                                let json = serde_json::to_string(&enrolled_msg).unwrap();
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
                                    let json = serde_json::to_string(&approved_msg).unwrap();
                                    if sink.send(Message::Text(json.into())).await.is_err() {
                                        return;
                                    }
                                }

                                break agent_id;
                            }
                            Err((_status, msg)) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "enrollment_failed".to_string(),
                                    message: msg.to_string(),
                                });
                                let _ = sink
                                    .send(Message::Text(
                                        serde_json::to_string(&err).unwrap().into(),
                                    ))
                                    .await;
                                return;
                            }
                        }
                    }
                    _ => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: "expected enroll message".to_string(),
                        });
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&err).unwrap().into()))
                            .await;
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
                                let json = serde_json::to_string(&response).unwrap();
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                                tracing::trace!(agent_ts, controller_ts, "ping/pong (enrolled)");
                            }
                            AgentMessage::RequestCertificate(_) => {
                                if !approved {
                                    let err = ControllerMessage::Error(ErrorPayload {
                                        code: "not_approved".to_string(),
                                        message: "agent is not yet approved".to_string(),
                                    });
                                    let json = serde_json::to_string(&err).unwrap();
                                    let _ = sink.send(Message::Text(json.into())).await;
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
                                        let json = serde_json::to_string(&err).unwrap();
                                        let _ = sink.send(Message::Text(json.into())).await;
                                        break;
                                    }
                                };

                                match do_sign_certificate(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    &state.db,
                                    agent,
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            key_pem: bundle.key_pem,
                                            not_after: bundle.not_after,
                                        });
                                        let json = serde_json::to_string(&cert_msg).unwrap();
                                        let _ = sink.send(Message::Text(json.into())).await;
                                        tracing::info!(%agent_id, "certificate issued via WS");
                                        break; // close connection after certificate issuance
                                    }
                                    Err((_status, msg)) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "certificate_error".to_string(),
                                            message: msg.to_string(),
                                        });
                                        let json = serde_json::to_string(&err).unwrap();
                                        let _ = sink.send(Message::Text(json.into())).await;
                                        break;
                                    }
                                }
                            }
                            AgentMessage::Enroll(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "already enrolled".to_string(),
                                });
                                let json = serde_json::to_string(&err).unwrap();
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                            AgentMessage::RenewCertificate(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "bad_request".to_string(),
                                    message: "not available during enrollment".to_string(),
                                });
                                let json = serde_json::to_string(&err).unwrap();
                                let _ = sink.send(Message::Text(json.into())).await;
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
                        let json = serde_json::to_string(&msg).unwrap();
                        let _ = sink.send(Message::Text(json.into())).await;
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
