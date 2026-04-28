//! Certificate request and approval-polling handlers for the enrolled loop.
//!
//! Contains [`handle_request_certificate`], [`CertificateResult`],
//! [`poll_approval_status`], [`ApprovalPollResult`], and the internal
//! [`sign_service_csr`] helper that deduplicates the system / regular service
//! CSR-signing branches.

use axum::extract::ws::{Message, WebSocket};
use futures_util::{Sink, SinkExt};
use sea_orm::EntityTrait;
use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome};

use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_wire::{
    ApprovedPayload, CertificatePayload, ControllerMessage, ErrorCode, ErrorPayload, OutgoingSeq,
    RejectedPayload, RequestCertificatePayload,
};

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
    handle_request_certificate_with_sink(
        sink, state, service_id, is_system, approved, out_seq, payload,
    )
    .await
}

async fn handle_request_certificate_with_sink<S>(
    sink: &mut S,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    approved: bool,
    out_seq: &mut OutgoingSeq,
    payload: &RequestCertificatePayload,
) -> CertificateResult
where
    S: Sink<Message> + Unpin,
{
    if !approved {
        let err = ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::NotApproved,
            message: "service is not yet approved".to_string(),
        });
        if let Some(json) = serialize_controller_msg(out_seq, err) {
            let _ = sink.send(Message::Text(json.into())).await;
        }
        emit_service_certificate_issue_audit_event(
            state,
            service_id,
            is_system,
            AuditOutcome::Denied,
            "not_approved",
        )
        .await;
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
            crate::routes::service_ws::handler::emit_service_certificate_issue_audit_event(
                state,
                service_id,
                bundle.not_after.into(),
            )
            .await;
            if is_system {
                tracing::info!(%service_id, "system service certificate issued via WS");
            } else {
                tracing::info!(%service_id, "certificate issued via WS");
            }
        }
        Err(message) => {
            let (outcome, reason_code, code) =
                classify_certificate_issue_error(is_system, message.as_str());
            emit_service_certificate_issue_audit_event(
                state,
                service_id,
                is_system,
                outcome,
                reason_code,
            )
            .await;
            // Distinguish "not found" (DB miss) from signing failure for the
            // log level: a missing row is unusual but not a signing error.
            let err = ControllerMessage::Error(ErrorPayload { code, message });
            if let Some(json) = serialize_controller_msg(out_seq, err) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
        }
    }

    CertificateResult::Break
}

fn classify_certificate_issue_error(
    is_system: bool,
    message: &str,
) -> (AuditOutcome, &'static str, ErrorCode) {
    if message.contains("not found") {
        return (
            AuditOutcome::Denied,
            if is_system {
                "system_service_not_found"
            } else {
                "service_not_found"
            },
            ErrorCode::InternalError,
        );
    }

    (
        AuditOutcome::Failed,
        "certificate_signing_failed",
        ErrorCode::CertificateError,
    )
}

async fn emit_service_certificate_issue_audit_event(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    outcome: AuditOutcome,
    reason_code: &'static str,
) {
    let identity = resolve_service_certificate_identity(state, service_id, is_system).await;
    let mut builder = AuditEntry::builder(AuditActionType::SERVICE_CERTIFICATE_ISSUE)
        .actor_service(service_id)
        .actor_display_opt(identity.service_app_name.clone())
        .target(
            "service",
            service_id.to_string(),
            Some(identity.target_display),
        )
        .outcome(outcome)
        .details(serde_json::json!({
            "reason_code": reason_code,
        }));

    builder = if is_system {
        builder.system_scope()
    } else if let Some(tenant_id) = identity.tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        tracing::warn!(
            %service_id,
            outcome = outcome.as_str(),
            reason_code,
            "skipping tenant certificate issue audit event because tenant scope could not be resolved"
        );
        return;
    };

    match builder.build() {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            %service_id,
            outcome = outcome.as_str(),
            reason_code,
            error = %error,
            "failed to build certificate issue audit entry"
        ),
    }
}

struct ServiceCertificateAuditIdentity {
    tenant_id: Option<uuid::Uuid>,
    service_app_name: Option<String>,
    target_display: String,
}

async fn resolve_service_certificate_identity(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
) -> ServiceCertificateAuditIdentity {
    if is_system {
        if let Ok(Some(service)) = sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            return ServiceCertificateAuditIdentity {
                tenant_id: None,
                service_app_name: service.service_app_name.clone(),
                target_display: if !service.friendly_name.is_empty() {
                    service.friendly_name
                } else if !service.hostname.is_empty() {
                    service.hostname
                } else if let Some(service_app_name) =
                    service.service_app_name.filter(|value| !value.is_empty())
                {
                    service_app_name
                } else {
                    service_id.to_string()
                },
            };
        }
    } else if let Ok(Some(service)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        return ServiceCertificateAuditIdentity {
            tenant_id: Some(service.tenant_id),
            service_app_name: service.service_app_name.clone(),
            target_display: if !service.friendly_name.is_empty() {
                service.friendly_name
            } else if !service.hostname.is_empty() {
                service.hostname
            } else if let Some(service_app_name) =
                service.service_app_name.filter(|value| !value.is_empty())
            {
                service_app_name
            } else {
                service_id.to_string()
            },
        };
    }

    ServiceCertificateAuditIdentity {
        tenant_id: None,
        service_app_name: None,
        target_display: service_id.to_string(),
    }
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
                crate::routes::service_ws::handler::emit_service_enrollment_completed_audit_event(
                    state, service_id,
                )
                .await;
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

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;

    use std::pin::Pin;
    use std::task::{Context, Poll};

    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
    use tokio::time::Duration;
    use uptrakit_shared_db::entity::{audit_log, system_audit_log};

    struct TestMessageSink {
        sent_messages: Vec<Message>,
    }

    impl TestMessageSink {
        fn recording() -> Self {
            Self {
                sent_messages: Vec::new(),
            }
        }
    }

    impl Sink<Message> for TestMessageSink {
        type Error = std::io::Error;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(
            mut self: Pin<&mut Self>,
            item: Message,
        ) -> std::result::Result<(), Self::Error> {
            self.sent_messages.push(item);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    struct SuccessfulCertSigner;

    #[async_trait::async_trait]
    impl AgentCertSigner for SuccessfulCertSigner {
        async fn sign_agent_csr(
            &self,
            _csr_pem: &str,
            _agent_id: &uuid::Uuid,
            _lifetime: time::Duration,
        ) -> crate::cert_signer::Result<SignedCertBundle> {
            Ok(SignedCertBundle {
                cert_pem: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n"
                    .to_string(),
                not_after: time::OffsetDateTime::now_utc().into(),
            })
        }

        fn active_ca_fingerprint(&self) -> String {
            "0".repeat(64)
        }
    }

    async fn insert_test_service_row(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        service_id: uuid::Uuid,
        service_app_name: &str,
    ) {
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("svc-{service_id}")),
            friendly_name: Set(format!("Service {service_id}")),
            ip_address: Set(None),
            status: Set(uptrakit_shared_types::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some(service_app_name.to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert test service");
    }

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    async fn system_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(system_audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    fn install_test_signer(state: &mut Arc<AppState>, signer: Arc<dyn AgentCertSigner>) {
        Arc::get_mut(state)
            .expect("test state should not be shared yet")
            .cert_signer = signer;
    }

    #[tokio::test]
    async fn request_certificate_not_approved_writes_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;
        let mut sink = TestMessageSink::recording();
        let mut out_seq = OutgoingSeq::new();

        let result = handle_request_certificate_with_sink(
            &mut sink,
            &state,
            service_id,
            false,
            false,
            &mut out_seq,
            &RequestCertificatePayload {
                csr_pem: "test-csr".to_string(),
            },
        )
        .await;

        assert!(matches!(result, CertificateResult::NotApproved));
        let row =
            tenant_audit_row_for_action(&db, AuditActionType::SERVICE_CERTIFICATE_ISSUE).await;
        assert_eq!(row.outcome, AuditOutcome::Denied.as_str());
        assert_eq!(row.actor_id, Some(service_id));
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], "not_approved");
    }

    #[tokio::test]
    async fn request_certificate_signing_failure_writes_failed_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;
        let mut sink = TestMessageSink::recording();
        let mut out_seq = OutgoingSeq::new();

        let result = handle_request_certificate_with_sink(
            &mut sink,
            &state,
            service_id,
            false,
            true,
            &mut out_seq,
            &RequestCertificatePayload {
                csr_pem: "test-csr".to_string(),
            },
        )
        .await;

        assert!(matches!(result, CertificateResult::Break));
        let row =
            tenant_audit_row_for_action(&db, AuditActionType::SERVICE_CERTIFICATE_ISSUE).await;
        assert_eq!(row.outcome, AuditOutcome::Failed.as_str());
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], "certificate_signing_failed");
    }

    #[tokio::test]
    async fn request_certificate_missing_system_service_writes_denied_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (mut state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        install_test_signer(&mut state, Arc::new(SuccessfulCertSigner));
        let service_id = uuid::Uuid::now_v7();
        let mut sink = TestMessageSink::recording();
        let mut out_seq = OutgoingSeq::new();

        let result = handle_request_certificate_with_sink(
            &mut sink,
            &state,
            service_id,
            true,
            true,
            &mut out_seq,
            &RequestCertificatePayload {
                csr_pem: "test-csr".to_string(),
            },
        )
        .await;

        assert!(matches!(result, CertificateResult::Break));
        let row =
            system_audit_row_for_action(&db, AuditActionType::SERVICE_CERTIFICATE_ISSUE).await;
        assert_eq!(row.outcome, AuditOutcome::Denied.as_str());
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], "system_service_not_found");
    }
}
