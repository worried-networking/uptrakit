use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uptrakit_internal_wire::{
    ApprovedPayload, CertificatePayload, ControllerMessage, ErrorPayload, MqttRegisteredPayload,
    MqttTenantAssignmentsPayload, PingPayload, PongPayload, RejectedPayload, ServiceMessage,
    now_millis,
};
use uptrakit_shared_db::entity::{
    service as mqtt_service, service_certificate as mqtt_service_certificate,
};

use rootcause::prelude::*;
use thiserror::Error;

use super::service_ws::{close_with_reason, serialize_msg};
use crate::AppState;
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
    service_id: uuid::Uuid,
    cert_serial: String,
    cert_ca_fingerprint: String,
) {
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
                let service_msg: ServiceMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "deserialize error");
                        return;
                    }
                };

                match service_msg {
                    ServiceMessage::Register(payload) => {
                        break (
                            payload.instance_id,
                            payload.max_tenants,
                            payload.active_mqtt_clients,
                        );
                    }
                    ServiceMessage::Ping(PingPayload { agent_ts }) => {
                        let controller_ts = now_millis();
                        let response = ControllerMessage::Pong(PongPayload {
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
                        let err = ControllerMessage::Error(ErrorPayload {
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

    // Register in connection registry.
    let mut push_rx = state
        .service_connections
        .register_mqtt(service_id, instance_id.clone(), max_tenants)
        .await;

    // Send Registered acknowledgment.
    let registered_msg = ControllerMessage::Registered(MqttRegisteredPayload {
        instance_id: instance_id.clone(),
    });
    let Some(json) = serialize_msg(&registered_msg) else {
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
        let mqtt_client_ids: Vec<uuid::Uuid> = active_mqtt_clients
            .iter()
            .filter_map(|s| uuid::Uuid::parse_str(s).ok())
            .collect();

        match lease_coordinator
            .reconcile_mqtt_clients(service_id, &instance_id, &mqtt_client_ids)
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
        let Some(json) = serialize_msg(&assignments_msg) else {
            state.service_connections.unregister(&service_id).await;
            return;
        };
        if sink.send(Message::Text(json.into())).await.is_err() {
            state.service_connections.unregister(&service_id).await;
            return;
        }
    }

    tracing::info!(%service_id, instance_id = %instance_id, "MQTT service registered");

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
                        let service_msg: ServiceMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match service_msg {
                            ServiceMessage::Ping(PingPayload { agent_ts }) => {
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

                                // Update lease heartbeats for all tenants held by this service
                                if let Err(e) = lease_coordinator
                                    .record_heartbeat(&service_id)
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to record heartbeat");
                                }
                            }
                            ServiceMessage::ReleaseTenants(payload) => {
                                let mqtt_client_ids: Vec<uuid::Uuid> = payload
                                    .mqtt_client_ids
                                    .iter()
                                    .filter_map(|s| uuid::Uuid::parse_str(s).ok())
                                    .collect();

                                if let Err(e) = lease_coordinator
                                    .release_mqtt_clients(&service_id, &mqtt_client_ids)
                                    .await
                                {
                                    tracing::warn!(error = %e, "failed to release mqtt clients");
                                }

                                tracing::info!(
                                    %service_id,
                                    count = mqtt_client_ids.len(),
                                    "MQTT service released mqtt clients"
                                );
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
                                            code: "forbidden".to_string(),
                                            message: "service is not approved".to_string(),
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
                                ).await {
                                    Ok(bundle) => {
                                        let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                                            cert_pem: bundle.cert_pem,
                                            not_after: bundle.not_after,
                                        });
                                        if let Some(json) = serialize_msg(&cert_msg) {
                                            let _ = sink.send(Message::Text(json.into())).await;
                                        }

                                        if let Err(e) = revoke_mqtt_service_certificate(
                                            &state.db,
                                            &cert_serial,
                                            &cert_ca_fingerprint,
                                            mqtt_service_certificate::RevocationReason::CertificateRenewed,
                                        ).await {
                                            tracing::error!(error = %e, "failed to revoke old certificate");
                                        }

                                        state.revocation_notify.notify_one();
                                        tracing::info!(%service_id, old_serial = %cert_serial, "MQTT service certificate renewed, old cert revoked");
                                        let _ = close_with_reason(sink, "certificate rotated").await;
                                        break;
                                    }
                                    Err(e) => {
                                        let err = ControllerMessage::Error(ErrorPayload {
                                            code: "certificate_error".to_string(),
                                            message: e.to_string(),
                                        });
                                        if let Some(json) = serialize_msg(&err) {
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

/// Service-type-specific enrolled handler for an MQTT connection.
///
/// Handles Ping, RequestCertificate, and polls for approval changes.
pub(crate) async fn handle_mqtt_enrolled(
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
                let service_msg: ServiceMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "deserialize error");
                        break;
                    }
                };

                match service_msg {
                    ServiceMessage::Ping(PingPayload { agent_ts }) => {
                        let controller_ts = now_millis();
                        let response = ControllerMessage::Pong(PongPayload {
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

                        // Poll database for status change (simplified).
                        if !approved
                            && let Ok(Some(s)) = mqtt_service::Entity::find_by_id(service_id)
                                .one(&state.db)
                                .await
                        {
                            match s.status {
                                mqtt_service::ServiceStatus::Approved => {
                                    approved = true;
                                    let msg = ControllerMessage::Approved(ApprovedPayload {
                                        service_id: service_id.to_string(),
                                    });
                                    if let Some(json) = serialize_msg(&msg) {
                                        let _ = sink.send(Message::Text(json.into())).await;
                                    }
                                }
                                mqtt_service::ServiceStatus::Rejected => {
                                    let msg = ControllerMessage::Rejected(RejectedPayload {
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
                    ServiceMessage::RequestCertificate(payload) => {
                        if !approved {
                            let err = ControllerMessage::Error(ErrorPayload {
                                code: "not_approved".to_string(),
                                message: "service is not yet approved".to_string(),
                            });
                            if let Some(json) = serialize_msg(&err) {
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
                                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
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
                                let err = ControllerMessage::Error(ErrorPayload {
                                    code: "certificate_error".to_string(),
                                    message: e.to_string(),
                                });
                                if let Some(json) = serialize_msg(&err) {
                                    let _ = sink.send(Message::Text(json.into())).await;
                                }
                                break;
                            }
                        }
                    }
                    ServiceMessage::Enroll(_) => {
                        let err = ControllerMessage::Error(ErrorPayload {
                            code: "bad_request".to_string(),
                            message: "already enrolled".to_string(),
                        });
                        if let Some(json) = serialize_msg(&err) {
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

    let service = service
        .insert(db)
        .await
        .map_err(|e| report!(MqttWsError::Database(e)))?;

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
    let validity_days = settings.agent_cert_lifetime_days().await;
    let validity = time::Duration::days(validity_days as i64);

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &service.id, validity)
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

    record
        .insert(db)
        .await
        .map_err(|e| report!(MqttWsError::Database(e)))?;

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
        .map_err(|e| report!(MqttWsError::Database(e)))?
        .ok_or_else(|| report!(MqttWsError::CertNotFound))?;

    let mut active: mqtt_service_certificate::ActiveModel = cert.into();
    active.revoked_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.revocation_reason = Set(Some(reason));
    active
        .update(db)
        .await
        .map_err(|e| report!(MqttWsError::Database(e)))?;

    Ok(())
}
