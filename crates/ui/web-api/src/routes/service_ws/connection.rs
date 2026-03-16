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
    service_profile::{ServiceProfile, parse_capabilities},
};
use uptrakit_shared_db::entity::service as service_entity;
use uptrakit_shared_db::entity::system_service as sys_svc_entity;
use uptrakit_shared_db::entity::system_service_certificate as sys_cert_entity;

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
    tenant_id: Option<uuid::Uuid>,
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

    let settings_msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
        renewal_window_hours,
        ca_bundle_hash,
        capabilities: controller_capabilities(),
        report_page_limits: uptrakit_internal_wire::ReportPageLimits::default(),
        shutdown_timeout: shutdown_timeout
            .map(|secs| std::time::Duration::from_secs(u64::from(secs))),
        ping_interval,
        tenant_id: service_status.tenant_id,
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
}
