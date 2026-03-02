//! Connection lifecycle handlers.
//!
//! Establishes a session (validates certificates, checks status, handles
//! enrollment) and then hands off to the handler loops.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use uptrakit_internal_wire::{
    ApprovedPayload, CloseReason, ControllerMessage, EnrolledPayload, ErrorCode, ErrorPayload,
    IncomingSeq, OutgoingSeq, RejectedPayload, ServiceMessage, ServiceSettingsPayload,
};
use uptrakit_shared_db::entity::service as service_entity;

use super::protocol::{
    AuthenticatedContext, CertIdentity, ServiceWsError, close_with_reason, controller_capabilities,
    deserialize_service_msg, record_service_activity, serialize_controller_msg,
};
use crate::AppState;

/// Maximum incoming WebSocket message size (1 MB).
///
/// The largest legitimate message is `ExecuteUpdate` with plugin config and
/// release assets, typically well under 100 KB. 1 MB provides ample headroom
/// while preventing memory-exhaustion DoS from oversized payloads.
pub(super) const MAX_WS_MESSAGE_SIZE: usize = 1_048_576;

/// Maximum time an anonymous WebSocket connection may remain idle before
/// sending the initial `Enroll` message.
const ANONYMOUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub(super) fn extract_bearer(headers: &HeaderMap) -> Option<String> {
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
pub(super) async fn handle_authenticated(
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
    use uptrakit_internal_wire::service_profile::{ServiceProfile, parse_capabilities};
    let capabilities = parse_capabilities(&service.capabilities);
    let profile = ServiceProfile::from_capabilities(&capabilities);
    let shutdown_timeout = profile.shutdown_timeout_secs();
    let ping_secs = service
        .ping_interval_seconds
        .map_or_else(|| profile.default_ping_interval_secs(), |v| v as u32);
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
        out_seq,
        in_seq,
    };
    super::handler::handle_authenticated_loop(&mut sink, &mut stream, &state, ctx).await;
}

// ---------------------------------------------------------------------------
// Enrolled path
// ---------------------------------------------------------------------------

/// Enrolled path: service reconnecting with Bearer secret, waiting for approval.
pub(super) async fn handle_enrolled(
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
    super::handler::handle_enrolled_loop(
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

/// Anonymous path: expects an Enroll message, then promotes in-place.
pub(super) async fn handle_anonymous(
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
    super::handler::handle_enrolled_loop(
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
        capabilities_json: uptrakit_internal_wire::service_profile::serialize_capabilities(
            &payload.capabilities,
        ),
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
                status: wire_status.clone(),
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
