use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use uptrakit_internal_wire::{
    CertificatePayload, ErrorPayload, MqttApprovedPayload, MqttControllerMessage,
    MqttEnrolledPayload, MqttRegisteredPayload, MqttRejectedPayload, MqttServiceMessage,
    MqttServiceSettingsPayload, MqttTenantAssignmentsPayload, PingPayload, PongPayload, now_millis,
};
use uptrakit_shared_db::entity::{mqtt_service, mqtt_service_certificate};

use crate::AppState;
use crate::extract::{ClientIp, MqttServiceIdentity};
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;

/// Serialize a [`MqttControllerMessage`] to JSON, logging on failure.
fn serialize_msg(msg: &MqttControllerMessage) -> Option<String> {
    match serde_json::to_string(msg) {
        Ok(json) => Some(json),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize mqtt controller message");
            None
        }
    }
}

/// Connection type determined at WebSocket upgrade time.
enum ConnectionType {
    /// mTLS client cert present → authenticated MQTT service
    Authenticated {
        service_id: uuid::Uuid,
        cert_serial: String,
    },
    /// Authorization: Bearer <secret> → reconnecting enrolled service
    Enrolled(uuid::Uuid),
    /// No auth → expects Enroll message
    Anonymous,
}

pub async fn mqtt_ws(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<MqttServiceIdentity>>,
    client_ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Determine connection type at upgrade time
    let conn_type = if let Some(Extension(ref id)) = identity {
        tracing::info!(service_id = %id.service_id, "authenticated MQTT service WS upgrade (mTLS)");
        ConnectionType::Authenticated {
            service_id: id.service_id,
            cert_serial: id.cert_serial.clone(),
        }
    } else if let Some(secret) = extract_bearer(&headers) {
        match do_lookup_by_secret(&state.db, &secret).await {
            Ok(service) => {
                tracing::info!(service_id = %service.id, "enrolled MQTT service WS upgrade (bearer)");
                ConnectionType::Enrolled(service.id)
            }
            Err(msg) => {
                tracing::warn!("bearer auth failed: {msg}");
                return (axum::http::StatusCode::UNAUTHORIZED, msg).into_response();
            }
        }
    } else {
        tracing::info!("anonymous MQTT service WS upgrade");
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

/// Look up MQTT service by enrollment secret.
async fn do_lookup_by_secret(
    db: &sea_orm::DatabaseConnection,
    secret: &str,
) -> Result<mqtt_service::Model, String> {
    use uptrakit_shared_db::entity::prelude::MqttService;

    // Find all non-deactivated MQTT services
    let services = MqttService::find()
        .filter(mqtt_service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    // Verify secret against each service's hash
    for service in services {
        if let Ok(true) =
            crate::auth::password::verify_password(secret, &service.enrollment_secret_hash)
        {
            return Ok(service);
        }
    }

    Err("invalid enrollment secret".to_string())
}

async fn handle_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    conn_type: ConnectionType,
    client_ip: Option<std::net::IpAddr>,
) {
    match conn_type {
        ConnectionType::Authenticated {
            service_id,
            cert_serial,
        } => {
            handle_authenticated(socket, state, service_id, cert_serial).await;
        }
        ConnectionType::Enrolled(service_id) => {
            handle_enrolled(socket, state, service_id).await;
        }
        ConnectionType::Anonymous => {
            handle_anonymous(socket, state, client_ip).await;
        }
    }
}

/// Authenticated path: mTLS MQTT service, operational loop.
async fn handle_authenticated(
    socket: WebSocket,
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    cert_serial: String,
) {
    tracing::debug!(%service_id, "authenticated MQTT service connected");

    let (mut sink, mut stream) = socket.split();

    // 1. Certificate validation check
    let cert_record = if cert_serial.is_empty() {
        match mqtt_service_certificate::Entity::find()
            .filter(mqtt_service_certificate::Column::MqttServiceId.eq(service_id))
            .filter(mqtt_service_certificate::Column::RevokedAt.is_null())
            .order_by_desc(mqtt_service_certificate::Column::CreatedAt)
            .one(&state.db)
            .await
        {
            Ok(Some(record)) => {
                tracing::warn!(
                    %service_id,
                    "MQTT service connected via proxy without cert serial, using service-id-only lookup"
                );
                record
            }
            Ok(None) => {
                tracing::warn!(
                    %service_id,
                    "rejected connection: no non-revoked certificate found for MQTT service"
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
        match mqtt_service_certificate::Entity::find()
            .filter(mqtt_service_certificate::Column::SerialNumber.eq(cert_serial.clone()))
            .filter(mqtt_service_certificate::Column::MqttServiceId.eq(service_id))
            .one(&state.db)
            .await
        {
            Ok(Some(record)) => {
                if record.revoked_at.is_some() {
                    tracing::warn!(
                        %service_id,
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
                    %service_id,
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

    // 2. Service status check
    match mqtt_service::Entity::find_by_id(service_id)
        .one(&state.db)
        .await
    {
        Ok(Some(service)) => {
            if service.deactivated_at.is_some() {
                tracing::warn!(%service_id, "deactivated MQTT service connected with valid certificate");
                let _ = close_with_reason(&mut sink, "service deactivated").await;
                return;
            }

            if service.status != mqtt_service::MqttServiceStatus::Approved {
                tracing::warn!(%service_id, "rejected connection: MQTT service not approved");
                let _ = close_with_reason(&mut sink, "service not approved").await;
                return;
            }
        }
        Ok(None) => {
            tracing::warn!(%service_id, "rejected connection: MQTT service not found");
            let _ = close_with_reason(&mut sink, "service not found").await;
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "MQTT service status check failed");
            let _ = close_with_reason(&mut sink, "internal error").await;
            return;
        }
    }

    // Save CA fingerprint before moving cert_record
    let cert_ca_fingerprint = cert_record.ca_fingerprint.clone();

    // Record certificate usage
    let mut active: mqtt_service_certificate::ActiveModel = cert_record.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    if let Err(e) = active.update(&state.db).await {
        tracing::error!(error = %e, "failed to update certificate last_seen_at");
    }

    // Send MqttServiceSettings on connect
    let renewal_window_hours = state.settings.renewal_window_hours().await;
    let ca_bundle_hash = state.ca_snapshot.borrow().bundle_hash.clone();
    let settings_msg = MqttControllerMessage::MqttServiceSettings(MqttServiceSettingsPayload {
        renewal_window_hours,
        ca_bundle_hash,
    });
    let Some(json) = serialize_msg(&settings_msg) else {
        return;
    };
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    // Wait for Register message before entering operational loop
    let (instance_id, max_tenants, active_tenants) = loop {
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
                let service_msg: MqttServiceMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "deserialize error");
                        return;
                    }
                };

                match service_msg {
                    MqttServiceMessage::Register(payload) => {
                        break (
                            payload.instance_id,
                            payload.max_tenants,
                            payload.active_tenants,
                        );
                    }
                    MqttServiceMessage::Ping(PingPayload { agent_ts }) => {
                        let controller_ts = now_millis();
                        let response = MqttControllerMessage::Pong(PongPayload {
                            agent_ts,
                            controller_ts,
                        });
                        let Some(json) = serialize_msg(&response) else {
                            return;
                        };
                        if sink.send(Message::Text(json.into())).await.is_err() {
                            return;
                        }
                    }
                    _ => {
                        let err = MqttControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: "expected register message".to_string(),
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

    // Register in connection registry
    let mut push_rx = state
        .mqtt_service_connections
        .register(service_id, instance_id.clone(), max_tenants)
        .await;

    // Send Registered acknowledgment
    let registered_msg = MqttControllerMessage::Registered(MqttRegisteredPayload {
        instance_id: instance_id.clone(),
    });
    let Some(json) = serialize_msg(&registered_msg) else {
        state.mqtt_service_connections.unregister(&service_id).await;
        return;
    };
    if sink.send(Message::Text(json.into())).await.is_err() {
        state.mqtt_service_connections.unregister(&service_id).await;
        return;
    }

    // Create lease coordinator
    let lease_coordinator =
        MqttLeaseCoordinator::new(state.db.clone(), state.mqtt_service_connections.clone());

    // Reconcile tenants if reconnecting with active tenants
    let tenant_configs = if !active_tenants.is_empty() {
        let tenant_ids: Vec<uuid::Uuid> = active_tenants
            .iter()
            .filter_map(|s| uuid::Uuid::parse_str(s).ok())
            .collect();

        match lease_coordinator
            .reconcile_tenants(service_id, &instance_id, &tenant_ids)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to reconcile tenants");
                vec![]
            }
        }
    } else {
        // Fresh start - assign available tenants
        // For now, claim up to max_tenants (or unlimited if max_tenants = 0)
        let requested = if max_tenants == 0 { 100 } else { max_tenants };
        match lease_coordinator
            .assign_available_tenants(service_id, &instance_id, requested)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to assign tenants");
                vec![]
            }
        }
    };

    // Send initial tenant assignments
    if !tenant_configs.is_empty() {
        let assignments_msg =
            MqttControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload {
                tenants: tenant_configs,
            });
        let Some(json) = serialize_msg(&assignments_msg) else {
            state.mqtt_service_connections.unregister(&service_id).await;
            return;
        };
        if sink.send(Message::Text(json.into())).await.is_err() {
            state.mqtt_service_connections.unregister(&service_id).await;
            return;
        }
    }

    tracing::info!(%service_id, instance_id = %instance_id, "MQTT service registered");

    // Enter operational loop
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
                        let service_msg: MqttServiceMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match service_msg {
                            MqttServiceMessage::Ping(PingPayload { agent_ts }) => {
                                let controller_ts = now_millis();
                                tracing::trace!(agent_ts, controller_ts, "ping/pong");
                                let response = MqttControllerMessage::Pong(PongPayload {
                                    agent_ts,
                                    controller_ts,
                                });
                                let Some(json) = serialize_msg(&response) else { break };
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            }
                            MqttServiceMessage::Heartbeat(payload) => {
                                let tenant_ids: Vec<uuid::Uuid> = payload
                                    .active_tenants
                                    .iter()
                                    .filter_map(|s| uuid::Uuid::parse_str(s).ok())
                                    .collect();

                                if let Err(e) = lease_coordinator
                                    .record_heartbeat(&service_id, &tenant_ids)
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to record heartbeat");
                                }
                            }
                            MqttServiceMessage::ReleaseTenants(payload) => {
                                let tenant_ids: Vec<uuid::Uuid> = payload
                                    .tenant_ids
                                    .iter()
                                    .filter_map(|s| uuid::Uuid::parse_str(s).ok())
                                    .collect();

                                if let Err(e) = lease_coordinator
                                    .release_tenants(&service_id, &tenant_ids)
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to release tenants");
                                }

                                tracing::info!(
                                    %service_id,
                                    count = tenant_ids.len(),
                                    "MQTT service released tenants"
                                );
                            }
                            MqttServiceMessage::RenewCertificate(payload) => {
                                // Re-fetch service from DB, verify still approved
                                let service = match mqtt_service::Entity::find_by_id(service_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(s)) if s.status == mqtt_service::MqttServiceStatus::Approved && s.deactivated_at.is_none() => s,
                                    _ => {
                                        let err = MqttControllerMessage::Error(ErrorPayload {
                                            code: "forbidden".to_string(),
                                            message: "service is not approved".to_string(),
                                        });
                                        if let Some(json) = serialize_msg(&err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                };

                                // Sign new certificate from service's CSR
                                match do_sign_mqtt_service_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    &state.db,
                                    service,
                                    &payload.csr_pem,
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = MqttControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_msg(&cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }

                                        // Revoke old cert
                                        if let Err(e) = revoke_mqtt_service_certificate(
                                            &state.db,
                                            &cert_serial,
                                            &cert_ca_fingerprint,
                                            mqtt_service_certificate::MqttServiceCertificateRevocationReason::CertificateRenewed,
                                        ).await {
                                            tracing::error!(error = %e, "failed to revoke old certificate");
                                        }

                                        state.revocation_notify.notify_one();
                                        tracing::info!(%service_id, old_serial = %cert_serial, "MQTT service certificate renewed, old cert revoked");
                                        let _ = close_with_reason(&mut sink, "certificate rotated").await;
                                        break;
                                    }
                                    Err(e) => {
                                        let err = MqttControllerMessage::Error(ErrorPayload {
                                            code: "certificate_error".to_string(),
                                            message: e,
                                        });
                                        if let Some(json) = serialize_msg(&err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                }
                            }
                            MqttServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "MQTT service disconnecting gracefully"
                                );
                                break;
                            }
                            _ => {
                                let err = MqttControllerMessage::Error(ErrorPayload {
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

    // Release all leases on disconnect
    if let Err(e) = lease_coordinator.release_all_for_service(&service_id).await {
        tracing::error!(error = %e, "failed to release leases on disconnect");
    }

    state.mqtt_service_connections.unregister(&service_id).await;
    tracing::debug!(%service_id, "authenticated MQTT service disconnected");
}

/// Enrolled path: service reconnecting with Bearer secret, waiting for approval.
async fn handle_enrolled(socket: WebSocket, state: Arc<AppState>, service_id: uuid::Uuid) {
    tracing::debug!(%service_id, "enrolled MQTT service connected (bearer)");

    // Check current status
    let service = match mqtt_service::Entity::find_by_id(service_id)
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!(%service_id, "MQTT service not found in DB");
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "DB lookup failed");
            return;
        }
    };

    let (mut sink, mut stream) = socket.split();

    // If already approved/rejected, push immediately
    match service.status {
        mqtt_service::MqttServiceStatus::Approved => {
            let msg = MqttControllerMessage::Approved(MqttApprovedPayload {
                service_id: service_id.to_string(),
            });
            let Some(json) = serialize_msg(&msg) else {
                return;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
        mqtt_service::MqttServiceStatus::Rejected => {
            let msg = MqttControllerMessage::Rejected(MqttRejectedPayload {
                service_id: service_id.to_string(),
            });
            if let Some(json) = serialize_msg(&msg) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            return;
        }
        _ => {
            // Pending — wait for approval via polling
        }
    }

    let approved = service.status == mqtt_service::MqttServiceStatus::Approved;
    run_enrolled_loop(&mut sink, &mut stream, &state, service_id, approved).await;
    tracing::debug!(%service_id, "enrolled MQTT service disconnected");
}

/// Shared enrolled loop: handles Ping, RequestCertificate, and polls for approval.
async fn run_enrolled_loop(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    mut approved: bool,
) {
    loop {
        let msg = match stream.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                tracing::debug!(error = %e, "websocket receive error");
                break;
            }
            None => break,
        };

        match msg {
            Message::Text(text) => {
                let service_msg: MqttServiceMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "deserialize error");
                        break;
                    }
                };

                match service_msg {
                    MqttServiceMessage::Ping(PingPayload { agent_ts }) => {
                        let controller_ts = now_millis();
                        let response = MqttControllerMessage::Pong(PongPayload {
                            agent_ts,
                            controller_ts,
                        });
                        let Some(json) = serialize_msg(&response) else {
                            break;
                        };
                        if sink.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                        tracing::trace!(agent_ts, controller_ts, "ping/pong (enrolled)");

                        // Poll database for status change (simplified)
                        if !approved
                            && let Ok(Some(s)) = mqtt_service::Entity::find_by_id(service_id)
                                .one(&state.db)
                                .await
                        {
                            match s.status {
                                mqtt_service::MqttServiceStatus::Approved => {
                                    approved = true;
                                    let msg =
                                        MqttControllerMessage::Approved(MqttApprovedPayload {
                                            service_id: service_id.to_string(),
                                        });
                                    if let Some(json) = serialize_msg(&msg) {
                                        let _ = sink.send(Message::Text(json.into())).await;
                                    }
                                }
                                mqtt_service::MqttServiceStatus::Rejected => {
                                    let msg =
                                        MqttControllerMessage::Rejected(MqttRejectedPayload {
                                            service_id: service_id.to_string(),
                                        });
                                    if let Some(json) = serialize_msg(&msg) {
                                        let _ = sink.send(Message::Text(json.into())).await;
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    MqttServiceMessage::RequestCertificate(payload) => {
                        if !approved {
                            let err = MqttControllerMessage::Error(ErrorPayload {
                                code: "not_approved".to_string(),
                                message: "service is not yet approved".to_string(),
                            });
                            if let Some(json) = serialize_msg(&err) {
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                            continue;
                        }

                        // Re-fetch service from DB
                        let service = match mqtt_service::Entity::find_by_id(service_id)
                            .one(&state.db)
                            .await
                        {
                            Ok(Some(s)) => s,
                            _ => {
                                let err = MqttControllerMessage::Error(ErrorPayload {
                                    code: "internal_error".to_string(),
                                    message: "service not found".to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                                break;
                            }
                        };

                        match do_sign_mqtt_service_csr(
                            state.cert_signer.as_ref(),
                            &state.settings,
                            &state.db,
                            service,
                            &payload.csr_pem,
                        )
                        .await
                        {
                            Ok(bundle) => {
                                let cert_msg =
                                    MqttControllerMessage::Certificate(CertificatePayload {
                                        cert_pem: bundle.cert_pem,
                                        not_after: bundle.not_after,
                                    });
                                if let Some(json) = serialize_msg(&cert_msg) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                                tracing::info!(%service_id, "MQTT service certificate issued via WS");
                                break; // close connection after certificate issuance
                            }
                            Err(e) => {
                                let err = MqttControllerMessage::Error(ErrorPayload {
                                    code: "certificate_error".to_string(),
                                    message: e,
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                                break;
                            }
                        }
                    }
                    MqttServiceMessage::Enroll(_) => {
                        let err = MqttControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: "already enrolled".to_string(),
                        });
                        if let Some(json) = serialize_msg(&err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                    }
                    MqttServiceMessage::Disconnecting(payload) => {
                        tracing::info!(
                            %service_id,
                            reason = ?payload.reason,
                            "MQTT service disconnecting gracefully during enrollment"
                        );
                        break;
                    }
                    _ => {
                        let err = MqttControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: "not available during enrollment".to_string(),
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
}

/// Anonymous path: expects Enroll message, then promotes in-place.
async fn handle_anonymous(
    socket: WebSocket,
    state: Arc<AppState>,
    _client_ip: Option<std::net::IpAddr>,
) {
    tracing::debug!("anonymous MQTT service connected");

    let (mut sink, mut stream) = socket.split();

    // Wait for first message — must be Enroll
    let (service_id, initial_status) = loop {
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
                let service_msg: MqttServiceMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let err = MqttControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: format!("invalid message: {e}"),
                        });
                        if let Some(json) = serialize_msg(&err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        return;
                    }
                };

                match service_msg {
                    MqttServiceMessage::Enroll(payload) => {
                        let result = do_mqtt_service_enroll(
                            &state.db,
                            &state.settings,
                            state.default_tenant_id,
                            &payload.hostname,
                            &payload.friendly_name,
                            payload.enrollment_token.as_deref(),
                        )
                        .await;

                        match result {
                            Ok(enroll_result) => {
                                let service_id = enroll_result.service.id;
                                let enrolled_msg =
                                    MqttControllerMessage::Enrolled(MqttEnrolledPayload {
                                        service_id: service_id.to_string(),
                                        enrollment_secret: enroll_result.enrollment_secret,
                                        status: format!("{:?}", enroll_result.status)
                                            .to_lowercase(),
                                    });
                                let Some(json) = serialize_msg(&enrolled_msg) else {
                                    return;
                                };
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    return;
                                }

                                tracing::info!(
                                    %service_id,
                                    status = ?enroll_result.status,
                                    "MQTT service enrolled via WS"
                                );

                                // If auto-approved (valid enrollment token), push Approved
                                if enroll_result.status == mqtt_service::MqttServiceStatus::Approved
                                {
                                    let approved_msg =
                                        MqttControllerMessage::Approved(MqttApprovedPayload {
                                            service_id: service_id.to_string(),
                                        });
                                    let Some(json) = serialize_msg(&approved_msg) else {
                                        return;
                                    };
                                    if sink.send(Message::Text(json.into())).await.is_err() {
                                        return;
                                    }
                                }

                                break (service_id, enroll_result.status);
                            }
                            Err(e) => {
                                let err = MqttControllerMessage::Error(ErrorPayload {
                                    code: "enrollment_failed".to_string(),
                                    message: e,
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                                return;
                            }
                        }
                    }
                    _ => {
                        let err = MqttControllerMessage::Error(ErrorPayload {
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

    // Now in enrolled state - continue with enrolled loop
    let approved = initial_status == mqtt_service::MqttServiceStatus::Approved;
    run_enrolled_loop(&mut sink, &mut stream, &state, service_id, approved).await;
    tracing::debug!(%service_id, "anonymous->enrolled MQTT service disconnected");
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

// --- Helper functions ---

struct MqttServiceEnrollResult {
    service: mqtt_service::Model,
    enrollment_secret: String,
    status: mqtt_service::MqttServiceStatus,
}

/// Enroll a new MQTT service.
async fn do_mqtt_service_enroll(
    db: &sea_orm::DatabaseConnection,
    _settings: &crate::settings::Settings,
    tenant_id: uuid::Uuid,
    hostname: &str,
    friendly_name: &str,
    enrollment_token: Option<&str>,
) -> Result<MqttServiceEnrollResult, String> {
    use uptrakit_shared_db::entity::{mqtt_enrollment_token, prelude::MqttEnrollmentToken};

    if hostname.trim().is_empty() {
        return Err("hostname must not be empty".to_string());
    }

    // Generate service_id server-side (single source of truth)
    let service_id = uuid::Uuid::now_v7();

    // Determine status based on enrollment token
    let status = if let Some(token) = enrollment_token {
        // Look up valid enrollment tokens for this tenant
        let tokens = MqttEnrollmentToken::find()
            .filter(mqtt_enrollment_token::Column::TenantId.eq(tenant_id))
            .all(db)
            .await
            .map_err(|e| format!("database error: {e}"))?;

        let mut matched_token = None;
        for t in tokens {
            // Check expiry
            if let Some(expires) = t.expires_at
                && expires < time::OffsetDateTime::now_utc()
            {
                continue;
            }
            // Check uses remaining
            if let Some(remaining) = t.uses_remaining
                && remaining <= 0
            {
                continue;
            }
            // Verify token
            if let Ok(true) = crate::auth::password::verify_password(token, &t.token_hash) {
                matched_token = Some(t);
                break;
            }
        }

        if let Some(t) = matched_token {
            // Decrement uses_remaining if set
            if t.uses_remaining.is_some() {
                let mut active: mqtt_enrollment_token::ActiveModel = t.into();
                active.uses_remaining = Set(active.uses_remaining.unwrap().map(|r| r - 1));
                let _ = active.update(db).await;
            }
            mqtt_service::MqttServiceStatus::Approved
        } else {
            return Err("invalid enrollment token".to_string());
        }
    } else {
        mqtt_service::MqttServiceStatus::Pending
    };

    // Generate enrollment secret
    let enrollment_secret = crate::auth::token::generate_secure_token()
        .map_err(|e| format!("failed to generate token: {e}"))?;
    let secret_hash = crate::auth::password::hash_password(&enrollment_secret)
        .map_err(|e| format!("failed to hash token: {e}"))?;

    let now = time::OffsetDateTime::now_utc();

    // Create service record
    let service = mqtt_service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(friendly_name.to_string()),
        status: Set(status),
        enrollment_secret_hash: Set(secret_hash),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let service = service
        .insert(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    Ok(MqttServiceEnrollResult {
        service,
        enrollment_secret,
        status,
    })
}

/// Sign a CSR for an MQTT service.
async fn do_sign_mqtt_service_csr(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    service: mqtt_service::Model,
    csr_pem: &str,
) -> Result<crate::cert_signer::SignedCertBundle, String> {
    let validity_days = settings.agent_cert_lifetime_days().await;
    let validity = time::Duration::days(validity_days as i64);

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &service.id, validity)
        .map_err(|e| format!("failed to sign CSR: {e}"))?;

    // Record certificate in database (parse cert to get serial, not_before, not_after)
    record_mqtt_service_certificate(db, service.id, &bundle.cert_pem, &ca_fp)
        .await
        .map_err(|e| format!("failed to record certificate: {e}"))?;

    // Update service last_seen_at
    let mut active: mqtt_service::ActiveModel = service.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.updated_at = Set(time::OffsetDateTime::now_utc());
    let _ = active.update(db).await;

    Ok(bundle)
}

/// Record a certificate in the mqtt_service_certificates table.
async fn record_mqtt_service_certificate(
    db: &sea_orm::DatabaseConnection,
    mqtt_service_id: uuid::Uuid,
    cert_pem: &str,
    ca_fingerprint: &str,
) -> Result<(), String> {
    // Parse certificate to extract metadata
    let (_, pem_block) =
        x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).map_err(|_| "failed to parse PEM")?;
    let cert = pem_block.parse_x509().map_err(|_| "failed to parse X509")?;

    let serial = cert.raw_serial_as_string();
    let validity = cert.validity();
    let not_before = time::OffsetDateTime::from_unix_timestamp(validity.not_before.timestamp())
        .map_err(|e| format!("invalid not_before timestamp: {e}"))?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(validity.not_after.timestamp())
        .map_err(|e| format!("invalid not_after timestamp: {e}"))?;

    let record = mqtt_service_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fingerprint.to_string()),
        serial_number: Set(serial),
        mqtt_service_id: Set(mqtt_service_id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(time::OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };

    record
        .insert(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    Ok(())
}

/// Revoke an MQTT service certificate.
async fn revoke_mqtt_service_certificate(
    db: &sea_orm::DatabaseConnection,
    serial: &str,
    ca_fingerprint: &str,
    reason: mqtt_service_certificate::MqttServiceCertificateRevocationReason,
) -> Result<(), String> {
    let cert = mqtt_service_certificate::Entity::find()
        .filter(mqtt_service_certificate::Column::SerialNumber.eq(serial))
        .filter(mqtt_service_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .one(db)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| "certificate not found".to_string())?;

    let mut active: mqtt_service_certificate::ActiveModel = cert.into();
    active.revoked_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.revocation_reason = Set(Some(reason));
    active
        .update(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    Ok(())
}
