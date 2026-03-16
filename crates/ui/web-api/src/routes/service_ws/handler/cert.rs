//! Certificate request and approval-polling handlers for the enrolled loop.
//!
//! Contains [`handle_request_certificate`], [`CertificateResult`],
//! [`poll_approval_status`], [`ApprovalPollResult`], and the internal
//! [`sign_service_csr`] helper that deduplicates the system / regular service
//! CSR-signing branches.

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;
use sea_orm::EntityTrait;

use uptrakit_internal_wire::{
    ApprovedPayload, CertificatePayload, ControllerMessage, ErrorCode, ErrorPayload, OutgoingSeq,
    RejectedPayload, RequestCertificatePayload,
};
use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};

use crate::AppState;
use crate::cert_signer::{AgentCertSigner, SignedCertBundle};
use crate::routes::agent_operations::{do_sign_csr, do_sign_csr_for_system_service};
use crate::routes::service_ws::protocol::serialize_controller_msg;
use crate::settings::Settings;

use std::sync::Arc;

// ---------------------------------------------------------------------------
// CertificateResult
// ---------------------------------------------------------------------------

/// Result of processing a `RequestCertificate` message.
pub(super) enum CertificateResult {
    /// Certificate issued (or error sent); break out of the main loop.
    Break,
    /// Service not yet approved; already sent error reply, continue looping.
    NotApproved,
}

// ---------------------------------------------------------------------------
// ApprovalPollResult
// ---------------------------------------------------------------------------

/// Result of polling the DB for service approval status changes.
pub(super) enum ApprovalPollResult {
    /// Service has been approved; notification sent.
    Approved,
    /// Service has been rejected; notification sent, caller should break.
    Rejected,
    /// No status change.
    Unchanged,
}

// ---------------------------------------------------------------------------
// sign_service_csr — shared CSR-signing helper
// ---------------------------------------------------------------------------

/// Sign a CSR for either a system service or a regular service.
///
/// Calls `do_sign_csr_for_system_service` or `do_sign_csr` as appropriate and
/// returns the resulting [`SignedCertBundle`], or an error string suitable for
/// inclusion in an [`ErrorPayload`].
async fn sign_service_csr(
    cert_signer: &dyn AgentCertSigner,
    settings: &Settings,
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    is_system: bool,
    csr_pem: &str,
) -> Result<SignedCertBundle, String> {
    if is_system {
        let sys_svc = sys_svc_entity::Entity::find_by_id(service_id)
            .one(db)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| "system service not found".to_string())?;
        do_sign_csr_for_system_service(cert_signer, settings, db, sys_svc, csr_pem)
            .await
            .map_err(|e| e.current_context().to_string())
    } else {
        let svc = service::Entity::find_by_id(service_id)
            .one(db)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| "service not found".to_string())?;
        do_sign_csr(cert_signer, settings, db, svc, csr_pem)
            .await
            .map_err(|e| e.current_context().to_string())
    }
}

// ---------------------------------------------------------------------------
// handle_request_certificate
// ---------------------------------------------------------------------------

/// Handle a `RequestCertificate` message during the enrolled loop.
///
/// Signs the CSR for either a system service or a regular service, sends the
/// certificate (or error) back over the WebSocket, and returns whether the
/// loop should break or continue.
pub(super) async fn handle_request_certificate(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    approved: bool,
    out_seq: &mut OutgoingSeq,
    payload: &RequestCertificatePayload,
) -> CertificateResult {
    if !approved {
        let err = ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::NotApproved,
            message: "service is not yet approved".to_string(),
        });
        if let Some(json) = serialize_controller_msg(out_seq, err) {
            let _ = sink.send(Message::Text(json.into())).await;
        }
        return CertificateResult::NotApproved;
    }

    match sign_service_csr(
        state.cert_signer.as_ref(),
        &state.settings,
        state.db(),
        service_id,
        is_system,
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
            if is_system {
                tracing::info!(%service_id, "system service certificate issued via WS");
            } else {
                tracing::info!(%service_id, "certificate issued via WS");
            }
        }
        Err(message) => {
            // Distinguish "not found" (DB miss) from signing failure for the
            // log level: a missing row is unusual but not a signing error.
            let code = if message.contains("not found") {
                ErrorCode::InternalError
            } else {
                ErrorCode::CertificateError
            };
            let err = ControllerMessage::Error(ErrorPayload { code, message });
            if let Some(json) = serialize_controller_msg(out_seq, err) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
        }
    }

    CertificateResult::Break
}

// ---------------------------------------------------------------------------
// poll_approval_status
// ---------------------------------------------------------------------------

/// Poll the database for approval status changes and send the appropriate
/// WebSocket notification.
pub(super) async fn poll_approval_status(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
    out_seq: &mut OutgoingSeq,
) -> ApprovalPollResult {
    if is_system {
        if let Ok(Some(s)) = sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            return match s.status {
                sys_svc_entity::SystemServiceStatus::Approved => {
                    let msg = ControllerMessage::Approved(ApprovedPayload { service_id });
                    if let Some(json) = serialize_controller_msg(out_seq, msg) {
                        let _ = sink.send(Message::Text(json.into())).await;
                    }
                    ApprovalPollResult::Approved
                }
                sys_svc_entity::SystemServiceStatus::Rejected => {
                    let msg = ControllerMessage::Rejected(RejectedPayload { service_id });
                    if let Some(json) = serialize_controller_msg(out_seq, msg) {
                        let _ = sink.send(Message::Text(json.into())).await;
                    }
                    ApprovalPollResult::Rejected
                }
                _ => ApprovalPollResult::Unchanged,
            };
        }
    } else if let Ok(Some(s)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        return match s.status {
            service::ServiceStatus::Approved => {
                let msg = ControllerMessage::Approved(ApprovedPayload { service_id });
                if let Some(json) = serialize_controller_msg(out_seq, msg) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                ApprovalPollResult::Approved
            }
            service::ServiceStatus::Rejected => {
                let msg = ControllerMessage::Rejected(RejectedPayload { service_id });
                if let Some(json) = serialize_controller_msg(out_seq, msg) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                ApprovalPollResult::Rejected
            }
            _ => ApprovalPollResult::Unchanged,
        };
    }
    ApprovalPollResult::Unchanged
}
