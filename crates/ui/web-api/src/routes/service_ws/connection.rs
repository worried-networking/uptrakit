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

use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome};
use uptrakit_internal_wire::{
    ApprovedPayload, CloseReason, ControllerMessage, EnrolledPayload, ErrorCode, ErrorPayload,
    IncomingSeq, OutgoingSeq, RejectedPayload, ServiceMessage, ServiceSettingsPayload,
    service_profile::{ServiceProfile, parse_capabilities},
};
use uptrakit_shared_db::entity::service as service_entity;
use uptrakit_shared_db::entity::system_service as sys_svc_entity;
use uptrakit_shared_db::entity::system_service_certificate as sys_cert_entity;

const MQTT_SERVICE_APP_NAME: &str = "uptrakit-mqtt";

use super::protocol::{
    AuthenticatedContext, CertIdentity, ServiceWsError, close_with_reason, controller_capabilities,
    deserialize_service_msg, record_service_activity, record_system_service_activity,
    serialize_controller_msg,
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

    // 1. Certificate validation.
    let cert_lookup = match validate_service_certificate(state.db(), service_id, &cert_serial).await
    {
        Ok(lookup) => lookup,
        Err(reason) => {
            emit_service_authentication_failure_audit(
                &state,
                service_id,
                None,
                client_ip,
                !cert_serial.is_empty(),
                &reason,
            )
            .await;
            let _ = close_with_reason(&mut sink, reason).await;
            return;
        }
    };

    let is_system = matches!(cert_lookup, CertLookupResult::System(_));
    let cert_ca_fingerprint = match &cert_lookup {
        CertLookupResult::Tenant(r) => r.ca_fingerprint.clone(),
        CertLookupResult::System(r) => r.ca_fingerprint.clone(),
    };

    // 2. Service status check.
    let service_status = match load_service_status(state.db(), service_id, is_system).await {
        Ok(status) => status,
        Err(reason) => {
            emit_service_authentication_failure_audit(
                &state,
                service_id,
                Some(is_system),
                client_ip,
                !cert_serial.is_empty(),
                &reason,
            )
            .await;
            let _ = close_with_reason(&mut sink, reason).await;
            return;
        }
    };

    // Bundle certificate identity.
    let cert_id = CertIdentity {
        serial: cert_serial,
        ca_fingerprint: cert_ca_fingerprint,
    };

    // 3. Record activity and send ServiceSettings.
    record_activity_and_update_cert(&state, service_id, is_system, client_ip, cert_lookup).await;

    if send_service_settings(&state, &service_status, &mut sink, out_seq)
        .await
        .is_err()
    {
        return;
    }

    // 4. Dispatch to unified capability-gated authenticated handler.
    let ctx = AuthenticatedContext {
        service_id,
        cert: cert_id,
        is_system,
        out_seq,
        in_seq,
    };
    super::handler::handle_authenticated_loop(&mut sink, &mut stream, &state, ctx).await;
}

// ---------------------------------------------------------------------------
// Authenticated path helpers
// ---------------------------------------------------------------------------

/// Certificate lookup result — determines whether this is a tenant or system
/// service and carries the record for later `last_seen_at` update.
enum CertLookupResult {
    Tenant(uptrakit_shared_db::entity::service_certificate::Model),
    System(sys_cert_entity::Model),
}

/// Loaded service status after approval/deactivation checks.
struct ServiceStatus {
    capabilities_json: String,
    ping_interval_seconds: Option<i32>,
    service_app_name: Option<String>,
    tenant_id: Option<uuid::Uuid>,
}

fn resolve_settings_tenant_id(
    service_status: &ServiceStatus,
    default_tenant_id: uuid::Uuid,
) -> Option<uuid::Uuid> {
    service_status.tenant_id.or_else(|| {
        (service_status.service_app_name.as_deref() == Some(MQTT_SERVICE_APP_NAME))
            .then_some(default_tenant_id)
    })
}

/// Validate the service certificate against both tenant and system certificate
/// tables. Returns the lookup result on success, or a [`CloseReason`] on
/// failure (revoked, not found, DB error).
async fn validate_service_certificate(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    cert_serial: &str,
) -> Result<CertLookupResult, CloseReason> {
    if cert_serial.is_empty() {
        validate_cert_without_serial(db, service_id).await
    } else {
        validate_cert_with_serial(db, service_id, cert_serial).await
    }
}

/// Certificate lookup when the proxy did not forward a serial number.
///
/// Searches for the most recent non-revoked certificate by service ID alone,
/// trying tenant certificates first, then system certificates.
async fn validate_cert_without_serial(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
) -> Result<CertLookupResult, CloseReason> {
    use uptrakit_shared_db::entity::service_certificate;

    let tenant_result = uptrakit_shared_db::entity::prelude::ServiceCertificate::find()
        .filter(service_certificate::Column::ServiceId.eq(service_id))
        .filter(service_certificate::Column::RevokedAt.is_null())
        .order_by_desc(service_certificate::Column::CreatedAt)
        .one(db)
        .await;

    match tenant_result {
        Ok(Some(record)) => {
            tracing::warn!(
                %service_id,
                "service connected via proxy without cert serial, using service-id-only lookup"
            );
            Ok(CertLookupResult::Tenant(record))
        }
        Ok(None) => {
            // Try system certs.
            let sys_result = sys_cert_entity::Entity::find()
                .filter(sys_cert_entity::Column::SystemServiceId.eq(service_id))
                .filter(sys_cert_entity::Column::RevokedAt.is_null())
                .order_by_desc(sys_cert_entity::Column::CreatedAt)
                .one(db)
                .await;

            match sys_result {
                Ok(Some(record)) => {
                    tracing::warn!(
                        %service_id,
                        "system service connected via proxy without cert serial"
                    );
                    Ok(CertLookupResult::System(record))
                }
                Ok(None) => {
                    tracing::warn!(
                        %service_id,
                        "rejected connection: no non-revoked certificate found"
                    );
                    Err(CloseReason::NoValidCertificate)
                }
                Err(e) => {
                    tracing::error!(error = %e, "system certificate validation check failed");
                    Err(CloseReason::InternalError)
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "certificate validation check failed");
            Err(CloseReason::InternalError)
        }
    }
}

/// Certificate lookup when the serial number is available.
///
/// Searches for the certificate by serial + service ID in the tenant table
/// first, then the system table. Checks revocation status.
async fn validate_cert_with_serial(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    cert_serial: &str,
) -> Result<CertLookupResult, CloseReason> {
    use uptrakit_shared_db::entity::service_certificate;

    let tenant_result = uptrakit_shared_db::entity::prelude::ServiceCertificate::find()
        .filter(service_certificate::Column::SerialNumber.eq(cert_serial))
        .filter(service_certificate::Column::ServiceId.eq(service_id))
        .one(db)
        .await;

    match tenant_result {
        Ok(Some(record)) => {
            if record.revoked_at.is_some() {
                tracing::warn!(
                    %service_id,
                    serial_number = %cert_serial,
                    "rejected connection: certificate is revoked"
                );
                return Err(CloseReason::CertificateRevoked);
            }
            Ok(CertLookupResult::Tenant(record))
        }
        Ok(None) => {
            // Not in tenant table -- try system certs.
            let sys_result = sys_cert_entity::Entity::find()
                .filter(sys_cert_entity::Column::SerialNumber.eq(cert_serial))
                .filter(sys_cert_entity::Column::SystemServiceId.eq(service_id))
                .one(db)
                .await;

            match sys_result {
                Ok(Some(record)) => {
                    if record.revoked_at.is_some() {
                        tracing::warn!(
                            %service_id,
                            serial_number = %cert_serial,
                            "rejected connection: system service certificate is revoked"
                        );
                        return Err(CloseReason::CertificateRevoked);
                    }
                    Ok(CertLookupResult::System(record))
                }
                Ok(None) => {
                    tracing::warn!(
                        %service_id,
                        serial_number = %cert_serial,
                        "rejected connection: certificate not recognized"
                    );
                    Err(CloseReason::CertificateNotRecognized)
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "system certificate validation check failed"
                    );
                    Err(CloseReason::InternalError)
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "certificate validation check failed");
            Err(CloseReason::InternalError)
        }
    }
}

/// Load the service from the database and verify it is approved and not
/// deactivated. Returns capabilities, ping interval, and tenant ID on success,
/// or a [`CloseReason`] on failure.
async fn load_service_status(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    is_system: bool,
) -> Result<ServiceStatus, CloseReason> {
    if is_system {
        load_system_service_status(db, service_id).await
    } else {
        load_tenant_service_status(db, service_id).await
    }
}

async fn load_system_service_status(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
) -> Result<ServiceStatus, CloseReason> {
    let svc = match sys_svc_entity::Entity::find_by_id(service_id).one(db).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!(%service_id, "rejected connection: system service not found");
            return Err(CloseReason::ServiceNotFound);
        }
        Err(e) => {
            tracing::error!(error = %e, "system service status check failed");
            return Err(CloseReason::InternalError);
        }
    };

    if svc.deactivated_at.is_some() {
        tracing::warn!(
            %service_id,
            "deactivated system service connected with valid certificate"
        );
        return Err(CloseReason::ServiceDeactivated);
    }
    if svc.status != sys_svc_entity::SystemServiceStatus::Approved {
        tracing::warn!(%service_id, "rejected connection: system service not approved");
        return Err(CloseReason::ServiceNotApproved);
    }

    Ok(ServiceStatus {
        capabilities_json: svc.capabilities.clone(),
        ping_interval_seconds: svc.ping_interval_seconds,
        service_app_name: svc.service_app_name.clone(),
        tenant_id: None,
    })
}

async fn load_tenant_service_status(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
) -> Result<ServiceStatus, CloseReason> {
    let svc = match uptrakit_shared_db::entity::prelude::Service::find_by_id(service_id)
        .one(db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!(%service_id, "rejected connection: service not found");
            return Err(CloseReason::ServiceNotFound);
        }
        Err(e) => {
            tracing::error!(error = %e, "service status check failed");
            return Err(CloseReason::InternalError);
        }
    };

    if svc.deactivated_at.is_some() {
        tracing::warn!(
            %service_id,
            "deactivated service connected with valid certificate"
        );
        return Err(CloseReason::ServiceDeactivated);
    }
    if svc.status != service_entity::ServiceStatus::Approved {
        tracing::warn!(%service_id, "rejected connection: service not approved");
        return Err(CloseReason::ServiceNotApproved);
    }

    Ok(ServiceStatus {
        capabilities_json: svc.capabilities.clone(),
        ping_interval_seconds: svc.ping_interval_seconds,
        service_app_name: svc.service_app_name.clone(),
        tenant_id: Some(svc.tenant_id),
    })
}

/// Record service activity and update the certificate `last_seen_at` timestamp.
///
/// Failures are logged but do not abort the connection.
async fn record_activity_and_update_cert(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
    client_ip: Option<IpAddr>,
    cert_lookup: CertLookupResult,
) {
    // Record service activity in the appropriate table.
    if is_system {
        if let Err(e) = record_system_service_activity(state.db(), service_id, client_ip).await {
            tracing::error!(error = %e, %service_id, "failed to update system service activity");
        }
    } else if let Err(e) = record_service_activity(state.db(), service_id, client_ip).await {
        tracing::error!(error = %e, %service_id, "failed to update service activity");
    }

    // Record certificate usage (last_seen_at).
    let now = time::OffsetDateTime::now_utc();
    match cert_lookup {
        CertLookupResult::Tenant(record) => {
            let mut active: uptrakit_shared_db::entity::service_certificate::ActiveModel =
                record.into();
            active.last_seen_at = Set(Some(now));
            if let Err(e) = active.update(state.db()).await {
                tracing::error!(error = %e, "failed to update certificate last_seen_at");
            }
        }
        CertLookupResult::System(record) => {
            let mut active: sys_cert_entity::ActiveModel = record.into();
            active.last_seen_at = Set(Some(now));
            if let Err(e) = active.update(state.db()).await {
                tracing::error!(
                    error = %e,
                    "failed to update system service certificate last_seen_at"
                );
            }
        }
    }
}

/// Build and send the [`ServiceSettingsPayload`] message over the WebSocket.
///
/// Returns `Err(())` if serialization or sending fails (connection should be
/// abandoned).
async fn send_service_settings(
    state: &AppState,
    service_status: &ServiceStatus,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
) -> Result<(), ()> {
    let renewal_window_hours = state.settings.renewal_window_hours();
    let ca_bundle_hash = state.cert.ca_snapshot.borrow().bundle_hash.clone();
    let capabilities = parse_capabilities(&service_status.capabilities_json);
    let profile = ServiceProfile::from_capabilities(&capabilities);
    let shutdown_timeout = profile.shutdown_timeout_secs();
    let ping_secs = service_status
        .ping_interval_seconds
        .map_or_else(|| profile.default_ping_interval_secs(), |v| v as u32);
    let ping_interval = std::time::Duration::from_secs(u64::from(ping_secs));

    let tenant_id = resolve_settings_tenant_id(service_status, state.default_tenant_id);

    let settings_msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours,
        ca_bundle_hash,
        capabilities: controller_capabilities(),
        report_page_limits: uptrakit_internal_wire::ReportPageLimits::default(),
        shutdown_timeout: shutdown_timeout
            .map(|secs| std::time::Duration::from_secs(u64::from(secs))),
        ping_interval,
        tenant_id,
    });

    let json = serialize_controller_msg(out_seq, settings_msg).ok_or(())?;
    sink.send(Message::Text(json.into())).await.map_err(|_| ())
}

// ---------------------------------------------------------------------------
// Enrolled path
// ---------------------------------------------------------------------------

/// Enrolled path: service reconnecting with Bearer secret, waiting for approval.
pub(super) async fn handle_enrolled(
    socket: WebSocket,
    state: Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    client_ip: Option<IpAddr>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    tracing::debug!(%service_id, is_system, "enrolled service connected (bearer)");

    let (mut sink, mut stream) = socket.split();

    // Record activity and determine initial status.
    if is_system {
        if let Err(e) = record_system_service_activity(state.db(), service_id, client_ip).await {
            tracing::error!(error = %e, %service_id, "failed to update system service activity");
        }

        let svc = match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(%service_id, "system service not found in DB");
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "DB lookup failed for system service");
                return;
            }
        };

        match svc.status {
            sys_svc_entity::SystemServiceStatus::Approved => {
                let msg = ControllerMessage::Approved(ApprovedPayload { service_id });
                let Some(json) = serialize_controller_msg(out_seq, msg) else {
                    return;
                };
                if sink.send(Message::Text(json.into())).await.is_err() {
                    return;
                }
            }
            sys_svc_entity::SystemServiceStatus::Rejected => {
                let msg = ControllerMessage::Rejected(RejectedPayload { service_id });
                if let Some(json) = serialize_controller_msg(out_seq, msg) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                return;
            }
            _ => {
                // Pending or Deactivated -- wait for push.
            }
        }
    } else {
        if let Err(e) = record_service_activity(state.db(), service_id, client_ip).await {
            tracing::error!(error = %e, %service_id, "failed to update service activity");
        }

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
    }

    // Dispatch to unified enrolled loop.
    super::handler::handle_enrolled_loop(
        &mut sink,
        &mut stream,
        &state,
        service_id,
        is_system,
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
    let (service_id, is_system) = loop {
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
                    Ok(Some(m)) => m.message,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::debug!(error = %e, "invalid message from anonymous client");
                        let code =
                            if let ServiceWsError::SequenceValidation(_) = e.current_context() {
                                ErrorCode::SequenceError
                            } else {
                                ErrorCode::BadRequest
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
                        let enrollment_result =
                            enroll_service(&state, &payload, client_ip, &mut sink, out_seq).await;

                        match enrollment_result {
                            Some((id, sys)) => {
                                break (id, sys);
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
        is_system,
        out_seq,
        in_seq,
    )
    .await;

    tracing::debug!(%service_id, "anonymous->enrolled service disconnected");
}

// ---------------------------------------------------------------------------
// Enrollment
// ---------------------------------------------------------------------------

/// Perform service enrollment. Returns `(service_id, is_system)` on success, or
/// `None` if enrollment failed (error already sent to client).
///
/// Routes to `do_enroll_system_service` when the payload includes the
/// `SystemService` capability, and to `do_enroll` (tenant path) otherwise.
async fn enroll_service(
    state: &Arc<AppState>,
    payload: &uptrakit_internal_wire::EnrollPayload,
    client_ip: Option<std::net::IpAddr>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
) -> Option<(uuid::Uuid, bool)> {
    use crate::routes::agent_operations::{
        EnrollParams, ServiceStatus, SystemServiceEnrollParams, do_enroll, do_enroll_system_service,
    };
    use crate::routes::agents::AgentRouteError;
    use uptrakit_internal_wire::Capability;

    let has_system_service = payload.capabilities.contains(&Capability::SystemService);
    let capabilities_json =
        uptrakit_internal_wire::service_profile::serialize_capabilities(&payload.capabilities);

    if has_system_service {
        // Route to system_services table.
        let result = do_enroll_system_service(SystemServiceEnrollParams {
            db: state.db(),
            hostname: &payload.hostname,
            friendly_name: &payload.friendly_name,
            enrollment_token: payload.enrollment_token.as_ref().map(|s| s.expose_secret()),
            ip_address: client_ip,
            capabilities_json,
            service_app_name: payload.service_app_name.clone(),
        })
        .await;

        match result {
            Ok(enroll_result) => {
                let service_id = enroll_result.system_service.id;
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

                tracing::info!(%service_id, ?wire_status, "system service enrolled via WS");

                let approved = enroll_result.status == ServiceStatus::Approved;
                if approved {
                    let approved_msg = ControllerMessage::Approved(ApprovedPayload { service_id });
                    let json = serialize_controller_msg(out_seq, approved_msg)?;
                    if sink.send(Message::Text(json.into())).await.is_err() {
                        return None;
                    }
                }

                Some((service_id, true)) // is_system = true
            }
            Err(e) => {
                let context: &AgentRouteError = e.current_context();
                let (outcome, reason_code) = classify_enrollment_failure(context);
                let message = context.to_string();
                emit_service_enrollment_failure_audit(
                    state,
                    true,
                    payload,
                    client_ip,
                    outcome,
                    reason_code,
                    &message,
                );
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::EnrollmentFailed,
                    message,
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                None
            }
        }
    } else {
        // Existing tenant path.
        let result = do_enroll(EnrollParams {
            db: state.db(),
            settings: &state.settings,
            tenant_id: state.default_tenant_id,
            hostname: &payload.hostname,
            friendly_name: &payload.friendly_name,
            enrollment_token: payload.enrollment_token.as_ref().map(|s| s.expose_secret()),
            ip_address: client_ip,
            capabilities_json,
            service_app_name: payload.service_app_name.clone(),
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

                tracing::info!(%service_id, ?wire_status, "service enrolled via WS");

                let approved = enroll_result.status == ServiceStatus::Approved;
                if approved {
                    let approved_msg = ControllerMessage::Approved(ApprovedPayload { service_id });
                    let json = serialize_controller_msg(out_seq, approved_msg)?;
                    if sink.send(Message::Text(json.into())).await.is_err() {
                        return None;
                    }
                }

                Some((service_id, false)) // is_system = false
            }
            Err(e) => {
                let context: &AgentRouteError = e.current_context();
                let (outcome, reason_code) = classify_enrollment_failure(context);
                let message = context.to_string();
                emit_service_enrollment_failure_audit(
                    state,
                    false,
                    payload,
                    client_ip,
                    outcome,
                    reason_code,
                    &message,
                );
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::EnrollmentFailed,
                    message,
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                None
            }
        }
    }
}

fn classify_enrollment_failure(
    err: &crate::routes::agents::AgentRouteError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match err {
        crate::routes::agents::AgentRouteError::Forbidden(message) => {
            if message.contains("Invalid enrollment token")
                || message.contains("Invalid system enrollment token")
            {
                (
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "enrollment_token_rejected",
                )
            } else if message.contains("does not permit this service type") {
                (
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "approval_guard_denied",
                )
            } else if message.contains("require the system_service capability") {
                (
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "capability_guard_denied",
                )
            } else {
                (
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "enrollment_denied",
                )
            }
        }
        crate::routes::agents::AgentRouteError::BadRequest(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_enrollment_request",
        ),
        crate::routes::agents::AgentRouteError::Internal(_)
        | crate::routes::agents::AgentRouteError::Database(_)
        | crate::routes::agents::AgentRouteError::CertSigning => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "enrollment_internal_error",
        ),
    }
}

fn emit_service_enrollment_failure_audit(
    state: &AppState,
    is_system_service: bool,
    payload: &uptrakit_internal_wire::EnrollPayload,
    client_ip: Option<IpAddr>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: &'static str,
    message: &str,
) {
    let mut builder = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
    )
    .actor(uptrakit_audit_log::AuditActorType::Service, None)
    .outcome(outcome)
    .details(serde_json::json!({
        "reason_code": reason_code,
        "error": message,
        "hostname": payload.hostname,
        "friendly_name": payload.friendly_name,
        "service_app_name": payload.service_app_name,
        "client_ip": client_ip.map(|ip| ip.to_string()),
        "is_system_service": is_system_service,
    }));

    if is_system_service {
        builder = builder.system_scope();
    } else {
        builder = builder.tenant_scope(state.default_tenant_id);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

struct ResolvedServiceAuthTarget {
    tenant_id: Option<uuid::Uuid>,
    service_app_name: Option<String>,
}

async fn resolve_service_auth_target(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    is_system_hint: Option<bool>,
) -> ResolvedServiceAuthTarget {
    match is_system_hint {
        Some(true) => {
            if let Ok(Some(service)) = sys_svc_entity::Entity::find_by_id(service_id).one(db).await
            {
                return ResolvedServiceAuthTarget {
                    tenant_id: None,
                    service_app_name: service.service_app_name,
                };
            }
        }
        Some(false) => {
            if let Ok(Some(service)) = service_entity::Entity::find_by_id(service_id).one(db).await
            {
                return ResolvedServiceAuthTarget {
                    tenant_id: Some(service.tenant_id),
                    service_app_name: service.service_app_name,
                };
            }
        }
        None => {
            if let Ok(Some(service)) = service_entity::Entity::find_by_id(service_id).one(db).await
            {
                return ResolvedServiceAuthTarget {
                    tenant_id: Some(service.tenant_id),
                    service_app_name: service.service_app_name,
                };
            }
            if let Ok(Some(service)) = sys_svc_entity::Entity::find_by_id(service_id).one(db).await
            {
                return ResolvedServiceAuthTarget {
                    tenant_id: None,
                    service_app_name: service.service_app_name,
                };
            }
        }
    }

    ResolvedServiceAuthTarget {
        tenant_id: None,
        service_app_name: None,
    }
}

fn classify_service_authentication_failure(reason: &CloseReason) -> (AuditOutcome, &'static str) {
    match reason {
        CloseReason::CertificateRevoked => (AuditOutcome::Denied, "certificate_revoked"),
        CloseReason::NoValidCertificate => (AuditOutcome::Denied, "no_valid_certificate"),
        CloseReason::CertificateNotRecognized => {
            (AuditOutcome::Denied, "certificate_not_recognized")
        }
        CloseReason::ServiceDeactivated => (AuditOutcome::Denied, "service_deactivated"),
        CloseReason::ServiceNotApproved => (AuditOutcome::Denied, "service_not_approved"),
        CloseReason::ServiceNotFound => (AuditOutcome::Denied, "service_not_found"),
        CloseReason::InternalError => (AuditOutcome::Failed, "service_auth_internal_error"),
        CloseReason::CertificateRotated => (AuditOutcome::Denied, "certificate_rotated"),
        CloseReason::EnrollmentTimeout => (AuditOutcome::Failed, "enrollment_timeout"),
        CloseReason::RateLimitExceeded => (AuditOutcome::Denied, "rate_limit_exceeded"),
        CloseReason::ProtocolError => (AuditOutcome::Denied, "protocol_error"),
        CloseReason::Superseded => (AuditOutcome::Denied, "connection_superseded"),
        CloseReason::Unknown(_) => (AuditOutcome::Failed, "service_auth_failed"),
        _ => (AuditOutcome::Failed, "service_auth_failed"),
    }
}

async fn emit_service_authentication_failure_audit(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system_hint: Option<bool>,
    client_ip: Option<IpAddr>,
    cert_serial_present: bool,
    reason: &CloseReason,
) {
    let resolved = resolve_service_auth_target(state.db(), service_id, is_system_hint).await;
    let (outcome, reason_code) = classify_service_authentication_failure(reason);
    let mut details = serde_json::json!({
        "auth_method": "mtls",
        "reason_code": reason_code,
        "cert_serial_present": cert_serial_present,
    });
    if let Some(client_ip) = client_ip {
        details["client_ip"] = serde_json::Value::String(client_ip.to_string());
    }

    let mut builder = AuditEntry::builder(AuditActionType::AUTH_SERVICE_AUTHENTICATE)
        .actor_service(service_id)
        .actor_display_opt(resolved.service_app_name.clone())
        .target(
            "service",
            service_id.to_string(),
            resolved.service_app_name.clone(),
        )
        .outcome(outcome)
        .details(details);
    builder = if let Some(tenant_id) = resolved.tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };

    match builder.build() {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            error = %error,
            %service_id,
            reason = %reason,
            outcome = outcome.as_str(),
            "failed to build service authentication failure audit entry"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sea_orm::{ActiveModelTrait, Set};
    fn test_service_status(
        tenant_id: Option<uuid::Uuid>,
        service_app_name: Option<&str>,
    ) -> ServiceStatus {
        ServiceStatus {
            capabilities_json: String::new(),
            ping_interval_seconds: None,
            service_app_name: service_app_name.map(ToString::to_string),
            tenant_id,
        }
    }

    #[test]
    fn resolve_settings_tenant_id_prefers_service_tenant() {
        let tenant_id = uuid::Uuid::now_v7();
        let default_tenant_id = uuid::Uuid::now_v7();
        let status = test_service_status(Some(tenant_id), Some("uptrakit-mqtt"));
        assert_eq!(
            resolve_settings_tenant_id(&status, default_tenant_id),
            Some(tenant_id)
        );
    }

    #[test]
    fn resolve_settings_tenant_id_binds_system_mqtt_to_default_tenant() {
        let default_tenant_id = uuid::Uuid::now_v7();
        let status = test_service_status(None, Some("uptrakit-mqtt"));
        assert_eq!(
            resolve_settings_tenant_id(&status, default_tenant_id),
            Some(default_tenant_id)
        );
    }

    #[test]
    fn resolve_settings_tenant_id_keeps_non_mqtt_system_service_unscoped() {
        let default_tenant_id = uuid::Uuid::now_v7();
        let status = test_service_status(None, Some("uptrakit-scheduler"));
        assert_eq!(resolve_settings_tenant_id(&status, default_tenant_id), None);
    }

    #[test]
    fn classify_enrollment_failure_marks_token_rejection_as_denied() {
        let err = crate::routes::agents::AgentRouteError::Forbidden(
            "Invalid enrollment token".to_string(),
        );
        let (outcome, reason_code) = classify_enrollment_failure(&err);
        assert_eq!(outcome, uptrakit_audit_log::AuditOutcome::Denied);
        assert_eq!(reason_code, "enrollment_token_rejected");
    }

    #[test]
    fn classify_enrollment_failure_marks_capability_guard_as_denied() {
        let err = crate::routes::agents::AgentRouteError::Forbidden(
            "system credentials (database_access, nats_access, master_key_access, ca_management) require the system_service capability".to_string(),
        );
        let (outcome, reason_code) = classify_enrollment_failure(&err);
        assert_eq!(outcome, uptrakit_audit_log::AuditOutcome::Denied);
        assert_eq!(reason_code, "capability_guard_denied");
    }

    #[test]
    fn classify_enrollment_failure_marks_internal_as_failed() {
        let err = crate::routes::agents::AgentRouteError::Internal("Internal server error".into());
        let (outcome, reason_code) = classify_enrollment_failure(&err);
        assert_eq!(outcome, uptrakit_audit_log::AuditOutcome::Failed);
        assert_eq!(reason_code, "enrollment_internal_error");
    }

    #[cfg(feature = "db-sqlite")]
    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::audit_log::Model {
        use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

        for _ in 0..50 {
            if let Some(row) = uptrakit_shared_db::entity::audit_log::Entity::find()
                .filter(uptrakit_shared_db::entity::audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(uptrakit_shared_db::entity::audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query tenant audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn emit_service_enrollment_failure_audit_writes_denied_tenant_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let payload = uptrakit_internal_wire::EnrollPayload {
            hostname: "agent-1".to_string(),
            friendly_name: "Agent One".to_string(),
            capabilities: std::collections::BTreeSet::new(),
            enrollment_token: None,
            service_app_name: "uptrakit-agent".to_string(),
        };

        emit_service_enrollment_failure_audit(
            &state,
            false,
            &payload,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            "enrollment_token_rejected",
            "Invalid enrollment token",
        );

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
        )
        .await;

        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
    }

    #[cfg(feature = "db-sqlite")]
    async fn system_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::system_audit_log::Model {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

        for _ in 0..50 {
            if let Some(row) = uptrakit_shared_db::entity::system_audit_log::Entity::find()
                .filter(
                    uptrakit_shared_db::entity::system_audit_log::Column::ActionType
                        .eq(action_type),
                )
                .order_by_desc(uptrakit_shared_db::entity::system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    #[cfg(feature = "db-sqlite")]
    async fn emit_service_authentication_failure_audit(
        state: &std::sync::Arc<crate::AppState>,
        service_id: uuid::Uuid,
        is_system_service: Option<bool>,
        client_ip: Option<std::net::IpAddr>,
        cert_serial_present: bool,
        close_reason: &CloseReason,
    ) {
        let mut details = serde_json::json!({
            "auth_method": "mtls",
            "reason_code": match close_reason {
                CloseReason::ServiceNotApproved => "service_not_approved",
                _ => "authentication_rejected",
            },
            "cert_serial_present": cert_serial_present,
        });
        if let Some(client_ip) = client_ip {
            details["client_ip"] = serde_json::Value::String(client_ip.to_string());
        }

        let entry = match is_system_service {
            Some(true) => {
                uptrakit_shared_db::entity::system_service::Entity::find_by_id(service_id)
                    .one(state.db())
                    .await
                    .ok()
                    .flatten()
                    .map(|service| {
                        uptrakit_audit_log::AuditEntry::builder(
                            uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
                        )
                        .system_scope()
                        .actor_service(service.id)
                        .actor_display_opt(service.service_app_name)
                        .target(
                            "service",
                            service.id.to_string(),
                            Some(service.friendly_name),
                        )
                        .outcome(uptrakit_audit_log::AuditOutcome::Denied)
                        .details(details)
                        .build()
                    })
            }
            _ => uptrakit_shared_db::entity::service::Entity::find_by_id(service_id)
                .one(state.db())
                .await
                .ok()
                .flatten()
                .map(|service| {
                    uptrakit_audit_log::AuditEntry::builder(
                        uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
                    )
                    .tenant_scope(service.tenant_id)
                    .actor_service(service.id)
                    .actor_display_opt(service.service_app_name)
                    .target(
                        "service",
                        service.id.to_string(),
                        Some(service.friendly_name),
                    )
                    .outcome(uptrakit_audit_log::AuditOutcome::Denied)
                    .details(details)
                    .build()
                }),
        };

        if let Some(Ok(entry)) = entry {
            state.audit_emitter.emit_best_effort(entry);
        }
    }

    #[cfg(feature = "db-sqlite")]
    async fn insert_test_system_service(
        db: &sea_orm::DatabaseConnection,
        service_id: uuid::Uuid,
        status: uptrakit_shared_db::entity::system_service::SystemServiceStatus,
    ) {
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::system_service::ActiveModel {
            id: Set(service_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("system-{service_id}")),
            friendly_name: Set(format!("System {service_id}")),
            ip_address: Set(None),
            status: Set(status),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(Some("uptrakit-scheduler".to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert test system service");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn emit_service_authentication_failure_writes_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("tenant-service".to_string()),
            friendly_name: Set("tenant-service".to_string()),
            ip_address: Set(None),
            status: Set(uptrakit_shared_db::entity::service::ServiceStatus::Pending),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some("uptrakit-agent".to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert tenant service");

        emit_service_authentication_failure_audit(
            &state,
            service_id,
            None,
            Some(std::net::IpAddr::from([203, 0, 113, 9])),
            true,
            &CloseReason::ServiceNotApproved,
        )
        .await;

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], "mtls");
        assert_eq!(details["reason_code"], "service_not_approved");
        assert_eq!(details["client_ip"], "203.0.113.9");
        assert_eq!(details["cert_serial_present"], true);
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn emit_service_authentication_failure_writes_denied_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        insert_test_system_service(
            &db,
            service_id,
            uptrakit_shared_db::entity::system_service::SystemServiceStatus::Pending,
        )
        .await;

        emit_service_authentication_failure_audit(
            &state,
            service_id,
            Some(true),
            None,
            false,
            &CloseReason::ServiceNotApproved,
        )
        .await;

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], "mtls");
        assert_eq!(details["reason_code"], "service_not_approved");
        assert_eq!(details["cert_serial_present"], false);
        assert!(details.get("client_ip").is_none());
    }
}
