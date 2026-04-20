//! Service credential delivery for authenticated connections.
//!
//! Contains [`deliver_service_credentials`], which sends DB URL, NATS URL,
//! and master-key hex to services that carry the corresponding capabilities.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{Sink, SinkExt};

use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome};
use uptrakit_internal_wire::{
    Capability, ControllerMessage, OutgoingSeq, ServiceCredentialsPayload,
};

use crate::AppState;
use crate::routes::service_ws::protocol::serialize_controller_msg;

// ---------------------------------------------------------------------------
// deliver_service_credentials
// ---------------------------------------------------------------------------

/// Identity of the service receiving credentials.
pub(super) struct ServiceCredentialTarget<'a> {
    pub service_id: uuid::Uuid,
    pub is_system: bool,
    pub service_tenant_id: Option<uuid::Uuid>,
    pub service_app_name: Option<&'a str>,
}

/// Deliver service credentials (DB URL, NATS URL, master key) to services
/// that have the corresponding capabilities.
///
/// Returns `Some(())` on success (including when no credentials are needed)
/// or `None` if the WebSocket write failed and the connection should close.
pub(super) async fn deliver_service_credentials(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    capabilities: &BTreeSet<Capability>,
    target: ServiceCredentialTarget<'_>,
    out_seq: &mut OutgoingSeq,
) -> Option<()> {
    deliver_service_credentials_with_sink(sink, state, capabilities, target, out_seq).await
}

async fn deliver_service_credentials_with_sink<S>(
    sink: &mut S,
    state: &Arc<AppState>,
    capabilities: &BTreeSet<Capability>,
    target: ServiceCredentialTarget<'_>,
    out_seq: &mut OutgoingSeq,
) -> Option<()>
where
    S: Sink<Message> + Unpin,
{
    let has_db_access = capabilities.contains(&Capability::DatabaseAccess);
    let has_nats_access = capabilities.contains(&Capability::NatsAccess);
    let has_master_key_access = capabilities.contains(&Capability::MasterKeyAccess);

    if !has_db_access && !has_nats_access && !has_master_key_access {
        return Some(());
    }

    let credential_classes =
        delivered_credential_classes(has_db_access, has_nats_access, has_master_key_access);
    let sources = &state.credential_sources;
    let payload = ServiceCredentialsPayload {
        db_url: if has_db_access {
            sources
                .db_url
                .as_ref()
                .map(|u| uptrakit_internal_wire::SecretString::new(u.clone()))
        } else {
            None
        },
        nats_url: if has_nats_access {
            sources.nats_url.clone()
        } else {
            None
        },
        master_key_hex: if has_master_key_access {
            sources.master_key_hex.clone()
        } else {
            None
        },
    };
    let cred_msg = ControllerMessage::ServiceCredentials(payload);
    if let Some(json) = serialize_controller_msg(out_seq, cred_msg)
        && sink.send(Message::Text(json.into())).await.is_err()
    {
        emit_service_credentials_audit_event(
            state,
            &target,
            &credential_classes,
            AuditOutcome::Failed,
            Some("websocket_write_failed"),
        );
        return None;
    }
    emit_service_credentials_audit_event(
        state,
        &target,
        &credential_classes,
        AuditOutcome::Success,
        None,
    );
    tracing::info!(
        service_id = %target.service_id,
        db = has_db_access,
        nats = has_nats_access,
        master_key = has_master_key_access,
        "delivered service credentials"
    );

    Some(())
}

fn delivered_credential_classes(
    has_db_access: bool,
    has_nats_access: bool,
    has_master_key_access: bool,
) -> Vec<&'static str> {
    let mut classes = Vec::with_capacity(3);
    if has_db_access {
        classes.push("database_access");
    }
    if has_nats_access {
        classes.push("nats_access");
    }
    if has_master_key_access {
        classes.push("master_key_access");
    }
    classes
}

fn emit_service_credentials_audit_event(
    state: &Arc<AppState>,
    target: &ServiceCredentialTarget<'_>,
    credential_classes: &[&'static str],
    outcome: AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let &ServiceCredentialTarget {
        service_id,
        is_system,
        service_tenant_id,
        service_app_name,
    } = target;
    let mut details = serde_json::json!({
        "credential_classes": credential_classes,
    });
    if let Some(service_app_name) = service_app_name {
        details["service_app_name"] = serde_json::Value::String(service_app_name.to_string());
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::Value::String(reason_code.to_string());
    }

    let mut builder = AuditEntry::builder(AuditActionType::SERVICE_CREDENTIALS_DELIVER)
        .actor_service(service_id)
        .actor_display_opt(service_app_name.map(str::to_string))
        .target(
            "service",
            service_id.to_string(),
            service_app_name.map(str::to_string),
        )
        .outcome(outcome)
        .details(details);
    builder = if !is_system {
        builder.tenant_scope(
            service_tenant_id.expect("tenant service credential delivery requires tenant scope"),
        )
    } else {
        builder.system_scope()
    };

    match builder.build() {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            error = %error,
            %service_id,
            outcome = outcome.as_str(),
            "failed to build service credential delivery audit entry"
        ),
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;

    use std::pin::Pin;
    use std::task::{Context, Poll};

    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
    use tokio::time::{Duration, Instant};
    use uptrakit_shared_db::entity::system_audit_log;

    struct TestMessageSink {
        sent_messages: Vec<Message>,
        fail_send: bool,
    }

    impl TestMessageSink {
        fn recording() -> Self {
            Self {
                sent_messages: Vec::new(),
                fail_send: false,
            }
        }

        fn failing() -> Self {
            Self {
                sent_messages: Vec::new(),
                fail_send: true,
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
            if self.fail_send {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "simulated websocket write failure",
                ));
            }
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

    async fn insert_test_system_service_row(
        db: &sea_orm::DatabaseConnection,
        service_id: uuid::Uuid,
        service_app_name: &str,
    ) {
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::system_service::ActiveModel {
            id: Set(service_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("sys-{service_id}")),
            friendly_name: Set(format!("System Service {service_id}")),
            ip_address: Set(None),
            status: Set(uptrakit_shared_db::entity::system_service::SystemServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(Some(service_app_name.to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert system service row");
    }

    async fn wait_for_system_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> system_audit_log::Model {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(system_audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            assert!(
                Instant::now() <= deadline,
                "timed out waiting for system audit row for action {action_type}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn build_state_with_credential_sources() -> Arc<AppState> {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        Arc::new(AppState {
            credential_sources: crate::ServiceCredentialSources {
                db_url: Some("sqlite:///controller.db".to_string()),
                nats_url: Some("nats://127.0.0.1:4222".to_string()),
                master_key_hex: Some(uptrakit_web_api_types::SecretString::new("a".repeat(64))),
            },
            ..(*state).clone()
        })
    }

    #[tokio::test]
    async fn deliver_service_credentials_emits_success_system_audit_entry() {
        let state = build_state_with_credential_sources().await;
        let service_id = uuid::Uuid::now_v7();
        insert_test_system_service_row(state.db(), service_id, "uptrakit-scheduler").await;
        let capabilities =
            BTreeSet::from([Capability::DatabaseAccess, Capability::MasterKeyAccess]);
        let mut sink = TestMessageSink::recording();
        let mut out_seq = OutgoingSeq::default();

        let result = deliver_service_credentials_with_sink(
            &mut sink,
            &state,
            &capabilities,
            ServiceCredentialTarget {
                service_id,
                is_system: true,
                service_tenant_id: None,
                service_app_name: Some("uptrakit-scheduler"),
            },
            &mut out_seq,
        )
        .await;

        assert_eq!(result, Some(()));
        assert_eq!(sink.sent_messages.len(), 1);

        let row = wait_for_system_audit_row_for_action(
            state.db(),
            AuditActionType::SERVICE_CREDENTIALS_DELIVER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.actor_display.as_deref(), Some("uptrakit-scheduler"));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
        assert_eq!(row.target_display.as_deref(), Some("uptrakit-scheduler"));
        assert_eq!(row.outcome, AuditOutcome::Success.as_str());
        let details = row
            .details_json
            .as_ref()
            .expect("credential delivery audit should include details");
        assert_eq!(
            details["credential_classes"],
            serde_json::json!(["database_access", "master_key_access"])
        );
        assert_eq!(details["service_app_name"], "uptrakit-scheduler");
        assert!(details.get("reason_code").is_none());
        assert!(details.get("db_url").is_none());
        assert!(details.get("master_key_hex").is_none());
    }

    #[tokio::test]
    async fn deliver_service_credentials_emits_failed_system_audit_entry_when_write_fails() {
        let state = build_state_with_credential_sources().await;
        let service_id = uuid::Uuid::now_v7();
        insert_test_system_service_row(state.db(), service_id, "uptrakit-scheduler").await;
        let capabilities = BTreeSet::from([Capability::NatsAccess]);
        let mut sink = TestMessageSink::failing();
        let mut out_seq = OutgoingSeq::default();

        let result = deliver_service_credentials_with_sink(
            &mut sink,
            &state,
            &capabilities,
            ServiceCredentialTarget {
                service_id,
                is_system: true,
                service_tenant_id: None,
                service_app_name: Some("uptrakit-scheduler"),
            },
            &mut out_seq,
        )
        .await;

        assert_eq!(result, None);
        assert!(sink.sent_messages.is_empty());

        let row = wait_for_system_audit_row_for_action(
            state.db(),
            AuditActionType::SERVICE_CREDENTIALS_DELIVER,
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
        assert_eq!(row.outcome, AuditOutcome::Failed.as_str());
        let details = row
            .details_json
            .as_ref()
            .expect("credential delivery failure audit should include details");
        assert_eq!(
            details["credential_classes"],
            serde_json::json!(["nats_access"])
        );
        assert_eq!(details["reason_code"], "websocket_write_failed");
        assert!(details.get("nats_url").is_none());
    }
}
