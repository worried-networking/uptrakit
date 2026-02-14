use std::net::IpAddr;
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
    ApprovedPayload, ControllerMessage, EnrolledPayload, ErrorCode, ErrorPayload, IncomingSeq,
    OutgoingSeq, PongPayload, RejectedPayload, ServiceEnvelope, ServiceMessage,
    ServiceSettingsPayload, now_millis,
};
use uptrakit_shared_db::entity::service as service_entity;

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

use crate::AppState;
use crate::extract::{ClientIp, ServiceIdentity};

/// Maximum number of incoming WebSocket messages per connection per second.
pub(crate) const WS_MESSAGE_RATE_LIMIT: u32 = 50;
/// Window for WebSocket message rate limiting.
pub(crate) const WS_MESSAGE_RATE_WINDOW: std::time::Duration = std::time::Duration::from_secs(1);

/// Fixed-window rate limiter for WebSocket message processing.
pub(crate) struct MessageRateLimiter {
    window_start: std::time::Instant,
    window: std::time::Duration,
    max_per_window: u32,
    count: u32,
}

impl MessageRateLimiter {
    pub(crate) fn new(window: std::time::Duration, max_per_window: u32) -> Self {
        Self {
            window_start: std::time::Instant::now(),
            window,
            max_per_window,
            count: 0,
        }
    }

    pub(crate) fn allow(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.window_start) >= self.window {
            self.window_start = now;
            self.count = 0;
        }

        if self.count < self.max_per_window {
            self.count = self.count.saturating_add(1);
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    fn set_window_start(&mut self, start: std::time::Instant) {
        self.window_start = start;
    }
}

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

#[derive(Debug, Error)]
pub(crate) enum ServiceActivityError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("service not found: {0}")]
    ServiceNotFound(uuid::Uuid),
}

pub(crate) type ServiceActivityResult<T> = std::result::Result<T, Report<ServiceActivityError>>;

impl_report_conversion!(sea_orm::DbErr => ServiceActivityError::Database);

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Serialize a [`ControllerMessage`] into a sequenced [`ControllerEnvelope`]
/// JSON string, logging on failure.
pub(crate) fn serialize_controller_msg(
    out_seq: &mut OutgoingSeq,
    msg: ControllerMessage,
) -> Option<String> {
    let envelope = out_seq.wrap_controller(msg);
    match serde_json::to_string(&envelope) {
        Ok(json) => Some(json),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize controller message");
            None
        }
    }
}

/// Deserialize a [`ServiceMessage`] from a sequenced [`ServiceEnvelope`]
/// JSON string, validating the sequence number.
pub(crate) fn deserialize_service_msg(
    in_seq: &mut IncomingSeq,
    text: &str,
) -> Result<ServiceMessage, String> {
    let envelope: ServiceEnvelope =
        serde_json::from_str(text).map_err(|e| format!("invalid message: {e}"))?;
    in_seq.validate(envelope.seq).map_err(|e| e.to_string())?;
    Ok(envelope.message)
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
    out_seq: &mut OutgoingSeq,
    service_ts: i64,
) -> Result<i64, ()> {
    let controller_ts = now_millis();
    let response = ControllerMessage::Pong(PongPayload {
        service_ts,
        controller_ts,
    });
    let Some(json) = serialize_controller_msg(out_seq, response) else {
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

pub(crate) async fn record_service_activity(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    ip_address: Option<IpAddr>,
) -> ServiceActivityResult<()> {
    let service = service_entity::Entity::find_by_id(service_id)
        .one(db)
        .await
        .context_to::<ServiceActivityError>()?
        .ok_or_else(|| report!(ServiceActivityError::ServiceNotFound(service_id)))?;

    let now = time::OffsetDateTime::now_utc();
    let mut active: service_entity::ActiveModel = service.into();
    active.last_seen_at = Set(Some(now));
    active.updated_at = Set(now);
    if let Some(ip) = ip_address {
        active.ip_address = Set(Some(ip.to_string()));
    }
    active
        .update(db)
        .await
        .context_to::<ServiceActivityError>()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Connection type (shared across both service types)
// ---------------------------------------------------------------------------

/// Certificate identity information extracted from the mTLS handshake.
///
/// Bundled into a struct to keep function signatures under the argument limit.
pub(crate) struct CertIdentity {
    pub serial: String,
    pub ca_fingerprint: String,
}

/// Shared context for authenticated service handlers.
pub(crate) struct AuthenticatedContext<'a> {
    pub service_id: uuid::Uuid,
    pub cert: CertIdentity,
    pub last_seen_at: Option<time::OffsetDateTime>,
    pub out_seq: &'a mut OutgoingSeq,
    pub in_seq: &'a mut IncomingSeq,
}

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
///
/// Per-IP rate limiting is applied before the WebSocket upgrade to prevent
/// connection floods and brute-force bearer secret guessing.
pub async fn service_ws(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<ServiceIdentity>>,
    client_ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    use crate::auth::rate_limit::RateLimitOutcome;

    // Per-IP connection rate limit: 30 attempts per 60 seconds.
    // Fail-closed on DB error to prevent bypass under load.
    if let Some(Extension(ClientIp(ref ip))) = client_ip {
        let key = format!("ws_connect:{ip}");
        match state.rate_limit_store.check_rate_limit(&key, 30, 60).await {
            Ok(RateLimitOutcome::Limited { retry_after_secs }) => {
                tracing::warn!(%ip, "WS connection rate limited");
                return (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    format!("Too many connection attempts. Retry after {retry_after_secs}s"),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::error!(error = %e, "WS rate limiter error — rejecting (fail-closed)");
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Service temporarily unavailable".to_string(),
                )
                    .into_response();
            }
            Ok(RateLimitOutcome::Allowed) => {}
        }
    }

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
                // Per-IP auth failure rate limit: 10 failures per 300 seconds.
                if let Some(Extension(ClientIp(ref ip))) = client_ip {
                    let fail_key = format!("ws_auth_fail:{ip}");
                    match state
                        .rate_limit_store
                        .check_rate_limit(&fail_key, 10, 300)
                        .await
                    {
                        Ok(RateLimitOutcome::Limited { retry_after_secs }) => {
                            tracing::warn!(%ip, "WS bearer auth failure rate limited");
                            return (
                                axum::http::StatusCode::TOO_MANY_REQUESTS,
                                format!(
                                    "Too many failed auth attempts. Retry after {retry_after_secs}s"
                                ),
                            )
                                .into_response();
                        }
                        Err(rate_err) => {
                            tracing::error!(
                                error = %rate_err,
                                "WS auth-fail rate limiter error — rejecting (fail-closed)"
                            );
                            return (
                                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                "Service temporarily unavailable".to_string(),
                            )
                                .into_response();
                        }
                        Ok(RateLimitOutcome::Allowed) => {}
                    }
                }
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

    ws.max_message_size(MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_connection(socket, state, conn_type, ip))
}

// ---------------------------------------------------------------------------
// Unified bearer-secret lookup (searches both Agent and Mqtt services)
// ---------------------------------------------------------------------------

/// Look up a service by bearer enrollment secret.
///
/// All services (agent and MQTT) use deterministic SHA-256 hashing for enrollment
/// secrets. This allows a single indexed DB query instead of iterating rows.
async fn lookup_by_secret(
    db: &sea_orm::DatabaseConnection,
    secret: &str,
) -> ServiceWsResult<service_entity::Model> {
    let secret_hash = crate::auth::token::hash_token(secret);
    uptrakit_shared_db::entity::prelude::Service::find()
        .filter(service_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(service_entity::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to::<ServiceWsError>()?
        .ok_or_else(|| report!(ServiceWsError::InvalidSecret))
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

async fn handle_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    conn_type: ConnectionType,
    client_ip: Option<IpAddr>,
) {
    let mut out_seq = OutgoingSeq::new();
    let mut in_seq = IncomingSeq::new();

    match conn_type {
        ConnectionType::Authenticated {
            service_id,
            cert_serial,
        } => {
            handle_authenticated(
                socket,
                state,
                service_id,
                cert_serial,
                client_ip,
                &mut out_seq,
                &mut in_seq,
            )
            .await;
        }
        ConnectionType::Enrolled(service_id) => {
            handle_enrolled(
                socket,
                state,
                service_id,
                client_ip,
                &mut out_seq,
                &mut in_seq,
            )
            .await;
        }
        ConnectionType::Anonymous => {
            handle_anonymous(socket, state, client_ip, &mut out_seq, &mut in_seq).await;
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
    client_ip: Option<IpAddr>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
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

    // Bundle certificate identity before moving cert_record.
    let cert_id = CertIdentity {
        serial: cert_serial,
        ca_fingerprint: cert_record.ca_fingerprint.clone(),
    };

    let previous_last_seen_at = service.last_seen_at;
    let now = time::OffsetDateTime::now_utc();

    if let Err(e) = record_service_activity(&state.db, service_id, client_ip).await {
        tracing::error!(error = %e, %service_id, "failed to update service activity");
    }

    // Record certificate usage.
    let mut active: uptrakit_shared_db::entity::service_certificate::ActiveModel =
        cert_record.into();
    active.last_seen_at = Set(Some(now));
    if let Err(e) = active.update(&state.db).await {
        tracing::error!(error = %e, "failed to update certificate last_seen_at");
    }

    // Send ServiceSettings on connect.
    let renewal_window_hours = state.settings.renewal_window_hours();
    let ca_bundle_hash = state.ca_snapshot.borrow().bundle_hash.clone();
    let shutdown_timeout = match service.service_type {
        service_entity::ServiceType::Agent => Some(120),
        service_entity::ServiceType::Mqtt => None,
    };
    let settings_msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours,
        ca_bundle_hash,
        protocol_version: uptrakit_internal_wire::PROTOCOL_VERSION,
        shutdown_timeout_seconds: shutdown_timeout,
    });
    let Some(json) = serialize_controller_msg(out_seq, settings_msg) else {
        return;
    };
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    // Dispatch to service-type-specific authenticated handler.
    match service.service_type {
        service_entity::ServiceType::Agent => {
            let ctx = AuthenticatedContext {
                service_id,
                cert: cert_id,
                last_seen_at: previous_last_seen_at,
                out_seq,
                in_seq,
            };
            super::agent_ws::handle_agent_authenticated(&mut sink, &mut stream, &state, ctx).await;
        }
        service_entity::ServiceType::Mqtt => {
            let ctx = AuthenticatedContext {
                service_id,
                cert: cert_id,
                last_seen_at: previous_last_seen_at,
                out_seq,
                in_seq,
            };
            super::mqtt_ws::handle_mqtt_authenticated(&mut sink, &mut stream, &state, ctx).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Enrolled path
// ---------------------------------------------------------------------------

/// Enrolled path: service reconnecting with Bearer secret, waiting for approval.
async fn handle_enrolled(
    socket: WebSocket,
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    client_ip: Option<IpAddr>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
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

    if let Err(e) = record_service_activity(&state.db, service_id, client_ip).await {
        tracing::error!(error = %e, %service_id, "failed to update service activity");
    }

    // If already approved/rejected, push immediately.
    match service.status {
        service_entity::ServiceStatus::Approved => {
            let msg = ControllerMessage::Approved(ApprovedPayload { service_id });
            let Some(json) = serialize_controller_msg(out_seq, msg) else {
                return;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
        service_entity::ServiceStatus::Rejected => {
            let msg = ControllerMessage::Rejected(RejectedPayload { service_id });
            if let Some(json) = serialize_controller_msg(out_seq, msg) {
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
            super::agent_ws::handle_agent_enrolled(
                &mut sink,
                &mut stream,
                &state,
                service_id,
                out_seq,
                in_seq,
            )
            .await;
        }
        service_entity::ServiceType::Mqtt => {
            super::mqtt_ws::handle_mqtt_enrolled(
                &mut sink,
                &mut stream,
                &state,
                service_id,
                service.status == service_entity::ServiceStatus::Approved,
                out_seq,
                in_seq,
            )
            .await;
        }
    }

    tracing::debug!(%service_id, "enrolled service disconnected");
}

// ---------------------------------------------------------------------------
// Anonymous path
// ---------------------------------------------------------------------------

/// Maximum incoming WebSocket message size (1 MB).
///
/// The largest legitimate message is `ExecuteUpdate` with provider config and
/// release assets, typically well under 100 KB. 1 MB provides ample headroom
/// while preventing memory-exhaustion DoS from oversized payloads.
const MAX_WS_MESSAGE_SIZE: usize = 1_048_576;

/// Maximum time an anonymous WebSocket connection may remain idle before
/// sending the initial `Enroll` message.
const ANONYMOUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Anonymous path: expects an Enroll message, then promotes in-place.
async fn handle_anonymous(
    socket: WebSocket,
    state: Arc<AppState>,
    client_ip: Option<IpAddr>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    tracing::debug!("anonymous service connected");

    let (mut sink, mut stream) = socket.split();

    let deadline = tokio::time::Instant::now() + ANONYMOUS_TIMEOUT;

    // Wait for first message -- must be Enroll.
    let (service_id, service_type, initial_approved) = loop {
        let msg = match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => {
                tracing::debug!(error = %e, "websocket receive error");
                return;
            }
            Ok(None) => return,
            Err(_) => {
                tracing::warn!("anonymous connection timed out after {ANONYMOUS_TIMEOUT:?}");
                let _ = close_with_reason(&mut sink, "enrollment timeout").await;
                return;
            }
        };

        match msg {
            Message::Text(text) => {
                let service_msg = match deserialize_service_msg(in_seq, &text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "invalid message from anonymous client");
                        let code = if e.starts_with("sequence error:") {
                            ErrorCode::SequenceError
                        } else {
                            ErrorCode::BadRequest
                        };
                        let err = ControllerMessage::Error(ErrorPayload { code, message: e });
                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        return;
                    }
                };

                match service_msg {
                    ServiceMessage::Enroll(payload) => {
                        match payload.service_type {
                            uptrakit_internal_wire::ServiceType::Agent => {
                                match enroll_agent(&state, &payload, client_ip, &mut sink, out_seq)
                                    .await
                                {
                                    Some((id, approved)) => {
                                        break (id, service_entity::ServiceType::Agent, approved);
                                    }
                                    None => return, // enrollment failed, error already sent
                                }
                            }
                            uptrakit_internal_wire::ServiceType::Mqtt => {
                                match enroll_mqtt(&state, &payload, client_ip, &mut sink, out_seq)
                                    .await
                                {
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
                        if let Some(json) = serialize_controller_msg(out_seq, err) {
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
            let (mut push_rx, cancel_token) =
                state.service_connections.register_agent(service_id).await;
            super::agent_ws::run_agent_enrolled_loop(
                &mut sink,
                &mut stream,
                (&mut push_rx, &cancel_token),
                &state,
                service_id,
                out_seq,
                in_seq,
            )
            .await;
            if !cancel_token.is_cancelled() {
                state.service_connections.unregister(&service_id).await;
            }
        }
        service_entity::ServiceType::Mqtt => {
            super::mqtt_ws::handle_mqtt_enrolled(
                &mut sink,
                &mut stream,
                &state,
                service_id,
                initial_approved,
                out_seq,
                in_seq,
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
    out_seq: &mut OutgoingSeq,
) -> Option<(uuid::Uuid, bool)> {
    use crate::routes::agents::{AgentStatus, EnrollParams, do_enroll};

    let result = do_enroll(EnrollParams {
        db: &state.db,
        settings: &state.settings,
        tenant_id: state.default_tenant_id,
        hostname: &payload.hostname,
        friendly_name: &payload.friendly_name,
        enrollment_token: payload.enrollment_token.as_ref().map(|s| s.expose_secret()),
        ip_address: client_ip,
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
                enrollment_secret: uptrakit_internal_wire::SecretString::new(
                    enroll_result.enrollment_secret,
                ),
                status: wire_status,
            });
            let json = serialize_controller_msg(out_seq, enrolled_msg)?;
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
                let json = serialize_controller_msg(out_seq, approved_msg)?;
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
            if let Some(json) = serialize_controller_msg(out_seq, err) {
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
    client_ip: Option<IpAddr>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
) -> Option<(uuid::Uuid, bool)> {
    let result = super::mqtt_ws::do_mqtt_service_enroll(
        &state.db,
        &state.settings,
        state.default_tenant_id,
        &payload.hostname,
        &payload.friendly_name,
        payload.enrollment_token.as_ref().map(|s| s.expose_secret()),
        client_ip,
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
                enrollment_secret: uptrakit_internal_wire::SecretString::new(
                    enroll_result.enrollment_secret,
                ),
                status: wire_status,
            });
            let json = serialize_controller_msg(out_seq, enrolled_msg)?;
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
                let json = serialize_controller_msg(out_seq, approved_msg)?;
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
            if let Some(json) = serialize_controller_msg(out_seq, err) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection,
    };

    #[test]
    fn message_rate_limiter_enforces_window() {
        let mut limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, 2);
        assert!(limiter.allow());
        assert!(limiter.allow());
        assert!(!limiter.allow());

        let reset_time = std::time::Instant::now() - WS_MESSAGE_RATE_WINDOW;
        limiter.set_window_start(reset_time);
        assert!(limiter.allow());
    }

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        db.execute_unprepared(
            "CREATE TABLE services (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                service_type TEXT NOT NULL,
                hostname TEXT NOT NULL,
                friendly_name TEXT NOT NULL,
                ip_address TEXT,
                status TEXT NOT NULL,
                enrollment_secret_hash TEXT NOT NULL UNIQUE,
                client_version TEXT,
                last_seen_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deactivated_at INTEGER
            )",
        )
        .await
        .expect("create services");
        db
    }

    async fn insert_service(db: &DatabaseConnection, ip_address: Option<&str>) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        service_entity::ActiveModel {
            id: Set(id),
            tenant_id: Set(uuid::Uuid::now_v7()),
            service_type: Set(service_entity::ServiceType::Agent),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("test-host".to_string()),
            ip_address: Set(ip_address.map(ToOwned::to_owned)),
            status: Set(service_entity::ServiceStatus::Pending),
            enrollment_secret_hash: Set(format!("secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert service");
        id
    }

    #[tokio::test]
    async fn record_service_activity_sets_ip_when_provided() {
        let db = setup_test_db().await;
        let service_id = insert_service(&db, None).await;

        let ip = std::net::IpAddr::from([10, 1, 2, 3]);
        record_service_activity(&db, service_id, Some(ip))
            .await
            .expect("record activity");

        let model = service_entity::Entity::find_by_id(service_id)
            .one(&db)
            .await
            .expect("query service")
            .expect("service exists");
        assert_eq!(model.ip_address.as_deref(), Some("10.1.2.3"));
        assert!(model.last_seen_at.is_some());
    }

    #[tokio::test]
    async fn record_service_activity_preserves_ip_when_absent() {
        let db = setup_test_db().await;
        let service_id = insert_service(&db, Some("192.0.2.7")).await;

        record_service_activity(&db, service_id, None)
            .await
            .expect("record activity");

        let model = service_entity::Entity::find_by_id(service_id)
            .one(&db)
            .await
            .expect("query service")
            .expect("service exists");
        assert_eq!(model.ip_address.as_deref(), Some("192.0.2.7"));
        assert!(model.last_seen_at.is_some());
    }
}
