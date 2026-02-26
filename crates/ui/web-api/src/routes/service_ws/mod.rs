mod protocol;

// Re-export protocol items used by service_handler and other modules.
pub(crate) use protocol::{
    AuthenticatedContext, CertIdentity, MessageRateLimiter, ServiceWsError, ServiceWsResult,
    WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW, close_with_reason, controller_capabilities,
    deserialize_service_msg, record_service_activity, send_pong, serialize_controller_msg,
};
use protocol::ConnectionType;

use std::net::IpAddr;
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use rootcause::prelude::*;
use uptrakit_internal_wire::{
    ApprovedPayload, CloseReason, ControllerMessage, EnrolledPayload, ErrorCode, ErrorPayload,
    IncomingSeq, OutgoingSeq, RejectedPayload, ServiceMessage, ServiceSettingsPayload,
};
use uptrakit_shared_db::entity::service as service_entity;

use crate::AppState;
use crate::extract::{ClientIp, ServiceIdentity};

// ---------------------------------------------------------------------------
// Unified entry point
// ---------------------------------------------------------------------------

/// Unified WebSocket handler for all service types.
///
/// Determines the connection type (Authenticated / Enrolled / Anonymous) and
/// dispatches to the unified capability-gated handler in
/// [`super::service_handler`].
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
        match lookup_by_secret(state.db(), &secret).await {
            Ok(service) => {
                tracing::info!(
                    service_id = %service.id,
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
// Unified bearer-secret lookup
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

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
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
            .one(state.db())
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
                let _ = close_with_reason(&mut sink, CloseReason::NoValidCertificate).await;
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "certificate validation check failed");
                let _ = close_with_reason(&mut sink, CloseReason::InternalError).await;
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
            .one(state.db())
            .await
        {
            Ok(Some(record)) => {
                if record.revoked_at.is_some() {
                    tracing::warn!(
                        %service_id,
                        serial_number = %cert_serial,
                        "rejected connection: certificate is revoked"
                    );
                    let _ = close_with_reason(&mut sink, CloseReason::CertificateRevoked).await;
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
                let _ = close_with_reason(&mut sink, CloseReason::CertificateNotRecognized).await;
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "certificate validation check failed");
                let _ = close_with_reason(&mut sink, CloseReason::InternalError).await;
                return;
            }
        }
    };

    // 2. Service status check.
    let service = match uptrakit_shared_db::entity::prelude::Service::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => {
            if svc.deactivated_at.is_some() {
                tracing::warn!(%service_id, "deactivated service connected with valid certificate");
                let _ = close_with_reason(&mut sink, CloseReason::ServiceDeactivated).await;
                return;
            }
            if svc.status != service_entity::ServiceStatus::Approved {
                tracing::warn!(%service_id, "rejected connection: service not approved");
                let _ = close_with_reason(&mut sink, CloseReason::ServiceNotApproved).await;
                return;
            }
            svc
        }
        Ok(None) => {
            tracing::warn!(%service_id, "rejected connection: service not found");
            let _ = close_with_reason(&mut sink, CloseReason::ServiceNotFound).await;
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "service status check failed");
            let _ = close_with_reason(&mut sink, CloseReason::InternalError).await;
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

    if let Err(e) = record_service_activity(state.db(), service_id, client_ip).await {
        tracing::error!(error = %e, %service_id, "failed to update service activity");
    }

    // Record certificate usage.
    let mut active: uptrakit_shared_db::entity::service_certificate::ActiveModel =
        cert_record.into();
    active.last_seen_at = Set(Some(now));
    if let Err(e) = active.update(state.db()).await {
        tracing::error!(error = %e, "failed to update certificate last_seen_at");
    }

    // Send ServiceSettings on connect.
    let renewal_window_hours = state.settings.renewal_window_hours();
    let ca_bundle_hash = state.ca_snapshot.borrow().bundle_hash.clone();
    use crate::service_profile::{ServiceProfile, parse_capabilities};
    let capabilities = parse_capabilities(&service.capabilities);
    let profile = ServiceProfile::from_capabilities(&capabilities);
    let shutdown_timeout = profile.shutdown_timeout_secs();
    let ping_secs = service.ping_interval_seconds.map_or_else(
        || profile.default_ping_interval_secs(),
        |v| v as u32,
    );
    let ping_interval = std::time::Duration::from_secs(u64::from(ping_secs));
    let settings_msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours,
        ca_bundle_hash,
        capabilities: controller_capabilities(),
        shutdown_timeout_seconds: shutdown_timeout,
        ping_interval,
    });
    let Some(json) = serialize_controller_msg(out_seq, settings_msg) else {
        return;
    };
    if sink.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    // Dispatch to unified capability-gated authenticated handler.
    let ctx = AuthenticatedContext {
        service_id,
        cert: cert_id,
        last_seen_at: previous_last_seen_at,
        out_seq,
        in_seq,
    };
    super::service_handler::handle_authenticated_loop(
        &mut sink,
        &mut stream,
        &state,
        ctx,
    )
    .await;
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
        .one(state.db())
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

    if let Err(e) = record_service_activity(state.db(), service_id, client_ip).await {
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

    // Dispatch to unified enrolled loop.
    super::service_handler::handle_enrolled_loop(
        &mut sink,
        &mut stream,
        &state,
        service_id,
        out_seq,
        in_seq,
    )
    .await;

    tracing::debug!(%service_id, "enrolled service disconnected");
}

// ---------------------------------------------------------------------------
// Anonymous path
// ---------------------------------------------------------------------------

/// Maximum incoming WebSocket message size (1 MB).
///
/// The largest legitimate message is `ExecuteUpdate` with plugin config and
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
    let (service_id, _enroll_capabilities, _initial_approved) = loop {
        let msg = match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => {
                tracing::debug!(error = %e, "websocket receive error");
                return;
            }
            Ok(None) => return,
            Err(_) => {
                tracing::warn!("anonymous connection timed out after {ANONYMOUS_TIMEOUT:?}");
                let _ = close_with_reason(&mut sink, CloseReason::EnrollmentTimeout).await;
                return;
            }
        };

        match msg {
            Message::Text(text) => {
                let service_msg = match deserialize_service_msg(in_seq, &text) {
                    Ok(Some(m)) => m,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::debug!(error = %e, "invalid message from anonymous client");
                        let code = match e.current_context() {
                            ServiceWsError::SequenceValidation(_) => ErrorCode::SequenceError,
                            _ => ErrorCode::BadRequest,
                        };
                        let message = e.to_string();
                        let err = ControllerMessage::Error(ErrorPayload { code, message });
                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        return;
                    }
                };

                match service_msg {
                    ServiceMessage::Enroll(payload) => {
                        let caps = payload.capabilities.clone();
                        let enrollment_result =
                            enroll_service(&state, &payload, client_ip, &mut sink, out_seq).await;

                        match enrollment_result {
                            Some((id, approved)) => {
                                break (id, caps, approved);
                            }
                            None => return, // enrollment failed, error already sent
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

    // Dispatch to unified enrolled loop.
    super::service_handler::handle_enrolled_loop(
        &mut sink,
        &mut stream,
        &state,
        service_id,
        out_seq,
        in_seq,
    )
    .await;

    tracing::debug!(%service_id, "anonymous->enrolled service disconnected");
}

// ---------------------------------------------------------------------------
// Enrollment
// ---------------------------------------------------------------------------

/// Perform service enrollment. Returns `(service_id, approved)` on success, or
/// `None` if enrollment failed (error already sent to client).
///
/// Uses the unified `do_enroll` which stores whatever capabilities the service
/// declares in its `EnrollPayload`.
async fn enroll_service(
    state: &Arc<AppState>,
    payload: &uptrakit_internal_wire::EnrollPayload,
    client_ip: Option<std::net::IpAddr>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
) -> Option<(uuid::Uuid, bool)> {
    use crate::routes::agents::{EnrollParams, ServiceStatus, do_enroll};

    let result = do_enroll(EnrollParams {
        db: state.db(),
        settings: &state.settings,
        tenant_id: state.default_tenant_id,
        hostname: &payload.hostname,
        friendly_name: &payload.friendly_name,
        enrollment_token: payload.enrollment_token.as_ref().map(|s| s.expose_secret()),
        ip_address: client_ip,
        capabilities_json: crate::service_profile::serialize_capabilities(&payload.capabilities),
    })
    .await;

    match result {
        Ok(enroll_result) => {
            let service_id = enroll_result.service.id;
            let wire_status = match enroll_result.status {
                ServiceStatus::Approved => uptrakit_internal_wire::EnrollmentStatus::Approved,
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
                "service enrolled via WS"
            );

            let approved = enroll_result.status == ServiceStatus::Approved;
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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection,
    };

    #[test]
    fn deserialize_unknown_type_returns_none() {
        let mut in_seq = IncomingSeq::new();
        // A JSON with a valid seq and seq-only-parseable structure, but an unknown message type.
        let json = r#"{"seq":1,"type":"future_message","data":{"foo":"bar"}}"#;
        let result = deserialize_service_msg(&mut in_seq, json);
        assert!(
            matches!(result, Ok(None)),
            "unknown type should return Ok(None)"
        );
    }

    #[test]
    fn deserialize_malformed_json_returns_err() {
        let mut in_seq = IncomingSeq::new();
        let result = deserialize_service_msg(&mut in_seq, "not valid json at all");
        assert!(result.is_err(), "malformed JSON should return Err");
    }

    #[test]
    fn deserialize_sequence_error_returns_err() {
        let mut in_seq = IncomingSeq::new();
        // Send seq=2 when 1 is expected → sequence validation error.
        let json = r#"{"seq":2,"type":"ping","service_ts":12345}"#;
        let result = deserialize_service_msg(&mut in_seq, json);
        assert!(
            matches!(result, Err(ref e) if matches!(e.current_context(), ServiceWsError::SequenceValidation(_))),
            "sequence mismatch should return Err(SequenceValidation)"
        );
    }

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
                capabilities TEXT NOT NULL DEFAULT '[]',
                hostname TEXT NOT NULL,
                friendly_name TEXT NOT NULL,
                ip_address TEXT,
                status TEXT NOT NULL,
                enrollment_secret_hash TEXT NOT NULL UNIQUE,
                client_version TEXT,
                last_seen_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deactivated_at INTEGER,
                ping_interval_seconds INTEGER
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
            capabilities: Set("[]".to_string()),
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
            ping_interval_seconds: Set(None),
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
