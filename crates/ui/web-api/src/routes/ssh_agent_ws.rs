use std::collections::{BTreeSet, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uptrakit_internal_wire::{
    ApprovedPayload, Capability, CertificatePayload, CloseReason, ControllerMessage, ErrorCode,
    ErrorPayload, IncomingSeq, OutgoingSeq, PingPayload, RejectedPayload, ServiceMessage,
};
use uptrakit_shared_db::entity::{
    service as ssh_agent_service, service_certificate as ssh_agent_service_certificate,
    service_host as ssh_agent_host,
};

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

use super::service_ws::{
    MessageRateLimiter, WS_MESSAGE_RATE_LIMIT, WS_MESSAGE_RATE_WINDOW, close_with_reason,
    deserialize_service_msg, record_service_activity, send_pong, serialize_controller_msg,
};
use crate::AppState;
use crate::routes::agent_ws::trigger_discovery_for_agent_host;
use crate::routes::agents::find_or_create_host_and_link;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub(crate) enum SshAgentWsError {
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

type SshAgentWsResult<T> = std::result::Result<T, Report<SshAgentWsError>>;

impl_report_conversion!(sea_orm::DbErr => SshAgentWsError::Database);

// ---------------------------------------------------------------------------
// Authenticated SSH agent handler (called from service_ws after shared auth)
// ---------------------------------------------------------------------------

/// Service-type-specific handler for an authenticated SSH agent connection.
///
/// Called by [`super::service_ws`] after certificate validation, service status
/// check, and sending `ServiceSettings`. Enters a Ping/Pong keepalive loop
/// with certificate renewal support.
pub(crate) async fn handle_ssh_agent_authenticated(
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

    // Load service to derive capabilities for registration.
    let capabilities: BTreeSet<Capability> = match ssh_agent_service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => crate::service_profile::parse_capabilities(&svc.capabilities),
        _ => BTreeSet::new(),
    };

    // Register in connection registry.
    let (mut push_rx, cancel_token) = state
        .service_connections
        .register(service_id, capabilities.clone(), None, None)
        .await;

    let delivered = state
        .notification_service
        .deliver_backlog_for_authenticated_service(
            service_id,
            &capabilities,
            last_seen_at,
        )
        .await;
    if delivered > 0 {
        tracing::info!(
            %service_id,
            delivered,
            "delivered outbox backlog to SSH agent"
        );
    }

    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

    // Cache host IDs linked to this SSH agent for future ownership validation.
    // Refreshed on ReportHosts (which may link new hosts).
    let mut linked_host_ids: HashSet<uuid::Uuid> =
        load_ssh_agent_linked_host_ids(state.db(), service_id)
            .await
            .unwrap_or_default();

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
                if !rate_limiter.allow() {
                    let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                    break;
                }
                match msg {
                    Message::Text(text) => {
                        let service_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                            Ok(Some(m)) => m,
                            Ok(None) => continue,
                            Err(e) => {
                                tracing::debug!(error = %e, "deserialize error");
                                break;
                            }
                        };

                        match service_msg {
                            ServiceMessage::Ping(PingPayload { service_ts }) => {
                                let Ok(controller_ts) = send_pong(sink, out_seq, service_ts).await else { break };
                                tracing::trace!(service_ts, controller_ts, "ping/pong");
                                if let Err(e) = record_service_activity(state.db(), service_id, None).await {
                                    tracing::warn!(error = %e, %service_id, "failed to record service activity");
                                }
                            }
                            ServiceMessage::RenewCertificate(payload) => {
                                // Re-fetch service from DB, verify still approved
                                let service = match ssh_agent_service::Entity::find_by_id(service_id)
                                    .one(state.db())
                                    .await
                                {
                                    Ok(Some(s)) if s.status == ssh_agent_service::ServiceStatus::Approved && s.deactivated_at.is_none() => s,
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

                                match do_sign_ssh_agent_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    state.db(),
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

                                        if let Err(e) = revoke_ssh_agent_certificate(
                                            state.db(),
                                            &cert.serial,
                                            &cert.ca_fingerprint,
                                            ssh_agent_service_certificate::RevocationReason::CertificateRenewed,
                                        ).await {
                                            tracing::error!(error = %e, "failed to revoke old certificate");
                                        }

                                        if let Err(e) = crate::settings_store::bump_revocation_version(state.db(), state.default_tenant_id).await {
                                            tracing::warn!(error = ?e, "failed to bump revocation version counter");
                                        }
                                        state.revocation_notify.notify_one();
                                        tracing::info!(%service_id, old_serial = %cert.serial, "SSH agent certificate renewed, old cert revoked");
                                        let _ = close_with_reason(sink, CloseReason::CertificateRotated).await;
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
                            ServiceMessage::ReportHosts(payload) => {
                                tracing::debug!(
                                    %service_id,
                                    capabilities = ?payload.capabilities,
                                    "received ReportHosts"
                                );

                                // Look up SSH agent service from DB.
                                let service_model = match ssh_agent_service::Entity::find_by_id(service_id)
                                    .one(state.db())
                                    .await
                                {
                                    Ok(Some(s)) => s,
                                    _ => continue,
                                };

                                // Update client_version in database.
                                let mut active: ssh_agent_service::ActiveModel = service_model.clone().into();
                                active.client_version = Set(Some(payload.agent_version.clone()));
                                active.updated_at = Set(time::OffsetDateTime::now_utc());
                                if let Err(e) = active.update(state.db()).await {
                                    tracing::error!(error = %e, "failed to update SSH agent client_version");
                                }

                                for host_info in &payload.hosts {
                                    let host_hostname = host_info.hostname.as_deref().unwrap_or(&service_model.hostname);
                                    let host_ip = host_info.ip_address.as_deref().or(service_model.ip_address.as_deref());
                                    match find_or_create_host_and_link(
                                        state.db(),
                                        service_model.tenant_id,
                                        service_id,
                                        host_info,
                                        host_hostname,
                                        host_ip,
                                    ).await {
                                        Ok(Some((_host_id, true))) => {
                                            // New host registered — trigger autodiscovery.
                                            trigger_discovery_for_agent_host(
                                                state,
                                                service_id,
                                                service_model.tenant_id,
                                                &host_info.machine_id,
                                            )
                                            .await;
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            tracing::warn!(error = %e, machine_id = %host_info.machine_id, "failed to link host to SSH agent");
                                        }
                                    }
                                }

                                // Refresh cached host IDs (may have linked new hosts).
                                if let Ok(ids) = load_ssh_agent_linked_host_ids(state.db(), service_id).await {
                                    linked_host_ids = ids;
                                }
                            }
                            ServiceMessage::DiscoveryResults(payload) => {
                                tracing::debug!(
                                    %service_id,
                                    host_machine_id = %payload.host_machine_id,
                                    results = payload.results.len(),
                                    "SSH agent received DiscoveryResults"
                                );

                                // Find the host this result targets.
                                let links = uptrakit_shared_db::entity::prelude::ServiceHost::find()
                                    .filter(ssh_agent_host::Column::ServiceId.eq(service_id))
                                    .all(state.db())
                                    .await
                                    .unwrap_or_default();

                                let mut host_id_opt: Option<uuid::Uuid> = None;
                                for link in &links {
                                    if let Ok(Some(h)) = uptrakit_shared_db::entity::host::Entity::find_by_id(link.host_id)
                                        .filter(uptrakit_shared_db::entity::host::Column::MachineId.eq(&payload.host_machine_id))
                                        .filter(uptrakit_shared_db::entity::host::Column::DeactivatedAt.is_null())
                                        .one(state.db())
                                        .await
                                    {
                                        host_id_opt = Some(h.id);
                                        break;
                                    }
                                }

                                if let Some(host_id) = host_id_opt {
                                    if let Ok(Some(svc)) = ssh_agent_service::Entity::find_by_id(service_id)
                                        .one(state.db())
                                        .await
                                        && let Err(e) = crate::queries::autodiscovery::process_discovery_results(
                                            state.db(),
                                            service_id,
                                            svc.tenant_id,
                                            host_id,
                                            payload,
                                        ).await
                                    {
                                        tracing::warn!(
                                            error = %e,
                                            %service_id,
                                            "failed to process SSH agent discovery results"
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        %service_id,
                                        host_machine_id = %payload.host_machine_id,
                                        "received DiscoveryResults for unknown host machine_id"
                                    );
                                }
                            }
                            ServiceMessage::Disconnecting(payload) => {
                                tracing::info!(
                                    %service_id,
                                    reason = ?payload.reason,
                                    "SSH agent disconnecting gracefully"
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
                tracing::info!(%service_id, "SSH agent connection superseded by new registration");
                let _ = close_with_reason(sink, CloseReason::Superseded).await;
                // Do NOT unregister — the new connection owns the registry entry.
                return;
            }
        }
    }

    // linked_host_ids will be used for update ownership validation (future work).
    let _ = &linked_host_ids;

    state.service_connections.unregister(&service_id).await;
    tracing::debug!(%service_id, "authenticated SSH agent disconnected");
}

// ---------------------------------------------------------------------------
// Enrolled SSH agent handler
// ---------------------------------------------------------------------------

/// Interval between approval-status DB polls in enrolled loops.
const APPROVAL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Service-type-specific enrolled handler for an SSH agent connection.
///
/// Handles Ping, RequestCertificate, and polls for approval changes at a
/// fixed interval (decoupled from client-controlled ping frequency).
pub(crate) async fn handle_ssh_agent_enrolled(
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
    let mut rate_limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT);

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
                if !rate_limiter.allow() {
                    let _ = close_with_reason(sink, CloseReason::RateLimitExceeded).await;
                    break;
                }

                match msg {
                    Message::Text(text) => {
                        let service_msg: ServiceMessage = match deserialize_service_msg(in_seq, &text) {
                            Ok(Some(m)) => m,
                            Ok(None) => continue,
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
                                if let Err(e) = record_service_activity(state.db(), service_id, None).await {
                                    tracing::warn!(error = %e, %service_id, "failed to record service activity");
                                }
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
                                let service = match ssh_agent_service::Entity::find_by_id(service_id)
                                    .one(state.db())
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

                                match do_sign_ssh_agent_csr(
                                    state.cert_signer.as_ref(),
                                    &state.settings,
                                    state.db(),
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
                                        tracing::info!(%service_id, "SSH agent certificate issued via WS");
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
                                    "SSH agent disconnecting gracefully during enrollment"
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
                if let Ok(Some(s)) = ssh_agent_service::Entity::find_by_id(service_id)
                    .one(state.db())
                    .await
                {
                    match s.status {
                        ssh_agent_service::ServiceStatus::Approved => {
                            approved = true;
                            let msg =
                                ControllerMessage::Approved(ApprovedPayload { service_id });
                            if let Some(json) = serialize_controller_msg(out_seq, msg) {
                                let _ = sink.send(Message::Text(json.into())).await;
                            }
                        }
                        ssh_agent_service::ServiceStatus::Rejected => {
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

    tracing::debug!(%service_id, "enrolled SSH agent disconnected");
}

// ---------------------------------------------------------------------------
// SSH agent enrollment helper (exposed for service_ws)
// ---------------------------------------------------------------------------

/// Load the set of host IDs linked to the given SSH agent service.
async fn load_ssh_agent_linked_host_ids(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
) -> SshAgentWsResult<HashSet<uuid::Uuid>> {
    let links = ssh_agent_host::Entity::find()
        .filter(ssh_agent_host::Column::ServiceId.eq(service_id))
        .all(db)
        .await
        .context_to::<SshAgentWsError>()?;

    Ok(links.into_iter().map(|l| l.host_id).collect())
}

pub(crate) struct SshAgentEnrollResult {
    pub service: ssh_agent_service::Model,
    pub enrollment_secret: String,
    pub status: ssh_agent_service::ServiceStatus,
}

/// Enroll a new SSH agent service.
pub(crate) async fn do_ssh_agent_enroll(
    db: &sea_orm::DatabaseConnection,
    settings: &crate::settings::Settings,
    tenant_id: uuid::Uuid,
    hostname: &str,
    friendly_name: &str,
    enrollment_token: Option<&str>,
    ip_address: Option<IpAddr>,
) -> SshAgentWsResult<SshAgentEnrollResult> {
    if hostname.trim().is_empty() {
        bail!(SshAgentWsError::Enrollment(
            "hostname must not be empty".into()
        ));
    }

    let service_id = uuid::Uuid::now_v7();

    let status = if let Some(token) = enrollment_token {
        let token_hash = match crate::settings_store::load_setting(
            db,
            tenant_id,
            crate::SettingKey::EnrollmentTokenHash,
        )
        .await
        {
            Ok(Some(v)) => match v.as_str() {
                Some(hash) => hash.to_string(),
                None => {
                    bail!(SshAgentWsError::Enrollment(
                        "no SSH agent enrollment token configured".into()
                    ));
                }
            },
            Ok(None) => {
                bail!(SshAgentWsError::Enrollment(
                    "no SSH agent enrollment token configured".into()
                ));
            }
            Err(e) => {
                bail!(SshAgentWsError::Enrollment(format!(
                    "database error: {e:?}"
                )));
            }
        };

        match crate::auth::password::verify_password(token, &token_hash) {
            Ok(true) => ssh_agent_service::ServiceStatus::Approved,
            Ok(false) => {
                bail!(SshAgentWsError::Enrollment(
                    "invalid enrollment token".into()
                ));
            }
            Err(e) => {
                bail!(SshAgentWsError::Enrollment(format!(
                    "token verification error: {e}"
                )));
            }
        }
    } else {
        ssh_agent_service::ServiceStatus::Pending
    };

    let _ = settings;

    let enrollment_secret = crate::auth::token::generate_secure_token().map_err(|e| {
        report!(SshAgentWsError::Enrollment(format!(
            "failed to generate token: {e}"
        )))
    })?;
    let secret_hash = crate::auth::token::hash_token(&enrollment_secret);

    let now = time::OffsetDateTime::now_utc();

    let ssh_caps: BTreeSet<Capability> = BTreeSet::from([
        Capability::GracefulShutdown,
        Capability::SoftwareDiscovery,
        Capability::SshRemote,
        Capability::UpdateHooks,
    ]);

    let service = ssh_agent_service::ActiveModel {
        id: Set(service_id),
        tenant_id: Set(tenant_id),
        capabilities: Set(crate::service_profile::serialize_capabilities(&ssh_caps)),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(friendly_name.to_string()),
        ip_address: Set(ip_address.map(|ip| ip.to_string())),
        status: Set(status),
        enrollment_secret_hash: Set(secret_hash),
        client_version: Set(None),
        last_seen_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
    };

    let service = service.insert(db).await.context_to::<SshAgentWsError>()?;

    Ok(SshAgentEnrollResult {
        service,
        enrollment_secret,
        status,
    })
}

// ---------------------------------------------------------------------------
// SSH agent certificate helpers
// ---------------------------------------------------------------------------

/// Sign a CSR for an SSH agent service.
async fn do_sign_ssh_agent_csr(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    service: ssh_agent_service::Model,
    csr_pem: &str,
) -> SshAgentWsResult<crate::cert_signer::SignedCertBundle> {
    let validity_days = settings.agent_cert_lifetime_days();
    let validity = time::Duration::days(validity_days as i64);

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &service.id, validity)
        .await
        .map_err(|e| {
            report!(SshAgentWsError::Certificate(format!(
                "failed to sign CSR: {e}"
            )))
        })?;

    record_ssh_agent_certificate(db, service.id, &bundle.cert_pem, &ca_fp).await?;

    let mut active: ssh_agent_service::ActiveModel = service.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.updated_at = Set(time::OffsetDateTime::now_utc());
    let _ = active.update(db).await;

    Ok(bundle)
}

/// Record a certificate in the service_certificates table.
async fn record_ssh_agent_certificate(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    cert_pem: &str,
    ca_fingerprint: &str,
) -> SshAgentWsResult<()> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(SshAgentWsError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(SshAgentWsError::X509Parse))?;

    let serial = cert.raw_serial_as_string();
    let validity = cert.validity();
    let not_before = time::OffsetDateTime::from_unix_timestamp(validity.not_before.timestamp())
        .map_err(|e| report!(SshAgentWsError::Timestamp(format!("not_before: {e}"))))?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(validity.not_after.timestamp())
        .map_err(|e| report!(SshAgentWsError::Timestamp(format!("not_after: {e}"))))?;

    let record = ssh_agent_service_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fingerprint.to_string()),
        serial_number: Set(serial),
        service_id: Set(service_id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(time::OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };

    record.insert(db).await.context_to::<SshAgentWsError>()?;

    Ok(())
}

/// Revoke an SSH agent service certificate.
async fn revoke_ssh_agent_certificate(
    db: &sea_orm::DatabaseConnection,
    serial: &str,
    ca_fingerprint: &str,
    reason: ssh_agent_service_certificate::RevocationReason,
) -> SshAgentWsResult<()> {
    let cert = ssh_agent_service_certificate::Entity::find()
        .filter(ssh_agent_service_certificate::Column::SerialNumber.eq(serial))
        .filter(ssh_agent_service_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .one(db)
        .await
        .context_to::<SshAgentWsError>()?
        .ok_or_else(|| report!(SshAgentWsError::CertNotFound))?;

    let mut active: ssh_agent_service_certificate::ActiveModel = cert.into();
    active.revoked_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.revocation_reason = Set(Some(reason));
    active.update(db).await.context_to::<SshAgentWsError>()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};

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

    fn test_settings() -> crate::settings::Settings {
        crate::settings::Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            7,
        )
    }

    #[tokio::test]
    async fn ssh_agent_enroll_sets_ip_and_last_seen() {
        let db = setup_test_db().await;
        let settings = test_settings();

        let result = do_ssh_agent_enroll(
            &db,
            &settings,
            uuid::Uuid::now_v7(),
            "ssh-agent-host",
            "ssh-agent-friendly",
            None,
            Some(IpAddr::from([203, 0, 113, 10])),
        )
        .await
        .expect("ssh agent enroll");

        assert_eq!(result.service.ip_address.as_deref(), Some("203.0.113.10"));
        assert!(result.service.last_seen_at.is_some());
        let caps = crate::service_profile::parse_capabilities(&result.service.capabilities);
        assert!(caps.contains(&Capability::SshRemote));
    }

    #[tokio::test]
    async fn ssh_agent_enroll_empty_hostname_fails() {
        let db = setup_test_db().await;
        let settings = test_settings();

        let result = do_ssh_agent_enroll(
            &db,
            &settings,
            uuid::Uuid::now_v7(),
            "",
            "friendly",
            None,
            None,
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ssh_agent_enroll_without_ip() {
        let db = setup_test_db().await;
        let settings = test_settings();

        let result = do_ssh_agent_enroll(
            &db,
            &settings,
            uuid::Uuid::now_v7(),
            "ssh-agent-host",
            "ssh-agent-friendly",
            None,
            None,
        )
        .await
        .expect("ssh agent enroll");

        assert_eq!(result.service.ip_address, None);
        assert!(result.service.last_seen_at.is_some());
    }
}
