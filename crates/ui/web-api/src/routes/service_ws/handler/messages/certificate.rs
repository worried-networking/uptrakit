use std::sync::Arc;

use sea_orm::EntityTrait;

use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_wire::{
    CertificatePayload, CloseReason, ControllerMessage, ErrorCode, ErrorPayload,
    RequestCrlRenewalPayload,
};

use super::{
    ProcessorResponse, emit_service_certificate_renew_audit_event, ingest_service_audit_event,
    sign_renewal_csr, sign_renewal_csr_system,
};
use crate::AppState;
use crate::routes::agent_operations::{revoke_certificate, revoke_system_certificate};
use crate::routes::service_ws::protocol::CertIdentity;

pub(super) async fn emit_service_certificate_renew_non_success_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: &'static str,
) {
    let payload = uptrakit_wire::AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: None,
        outcome: outcome.as_str().to_string(),
        details_json: Some(
            serde_json::json!({
                "reason_code": reason_code,
            })
            .to_string(),
        ),
        request_id: None,
        correlation_id: None,
    };
    let _ = ingest_service_audit_event(state, service_id, is_system, None, None, payload).await;
}

/// Handle a `RenewCertificate` message: verify approved, sign renewal CSR,
/// revoke old cert.
///
/// Returns a [`ProcessorResponse`] with the reply message and action.
#[tracing::instrument(skip_all, fields(%service_id, is_system))]
pub(in super::super) async fn handle_renew_certificate(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    cert: &CertIdentity,
    payload: &uptrakit_wire::RenewCertificatePayload,
    is_system: bool,
) -> ProcessorResponse {
    if is_system {
        // System service renewal path.
        let svc = match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(s))
                if s.status == sys_svc_entity::SystemServiceStatus::Approved
                    && s.deactivated_at.is_none() =>
            {
                s
            }
            _ => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    true,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "not_approved",
                )
                .await;
                return ProcessorResponse::reply_and_break(ControllerMessage::Error(
                    ErrorPayload {
                        code: ErrorCode::Forbidden,
                        message: "service is not approved".to_string(),
                    },
                ));
            }
        };

        match sign_renewal_csr_system(
            state.cert_signer.as_ref(),
            &state.settings,
            state.db(),
            svc,
            &payload.csr_pem,
        )
        .await
        {
            Ok(bundle) => {
                // Revoke old system service certificate.
                if let Err(e) =
                    revoke_system_certificate(state.db(), &cert.serial, &cert.ca_fingerprint).await
                {
                    tracing::error!(error = %e, "failed to revoke old system service certificate");
                }

                // Bump CRL and notify (system certs share the CRL).
                if let Err(e) = crate::settings_store::bump_revocation_version(
                    state.db(),
                    state.default_tenant_id,
                )
                .await
                {
                    tracing::warn!(error = ?e, "failed to bump revocation version counter");
                }
                state.cert.revocation_notify.notify_one();
                state
                    .notification
                    .notification_service
                    .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                        RequestCrlRenewalPayload::default(),
                    ))
                    .await;
                tracing::info!(
                    %service_id,
                    old_serial = %cert.serial,
                    "system service certificate renewed, old cert revoked"
                );

                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                emit_service_certificate_renew_audit_event(
                    state,
                    service_id,
                    true,
                    bundle.not_after.into(),
                )
                .await;
                ProcessorResponse::reply_and_close(cert_msg, CloseReason::CertificateRotated)
            }
            Err(e) => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    true,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    "certificate_signing_failed",
                )
                .await;
                ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.to_string(),
                }))
            }
        }
    } else {
        // Tenant service renewal path.
        let svc = match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(s))
                if s.status == service::ServiceStatus::Approved && s.deactivated_at.is_none() =>
            {
                s
            }
            _ => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    false,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "not_approved",
                )
                .await;
                return ProcessorResponse::reply_and_break(ControllerMessage::Error(
                    ErrorPayload {
                        code: ErrorCode::Forbidden,
                        message: "service is not approved".to_string(),
                    },
                ));
            }
        };

        match sign_renewal_csr(
            state.cert_signer.as_ref(),
            &state.settings,
            state.db(),
            svc,
            &payload.csr_pem,
        )
        .await
        {
            Ok(bundle) => {
                // Revoke old certificate.
                if let Err(e) = revoke_certificate(
                    state.db(),
                    &cert.serial,
                    &cert.ca_fingerprint,
                    uptrakit_shared_db::entity::prelude::RevocationReason::CertificateRenewed,
                )
                .await
                {
                    tracing::error!(error = %e, "failed to revoke old certificate");
                }

                if let Err(e) = crate::settings_store::bump_revocation_version(
                    state.db(),
                    state.default_tenant_id,
                )
                .await
                {
                    tracing::warn!(error = ?e, "failed to bump revocation version counter");
                }
                state.cert.revocation_notify.notify_one();
                state
                    .notification
                    .notification_service
                    .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                        RequestCrlRenewalPayload::default(),
                    ))
                    .await;
                tracing::info!(
                    %service_id,
                    old_serial = %cert.serial,
                    "certificate renewed, old cert revoked"
                );

                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                emit_service_certificate_renew_audit_event(
                    state,
                    service_id,
                    false,
                    bundle.not_after.into(),
                )
                .await;
                ProcessorResponse::reply_and_close(cert_msg, CloseReason::CertificateRotated)
            }
            Err(e) => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    false,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    "certificate_signing_failed",
                )
                .await;
                ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.to_string(),
                }))
            }
        }
    }
}
