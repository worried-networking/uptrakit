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
    ApprovedPayload, ControllerMessage, EnrolledPayload, ErrorCode, ErrorPayload, PongPayload,
    RejectedPayload, ServiceMessage, ServiceSettingsPayload, now_millis,
};
use uptrakit_shared_db::entity::service as service_entity;

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

use crate::AppState;
use crate::extract::{ClientIp, ServiceIdentity};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
enum ServiceWsError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("invalid enrollment secret")]
    InvalidSecret,
}

type ServiceWsResult<T> = std::result::Result<T, Report<ServiceWsError>>;

impl_report_conversion!(sea_orm::DbErr => ServiceWsError::Database);

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Serialize a [`ControllerMessage`] to JSON, logging on failure.
pub(crate) fn serialize_msg(msg: &ControllerMessage) -> Option<String> {
    match serde_json::to_string(msg) {
        Ok(json) => Some(json),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize controller message");
            None
        }
    }
}

pub(crate) async fn close_with_reason(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    reason: &str,
) -> Result<(), axum::Error> {
    sink.send(Message::Close(Some(CloseFrame {
        code: axum::extract::ws::close_code::POLICY,
        reason: reason.into(),
    })))
    .await
}

/// Send a Pong response for a received Ping.
///
/// Returns the controller timestamp on success so callers can use it
/// for trace logging.
pub(crate) async fn send_pong(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    service_ts: i64,
) -> Result<i64, ()> {
    let controller_ts = now_millis();
    let response = ControllerMessage::Pong(PongPayload {
        service_ts,
        controller_ts,
    });
    let Some(json) = serialize_msg(&response) else {
        return Err(());
    };
    sink.send(Message::Text(json.into()))
        .await
        .map(|()| controller_ts)
        .map_err(|_| ())
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Connection type (shared across both service types)
// ---------------------------------------------------------------------------

/// Connection type determined at WebSocket upgrade time.
enum ConnectionType {
    /// mTLS client cert present -- authenticated service.
    Authenticated {
        service_id: uuid::Uuid,
        cert_serial: String,
    },
    /// Authorization: Bearer <secret> -- reconnecting enrolled service.
    Enrolled(uuid::Uuid),
    /// No auth -- expects Enroll message.
    Anonymous,
}

// ---------------------------------------------------------------------------
// Unified entry point
// ---------------------------------------------------------------------------

/// Unified WebSocket handler for both agent and MQTT services.
///
/// Determines the connection type (Authenticated / Enrolled / Anonymous) and,
/// once the `service_type` is known, dispatches to the appropriate
/// service-specific handler in [`super::agent_ws`] or [`super::mqtt_ws`].
pub async fn service_ws(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<ServiceIdentity>>,
    client_ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let conn_type = if let Some(Extension(ref id)) = identity {
        tracing::info!(service_id = %id.service_id, "service WS upgrade (mTLS)");
        ConnectionType::Authenticated {
            service_id: id.service_id,
            cert_serial: id.cert_serial.clone(),
        }
    } else if let Some(secret) = extract_bearer(&headers) {
        // Try unified lookup: find any non-deactivated service by secret hash.
        match lookup_by_secret(&state.db, &secret).await {
            Ok(service) => {
                tracing::info!(
                    service_id = %service.id,
                    service_type = ?service.service_type,
                    "enrolled service WS upgrade (bearer)"
                );
                ConnectionType::Enrolled(service.id)
            }
            Err(e) => {
                let msg = e.to_string();
                tracing::warn!("bearer auth failed: {msg}");
                return (axum::http::StatusCode::UNAUTHORIZED, msg).into_response();
            }
        }
    } else {
        tracing::info!("anonymous service WS upgrade");
        ConnectionType::Anonymous
    };

    let ip = client_ip.map(|Extension(ClientIp(ip))| ip);

    ws.on_upgrade(move |socket| handle_connection(socket, state, conn_type, ip))
}

// ---------------------------------------------------------------------------
// Unified bearer-secret lookup (searches both Agent and Mqtt services)
// ---------------------------------------------------------------------------

/// Look up a service by bearer enrollment secret.
///
/// For agent services the secret hash is computed deterministically (SHA-256);
/// for MQTT services the hash is verified via argon2 (variable-time).
/// We first try the fast agent-style lookup, then fall back to the MQTT-style
/// brute-force verification.
async fn lookup_by_secret(
    db: &sea_orm::DatabaseConnection,
    secret: &str,
) -> ServiceWsResult<service_entity::Model> {
    // Agent-style: deterministic hash lookup.
    let secret_hash = crate::auth::token::hash_token(secret);
    if let Ok(Some(service)) = uptrakit_shared_db::entity::prelude::Service::find()
        .filter(service_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(service_entity::Column::DeactivatedAt.is_null())
        .one(db)
        .await
    {
        return Ok(service);
    }

    // MQTT-style: iterate non-deactivated MQTT services and verify via argon2.
    let mqtt_services = uptrakit_shared_db::entity::prelude::Service::find()
        .filter(service_entity::Column::ServiceType.eq(service_entity::ServiceType::Mqtt))
        .filter(service_entity::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to::<ServiceWsError>()?;

    for svc in mqtt_services {
        if let Ok(true) =
            crate::auth::password::verify_password(secret, &svc.enrollment_secret_hash)
        {
            return Ok(svc);
        }
    }

    Err(report!(ServiceWsError::InvalidSecret))
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Authenticated path
// ---------------------------------------------------------------------------

/// Authenticated path: mTLS-based connection. Validates certificate and service
/// status, sends ServiceSettings, then dispatches to the service-type-specific
/// authenticated loop.
async fn handle_authenticated(
    socket: WebSocket,
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    cert_serial: String,
) {
    tracing::debug!(%service_id, "authenticated service connected");

    let (mut sink, mut stream) = socket.split();

    // 1. Certificate validation check
    let cert_record = if cert_serial.is_empty() {
        match uptrakit_shared_db::entity::prelude::ServiceCertificate::find()
            .filter(
                uptrakit_shared_db::entity::service_certificate::Column::ServiceId.eq(service_id),
            )
            .filter(uptrakit_shared_db::entity::service_certificate::Column::RevokedAt.is_null())
            .order_by_desc(uptrakit_shared_db::entity::service_certificate::Column::CreatedAt)
            .one(&state.db)
            .await
        {
            Ok(Some(record)) => {
                tracing::warn!(
                    %service_id,
                    "service connected via proxy without cert serial, using service-id-only lookup"
                );
                record
            }
            Ok(None) => {
                tracing::warn!(
                    %service_id,
                    "rejected connection: no non-revoked certificate found for service"
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
        match uptrakit_shared_db::entity::prelude::ServiceCertificate::find()
            .filter(
                uptrakit_shared_db::entity::service_certificate::Column::SerialNumber
                    .eq(cert_serial.clone()),
            )
            .filter(
                uptrakit_shared_db::entity::service_certificate::Column::ServiceId.eq(service_id),
            )
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

    // 2. Service status check -- also determines service_type.
    let service = match uptrakit_shared_db::entity::prelude::Service::find_by_id(service_id)
        .one(&state.db)
        .await
    {
        Ok(Some(svc)) => {
            if svc.deactivated_at.is_some() {
                tracing::warn!(%service_id, "deactivated service connected with valid certificate");
                let _ = close_with_reason(&mut sink, "service deactivated").await;
                return;
            }
            if svc.status != service_entity::ServiceStatus::Approved {
                tracing::warn!(%service_id, "rejected connection: service not approved");
                let _ = close_with_reason(&mut sink, "service not approved").await;
                return;
            }
            svc
        }
        Ok(None) => {
            tracing::warn!(%service_id, "rejected connection: service not found");
            let _ = close_with_reason(&mut sink, "service not found").await;
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "service status check failed");
            let _ = close_with_reason(&mut sink, "internal error").await;
            return;
        }
    };

    // Save CA fingerprint before moving cert_record.
    let cert_ca_fingerprint = cert_record.ca_fingerprint.clone();

    // Record certificate usage.
    let mut active: uptrakit_shared_db::entity::service_certificate::ActiveModel =
        cert_record.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    if let Err(e) = active.update(&state.db).await {
        tracing::error!(error = %e, "failed to update certificate last_seen_at");
    }

    // Send ServiceSettings on connect.
    let renewal_window_hours = state.settings.renewal_window_hours().await;
    let ca_bundle_hash = state.ca_snapshot.borrow().bundle_hash.clone();
    let shutdown_timeout = match service.service_type {
        service_entity::ServiceType::Agent => Some(120),
        service_entity::ServiceType::Mqtt => None,
    };
    let settings_msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours,
        ca_bundle_hash,
        shutdown_timeout_seconds: shutdown_timeout,
    });
    let Some(json) = serialize_msg(&settings_msg) else {
        return;
    };
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    // Dispatch to service-type-specific authenticated handler.
    match service.service_type {
        service_entity::ServiceType::Agent => {
            super::agent_ws::handle_agent_authenticated(
                &mut sink,
                &mut stream,
                &state,
                service_id,
                cert_serial,
                cert_ca_fingerprint,
            )
            .await;
        }
        service_entity::ServiceType::Mqtt => {
            super::mqtt_ws::handle_mqtt_authenticated(
                &mut sink,
                &mut stream,
                &state,
                service_id,
                cert_serial,
                cert_ca_fingerprint,
            )
            .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Enrolled path
// ---------------------------------------------------------------------------

/// Enrolled path: service reconnecting with Bearer secret, waiting for approval.
async fn handle_enrolled(socket: WebSocket, state: Arc<AppState>, service_id: uuid::Uuid) {
    tracing::debug!(%service_id, "enrolled service connected (bearer)");

    // Look up service to determine type and current status.
    let service = match uptrakit_shared_db::entity::prelude::Service::find_by_id(service_id)
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!(%service_id, "service not found in DB");
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "DB lookup failed");
            return;
        }
    };

    let (mut sink, mut stream) = socket.split();

    // If already approved/rejected, push immediately.
    match service.status {
        service_entity::ServiceStatus::Approved => {
            let msg = ControllerMessage::Approved(ApprovedPayload { service_id });
            let Some(json) = serialize_msg(&msg) else {
                return;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
        service_entity::ServiceStatus::Rejected => {
            let msg = ControllerMessage::Rejected(RejectedPayload { service_id });
            if let Some(json) = serialize_msg(&msg) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            return;
        }
        _ => {
            // Pending -- wait for push.
        }
    }

    // Dispatch to service-type-specific enrolled loop.
    match service.service_type {
        service_entity::ServiceType::Agent => {
            super::agent_ws::handle_agent_enrolled(&mut sink, &mut stream, &state, service_id)
                .await;
        }
        service_entity::ServiceType::Mqtt => {
            super::mqtt_ws::handle_mqtt_enrolled(
                &mut sink,
                &mut stream,
                &state,
                service_id,
                service.status == service_entity::ServiceStatus::Approved,
            )
            .await;
        }
    }

    tracing::debug!(%service_id, "enrolled service disconnected");
}

// ---------------------------------------------------------------------------
// Anonymous path
// ---------------------------------------------------------------------------

/// Anonymous path: expects an Enroll message, then promotes in-place.
async fn handle_anonymous(
    socket: WebSocket,
    state: Arc<AppState>,
    client_ip: Option<std::net::IpAddr>,
) {
    tracing::debug!("anonymous service connected");

    let (mut sink, mut stream) = socket.split();

    // Wait for first message -- must be Enroll.
    let (service_id, service_type, initial_approved) = loop {
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
                let service_msg: ServiceMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: ErrorCode::BadRequest,
                            message: format!("invalid message: {e}"),
                        });
                        if let Some(json) = serialize_msg(&err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        return;
                    }
                };

                match service_msg {
                    ServiceMessage::Enroll(payload) => {
                        match payload.service_type {
                            uptrakit_internal_wire::ServiceType::Agent => {
                                match enroll_agent(&state, &payload, client_ip, &mut sink).await {
                                    Some((id, approved)) => {
                                        break (id, service_entity::ServiceType::Agent, approved);
                                    }
                                    None => return, // enrollment failed, error already sent
                                }
                            }
                            uptrakit_internal_wire::ServiceType::Mqtt => {
                                match enroll_mqtt(&state, &payload, &mut sink).await {
                                    Some((id, approved)) => {
                                        break (id, service_entity::ServiceType::Mqtt, approved);
                                    }
                                    None => return,
                                }
                            }
                        }
                    }
                    _ => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: ErrorCode::BadRequest,
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

    // Dispatch to service-type-specific enrolled loop.
    match service_type {
        service_entity::ServiceType::Agent => {
            // Register in connection registry.
            let mut push_rx = state.service_connections.register_agent(service_id).await;
            super::agent_ws::run_agent_enrolled_loop(
                &mut sink,
                &mut stream,
                &mut push_rx,
                &state,
                service_id,
            )
            .await;
            state.service_connections.unregister(&service_id).await;
        }
        service_entity::ServiceType::Mqtt => {
            super::mqtt_ws::handle_mqtt_enrolled(
                &mut sink,
                &mut stream,
                &state,
                service_id,
                initial_approved,
            )
            .await;
        }
    }

    tracing::debug!(%service_id, "anonymous->enrolled service disconnected");
}

// ---------------------------------------------------------------------------
// Agent enrollment
// ---------------------------------------------------------------------------

/// Perform agent enrollment. Returns `(service_id, approved)` on success, or
/// `None` if enrollment failed (error already sent to client).
async fn enroll_agent(
    state: &Arc<AppState>,
    payload: &uptrakit_internal_wire::EnrollPayload,
    client_ip: Option<std::net::IpAddr>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Option<(uuid::Uuid, bool)> {
    use crate::routes::agents::{AgentStatus, EnrollParams, do_enroll};

    let result = do_enroll(EnrollParams {
        db: &state.db,
        settings: &state.settings,
        tenant_id: state.default_tenant_id,
        hostname: &payload.hostname,
        friendly_name: &payload.friendly_name,
        enrollment_token: payload.enrollment_token.as_deref(),
        ip_address: client_ip,
        host_info: payload.host_info.as_ref(),
    })
    .await;

    match result {
        Ok(enroll_result) => {
            let service_id = enroll_result.agent.id;
            let wire_status = match enroll_result.status {
                AgentStatus::Approved => uptrakit_internal_wire::EnrollmentStatus::Approved,
                _ => uptrakit_internal_wire::EnrollmentStatus::Pending,
            };
            let enrolled_msg = ControllerMessage::Enrolled(EnrolledPayload {
                service_id,
                enrollment_secret: enroll_result.enrollment_secret,
                status: wire_status,
            });
            let json = serialize_msg(&enrolled_msg)?;
            if sink.send(Message::Text(json.into())).await.is_err() {
                return None;
            }

            tracing::info!(
                %service_id,
                ?wire_status,
                "agent enrolled via WS"
            );

            let approved = enroll_result.status == AgentStatus::Approved;
            if approved {
                let approved_msg = ControllerMessage::Approved(ApprovedPayload { service_id });
                let json = serialize_msg(&approved_msg)?;
                if sink.send(Message::Text(json.into())).await.is_err() {
                    return None;
                }
            }

            Some((service_id, approved))
        }
        Err(e) => {
            let err = ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::EnrollmentFailed,
                message: e.current_context().to_string(),
            });
            if let Some(json) = serialize_msg(&err) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// MQTT enrollment
// ---------------------------------------------------------------------------

/// Perform MQTT service enrollment. Returns `(service_id, approved)` on
/// success, or `None` if enrollment failed (error already sent to client).
async fn enroll_mqtt(
    state: &Arc<AppState>,
    payload: &uptrakit_internal_wire::EnrollPayload,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Option<(uuid::Uuid, bool)> {
    let result = super::mqtt_ws::do_mqtt_service_enroll(
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
            let wire_status = match enroll_result.status {
                service_entity::ServiceStatus::Approved => {
                    uptrakit_internal_wire::EnrollmentStatus::Approved
                }
                _ => uptrakit_internal_wire::EnrollmentStatus::Pending,
            };
            let enrolled_msg = ControllerMessage::Enrolled(EnrolledPayload {
                service_id,
                enrollment_secret: enroll_result.enrollment_secret,
                status: wire_status,
            });
            let json = serialize_msg(&enrolled_msg)?;
            if sink.send(Message::Text(json.into())).await.is_err() {
                return None;
            }

            tracing::info!(
                %service_id,
                ?wire_status,
                "MQTT service enrolled via WS"
            );

            let approved = enroll_result.status == service_entity::ServiceStatus::Approved;
            if approved {
                let approved_msg = ControllerMessage::Approved(ApprovedPayload { service_id });
                let json = serialize_msg(&approved_msg)?;
                if sink.send(Message::Text(json.into())).await.is_err() {
                    return None;
                }
            }

            Some((service_id, approved))
        }
        Err(e) => {
            let err = ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::EnrollmentFailed,
                message: e.to_string(),
            });
            if let Some(json) = serialize_msg(&err) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            None
        }
    }
}
