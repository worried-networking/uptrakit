use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uptrakit_internal_wire::{
    ApprovedPayload, CertificatePayload, ControllerMessage, ErrorCode, ErrorPayload, IncomingSeq,
    MqttClientConnectionStatus as WireMqttClientConnectionStatus, MqttRegisteredPayload,
    MqttTenantAssignmentsPayload, OutgoingSeq, PingPayload, RejectedPayload, ServiceMessage,
};
use uptrakit_shared_db::entity::{
    service as mqtt_service, service_certificate as mqtt_service_certificate,
};
use uptrakit_web_api_types::settings_mqtt::MqttClientConnectionStatus as ApiMqttClientConnectionStatus;

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

use super::service_ws::{
    close_with_reason, deserialize_service_msg, send_pong, serialize_controller_msg,
};
use crate::AppState;
use crate::mqtt_client_store;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub(crate) enum MqttWsError {
    #[error("{0}")]
    Enrollment(String),
    #[error("{0}")]
    Certificate(String),
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("PEM parse error")]
    PemParse,
    #[error("X.509 parse error")]
    X509Parse,
    #[error("invalid timestamp: {0}")]
    Timestamp(String),
    #[error("certificate not found")]
    CertNotFound,
}

type MqttWsResult<T> = std::result::Result<T, Report<MqttWsError>>;

impl_report_conversion!(sea_orm::DbErr => MqttWsError::Database);

// ---------------------------------------------------------------------------
// Authenticated MQTT handler (called from service_ws after shared auth)
// ---------------------------------------------------------------------------

/// Service-type-specific handler for an authenticated MQTT connection.
///
/// Called by [`super::service_ws`] after certificate validation, service status
/// check, and sending `ServiceSettings`. Waits for a `Register` message, then
/// enters the MQTT-specific operational loop (Ping/Pong, ReleaseTenants,
/// RenewCertificate).
pub(crate) async fn handle_mqtt_authenticated(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    ctx: super::service_ws::AuthenticatedContext<'_>,
) {
    let super::service_ws::AuthenticatedContext {
        service_id,
        cert,
        last_seen_at,
        out_seq,
        in_seq,
    } = ctx;
    // Wait for Register message before entering operational loop.
    let (instance_id, max_tenants, active_mqtt_clients) = loop {
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
                let service_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "deserialize error");
                        return;
                    }
                };

                match service_msg {
                    ServiceMessage::Register(payload) => {
                        if payload.protocol_version != uptrakit_internal_wire::PROTOCOL_VERSION {
                            tracing::warn!(
                                %service_id,
                                reported = payload.protocol_version,
                                expected = uptrakit_internal_wire::PROTOCOL_VERSION,
                                "MQTT service protocol version mismatch"
                            );
                        }
                        break (
                            payload.instance_id,
                            payload.max_tenants,
                            payload.active_mqtt_clients,
                        );
                    }
                    ServiceMessage::Ping(PingPayload { service_ts }) => {
                        if send_pong(sink, out_seq, service_ts).await.is_err() {
                            return;
                        }
                    }
                    _ => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: ErrorCode::BadRequest,
                            message: "expected register message".to_string(),
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

    // Register in connection registry.
    let (mut push_rx, cancel_token) = state
        .service_connections
        .register_mqtt(service_id, instance_id.clone(), max_tenants)
        .await;

    // Send Registered acknowledgment.
    let registered_msg = ControllerMessage::Registered(MqttRegisteredPayload {
        instance_id: instance_id.clone(),
    });
    let Some(json) = serialize_controller_msg(out_seq, registered_msg) else {
        state.service_connections.unregister(&service_id).await;
        return;
    };
    if sink.send(Message::Text(json.into())).await.is_err() {
        state.service_connections.unregister(&service_id).await;
        return;
    }

    // Create lease coordinator.
    let lease_coordinator =
        MqttLeaseCoordinator::new(state.db.clone(), state.service_connections.clone());

    // Reconcile MQTT clients if reconnecting with active clients.
    let tenant_configs = if !active_mqtt_clients.is_empty() {
        match lease_coordinator
            .reconcile_mqtt_clients(service_id, &instance_id, &active_mqtt_clients)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to reconcile mqtt clients");
                vec![]
            }
        }
    } else {
        let requested = if max_tenants == 0 { 100 } else { max_tenants };
        match lease_coordinator
            .assign_available_tenants(service_id, &instance_id, requested)
            .await
        {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to assign mqtt clients");
                vec![]
            }
        }
    };

    // Send initial tenant assignments.
    if !tenant_configs.is_empty() {
        let assignments_msg = ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload {
            tenants: tenant_configs,
        });
        let Some(json) = serialize_controller_msg(out_seq, assignments_msg) else {
            state.service_connections.unregister(&service_id).await;
            return;
        };
        if sink.send(Message::Text(json.into())).await.is_err() {
            state.service_connections.unregister(&service_id).await;
            return;
        }
    }

    tracing::info!(%service_id, instance_id = %instance_id, "MQTT service registered");

    let delivered = state
        .notification_service
        .deliver_backlog_for_authenticated_service(
            service_id,
            uptrakit_shared_db::entity::service::ServiceType::Mqtt,
            last_seen_at,
        )
        .await;
    if delivered > 0 {
        tracing::info!(
            %service_id,
            delivered,
            "delivered outbox backlog to MQTT service"
        );
    }

    // Enter operational loop.
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
                        let service_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match service_msg {
                            ServiceMessage::Ping(PingPayload { service_ts }) => {
                                let Ok(controller_ts) = send_pong(sink, out_seq, service_ts).await else { break };
                                tracing::trace!(service_ts, controller_ts, "ping/pong");

                                // Update lease heartbeats for all tenants held by this service
                                if let Err(e) = lease_coordinator
                                    .record_heartbeat(&service_id)
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to record heartbeat");
                                }
                            }
                            ServiceMessage::ReleaseTenants(payload) => {
                                if let Err(e) = lease_coordinator
                                    .release_mqtt_clients(&service_id, &payload.mqtt_client_ids)
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to release mqtt clients");
                                }

                                tracing::info!(
                                    %service_id,
                                    count = payload.mqtt_client_ids.len(),
                                    "MQTT service released mqtt clients"
                                );
                            }
                            ServiceMessage::MqttClientStatus(payload) => {
                                let status = match payload.status {
                                    WireMqttClientConnectionStatus::Online => {
                                        ApiMqttClientConnectionStatus::Online
                                    }
                                    WireMqttClientConnectionStatus::Offline => {
                                        ApiMqttClientConnectionStatus::Offline
                                    }
                                    WireMqttClientConnectionStatus::Connecting => {
                                        ApiMqttClientConnectionStatus::Connecting
                                    }
                                };

                                if let Err(e) = mqtt_client_store::update_mqtt_client_status(
                                    &state.db,
                                    payload.mqtt_client_id,
                                    status,
                                )
                                .await
                                {
                                    tracing::warn!(error = %e, "failed to update mqtt client status");
                                }
                            }
                            ServiceMessage::RenewCertificate(payload) => {
                                // Re-fetch service from DB, verify still approved
                                let service = match mqtt_service::Entity::find_by_id(service_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(s)) if s.status == mqtt_service::ServiceStatus::Approved && s.deactivated_at.is_none() => s,
                                    _ => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::Forbidden,
                                            message: "service is not approved".to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
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
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }

                                        if let Err(e) = revoke_mqtt_service_certificate(
                                            &state.db,
                                            &cert.serial,
                                            &cert.ca_fingerprint,
                                            mqtt_service_certificate::RevocationReason::CertificateRenewed,
                                        ).await {
                                            tracing::error!(error = %e, "failed to revoke old certificate");
                                        }

                                        if let Err(e) = crate::settings_store::bump_revocation_version(&state.db, state.default_tenant_id).await {
                                            tracing::warn!(error = ?e, "failed to bump revocation version counter");
                                        }
                                        state.revocation_notify.notify_one();
                                        tracing::info!(%service_id, old_serial = %cert.serial, "MQTT service certificate renewed, old cert revoked");
                                        let _ = close_with_reason(sink, "certificate rotated").await;
                                        break;
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::CertificateError,
                                            message: e.to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                }
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "MQTT service disconnecting gracefully"
                                );
                                break;
                            }
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "unexpected message for authenticated connection".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
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
                let Some(json) = serialize_controller_msg(out_seq, msg) else { break };
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            _ = cancel_token.cancelled() => {
                tracing::info!(%service_id, "MQTT connection superseded by new registration");
                let _ = close_with_reason(sink, "superseded by new connection").await;
                // Do NOT unregister — the new connection owns the registry entry.
                // Still release leases since the new connection will re-reconcile.
                if let Err(e) = lease_coordinator.release_all_for_service(&service_id).await {
                    tracing::error!(error = %e, "failed to release leases on superseded disconnect");
                }
                return;
            }
        }
    }

    // Release all leases on disconnect.
    if let Err(e) = lease_coordinator.release_all_for_service(&service_id).await {
        tracing::error!(error = %e, "failed to release leases on disconnect");
    }

    state.service_connections.unregister(&service_id).await;
    tracing::debug!(%service_id, "authenticated MQTT service disconnected");
}

// ---------------------------------------------------------------------------
// Enrolled MQTT handler
// ---------------------------------------------------------------------------

/// Interval between approval-status DB polls in enrolled loops.
const APPROVAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Service-type-specific enrolled handler for an MQTT connection.
///
/// Handles Ping, RequestCertificate, and polls for approval changes at a
/// fixed interval (decoupled from client-controlled ping frequency).
pub(crate) async fn handle_mqtt_enrolled(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    mut approved: bool,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
) {
    let mut approval_poll = tokio::time::interval(APPROVAL_POLL_INTERVAL);
    approval_poll.tick().await; // skip immediate first tick

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
                        let service_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match service_msg {
                            ServiceMessage::Ping(PingPayload { service_ts }) => {
                                let Ok(controller_ts) = send_pong(sink, out_seq, service_ts).await else {
                                    break;
                                };
                                tracing::trace!(service_ts, controller_ts, "ping/pong (enrolled)");
                            }
                            ServiceMessage::RequestCertificate(payload) => {
                                if !approved {
                                    let err = ControllerMessage::Error(ErrorPayload {
                                        code: ErrorCode::NotApproved,
                                        message: "service is not yet approved".to_string(),
                                    });
                                    if let Some(json) = serialize_controller_msg(out_seq, err) {
                                        let _ = sink.send(Message::Text(json.into())).await;
                                    }
                                    continue;
                                }

                                // Re-fetch service from DB.
                                let service = match mqtt_service::Entity::find_by_id(service_id)
                                    .one(&state.db)
                                    .await
                                {
                                    Ok(Some(s)) => s,
                                    _ => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::InternalError,
                                            message: "service not found".to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
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
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        tracing::info!(%service_id, "MQTT service certificate issued via WS");
                                        break; // close connection after certificate issuance
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: ErrorCode::CertificateError,
                                            message: e.to_string(),
                                        });
                                        if let Some(json) = serialize_controller_msg(out_seq, err) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }
                                        break;
                                    }
                                }
                            }
                            ServiceMessage::Enroll(_) => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "already enrolled".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "MQTT service disconnecting gracefully during enrollment"
                                );
                                break;
                            }
                            _ => {
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: ErrorCode::BadRequest,
                                    message: "not available during enrollment".to_string(),
                                });
                                if let Some(json) = serialize_controller_msg(out_seq, err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            // Dedicated approval poll at a fixed interval, decoupled from
            // client-controlled ping frequency.
            _ = approval_poll.tick(), if !approved => {
                if let Ok(Some(s)) = mqtt_service::Entity::find_by_id(service_id)
                    .one(&state.db)
                    .await
                {
                    match s.status {
                        mqtt_service::ServiceStatus::Approved => {
                            approved = true;
                            let msg =
                                ControllerMessage::Approved(ApprovedPayload { service_id });
                            if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                        }
                        mqtt_service::ServiceStatus::Rejected => {
                            let msg =
                                ControllerMessage::Rejected(RejectedPayload { service_id });
                            if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    tracing::debug!(%service_id, "enrolled MQTT service disconnected");
}

// ---------------------------------------------------------------------------
// MQTT enrollment helper (exposed for service_ws)
// ---------------------------------------------------------------------------

pub(crate) struct MqttServiceEnrollResult {
    pub service: mqtt_service::Model,
    pub enrollment_secret: String,
    pub status: mqtt_service::ServiceStatus,
}

/// Enroll a new MQTT service.
pub(crate) async fn do_mqtt_service_enroll(
    db: &sea_orm::DatabaseConnection,
    settings: &crate::settings::Settings,
    tenant_id: uuid::Uuid,
    hostname: &str,
    friendly_name: &str,
    enrollment_token: Option<&str>,
) -> MqttWsResult<MqttServiceEnrollResult> {
    if hostname.trim().is_empty() {
        return Err(report!(MqttWsError::Enrollment(
            "hostname must not be empty".into()
        )));
    }

    let service_id = uuid::Uuid::now_v7();

    let status = if let Some(token) = enrollment_token {
        let token_hash = match crate::settings_store::load_setting(
            db,
            tenant_id,
            crate::SettingKey::MqttEnrollmentTokenHash,
        )
        .await
        {
            Ok(Some(v)) => match v.as_str() {
                Some(hash) => hash.to_string(),
                None => {
                    return Err(report!(MqttWsError::Enrollment(
                        "no MQTT enrollment token configured".into()
                    )));
                }
            },
            Ok(None) => {
                return Err(report!(MqttWsError::Enrollment(
                    "no MQTT enrollment token configured".into()
                )));
            }
            Err(e) => {
                return Err(report!(MqttWsError::Enrollment(format!(
                    "database error: {e:?}"
                ))));
            }
        };

        match crate::auth::password::verify_password(token, &token_hash) {
            Ok(true) => mqtt_service::ServiceStatus::Approved,
            Ok(false) => {
                return Err(report!(MqttWsError::Enrollment(
                    "invalid enrollment token".into()
                )));
            }
            Err(e) => {
                return Err(report!(MqttWsError::Enrollment(format!(
                    "token verification error: {e}"
                ))));
            }
        }
    } else {
        mqtt_service::ServiceStatus::Pending
    };

    let _ = settings;

    let enrollment_secret = crate::auth::token::generate_secure_token().map_err(|e| {
        report!(MqttWsError::Enrollment(format!(
            "failed to generate token: {e}"
        )))
    })?;
    let secret_hash = crate::auth::token::hash_token(&enrollment_secret);

    let now = time::OffsetDateTime::now_utc();

    let service = mqtt_service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        service_type: Set(mqtt_service::ServiceType::Mqtt),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(friendly_name.to_string()),
        ip_address: Set(None),
        status: Set(status),
        enrollment_secret_hash: Set(secret_hash),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let service = service.insert(db).await.context_to::<MqttWsError>()?;

    Ok(MqttServiceEnrollResult {
        service,
        enrollment_secret,
        status,
    })
}

// ---------------------------------------------------------------------------
// MQTT certificate helpers
// ---------------------------------------------------------------------------

/// Sign a CSR for an MQTT service.
async fn do_sign_mqtt_service_csr(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    service: mqtt_service::Model,
    csr_pem: &str,
) -> MqttWsResult<crate::cert_signer::SignedCertBundle> {
    let validity_days = settings.agent_cert_lifetime_days();
    let validity = time::Duration::days(validity_days as i64);

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &service.id, validity)
        .await
        .map_err(|e| report!(MqttWsError::Certificate(format!("failed to sign CSR: {e}"))))?;

    record_mqtt_service_certificate(db, service.id, &bundle.cert_pem, &ca_fp).await?;

    let mut active: mqtt_service::ActiveModel = service.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.updated_at = Set(time::OffsetDateTime::now_utc());
    let _ = active.update(db).await;

    Ok(bundle)
}

/// Record a certificate in the service_certificates table.
async fn record_mqtt_service_certificate(
    db: &sea_orm::DatabaseConnection,
    mqtt_service_id: uuid::Uuid,
    cert_pem: &str,
    ca_fingerprint: &str,
) -> MqttWsResult<()> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(MqttWsError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(MqttWsError::X509Parse))?;

    let serial = cert.raw_serial_as_string();
    let validity = cert.validity();
    let not_before = time::OffsetDateTime::from_unix_timestamp(validity.not_before.timestamp())
        .map_err(|e| report!(MqttWsError::Timestamp(format!("not_before: {e}"))))?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(validity.not_after.timestamp())
        .map_err(|e| report!(MqttWsError::Timestamp(format!("not_after: {e}"))))?;

    let record = mqtt_service_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fingerprint.to_string()),
        serial_number: Set(serial),
        service_id: Set(mqtt_service_id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(time::OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };

    record.insert(db).await.context_to::<MqttWsError>()?;

    Ok(())
}

/// Revoke an MQTT service certificate.
async fn revoke_mqtt_service_certificate(
    db: &sea_orm::DatabaseConnection,
    serial: &str,
    ca_fingerprint: &str,
    reason: mqtt_service_certificate::RevocationReason,
) -> MqttWsResult<()> {
    let cert = mqtt_service_certificate::Entity::find()
        .filter(mqtt_service_certificate::Column::SerialNumber.eq(serial))
        .filter(mqtt_service_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .one(db)
        .await
        .context_to::<MqttWsError>()?
        .ok_or_else(|| report!(MqttWsError::CertNotFound))?;

    let mut active: mqtt_service_certificate::ActiveModel = cert.into();
    active.revoked_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.revocation_reason = Set(Some(reason));
    active.update(db).await.context_to::<MqttWsError>()?;

    Ok(())
}
