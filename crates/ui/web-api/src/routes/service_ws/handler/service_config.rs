//! Controller-side handlers for the service config store wire protocol.
//!
//! Provides:
//! - [`deliver_service_config`]: called during session setup to send all stored
//!   entries for this `service_app_name` to the connecting service.
//! - [`handle_store_service_config`]: upsert handler for `StoreServiceConfig`.
//! - [`handle_delete_service_config`]: delete handler for `DeleteServiceConfig`.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{Sink, SinkExt};

use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome};
use uptrakit_wire::{
    ControllerMessage, DeleteServiceConfigPayload, OutgoingSeq, ServiceConfigAckPayload,
    ServiceConfigDeliveryPayload, ServiceConfigEntry, ServiceConfigKey,
    ServiceConfigUpdatedPayload, StoreServiceConfigPayload,
};

use super::shared_types::ProcessorResponse;
use crate::AppState;
use crate::routes::service_ws::protocol::serialize_controller_msg;

pub(super) struct ServiceConfigAuditCtx<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) action_type: uptrakit_audit_log::RegisteredAuditAction,
    pub(super) service_id: uuid::Uuid,
    pub(super) service_app_name: &'a str,
}

struct ServiceScopeCtx<'a> {
    state: &'a Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    service_app_name: &'a str,
}

/// Deliver all stored config entries for `service_app_name` to the connecting service.
///
/// Called during session setup after credential delivery.
/// Returns `Some(())` on success or `None` if the WebSocket write failed.
pub(super) async fn deliver_service_config(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    service_app_name: &str,
    out_seq: &mut OutgoingSeq,
) -> Option<()> {
    deliver_service_config_with_sink(
        sink,
        state,
        service_id,
        is_system,
        service_tenant_id,
        service_app_name,
        out_seq,
    )
    .await
}

async fn deliver_service_config_with_sink<S>(
    sink: &mut S,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    service_app_name: &str,
    out_seq: &mut OutgoingSeq,
) -> Option<()>
where
    S: Sink<Message> + Unpin,
{
    let rows = match crate::queries::service_config::load_for_service(state.db(), service_app_name)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                "failed to load service config entries; skipping delivery"
            );
            emit_service_config_delivery_audit_event(
                &ServiceScopeCtx {
                    state,
                    service_id,
                    is_system,
                    service_tenant_id,
                    service_app_name,
                },
                0,
                0,
                0,
                AuditOutcome::Failed,
                Some("load_failed"),
            );
            return Some(()); // non-fatal: continue session setup
        }
    };

    let delivered_entry_count = rows.len();
    let tenant_entry_count = rows.iter().filter(|row| row.tenant_id.is_some()).count();
    let global_entry_count = delivered_entry_count.saturating_sub(tenant_entry_count);

    let entries: Vec<ServiceConfigEntry> = rows
        .into_iter()
        .map(|r| ServiceConfigEntry::new(r.tenant_id, r.key, r.value))
        .collect();

    let msg = ControllerMessage::ServiceConfigDelivery(ServiceConfigDeliveryPayload::new(entries));
    if let Some(json) = serialize_controller_msg(out_seq, msg)
        && sink.send(Message::Text(json.into())).await.is_err()
    {
        emit_service_config_delivery_audit_event(
            &ServiceScopeCtx {
                state,
                service_id,
                is_system,
                service_tenant_id,
                service_app_name,
            },
            delivered_entry_count,
            tenant_entry_count,
            global_entry_count,
            AuditOutcome::Failed,
            Some("websocket_write_failed"),
        );
        return None;
    }

    emit_service_config_delivery_audit_event(
        &ServiceScopeCtx {
            state,
            service_id,
            is_system,
            service_tenant_id,
            service_app_name,
        },
        delivered_entry_count,
        tenant_entry_count,
        global_entry_count,
        AuditOutcome::Success,
        None,
    );

    Some(())
}

fn service_config_scope_label(tenant_id: Option<uuid::Uuid>) -> &'static str {
    if tenant_id.is_some() {
        "tenant"
    } else {
        "global"
    }
}

fn service_config_target_id(
    service_app_name: &str,
    tenant_id: Option<uuid::Uuid>,
    key: &str,
) -> String {
    match tenant_id {
        Some(tenant_id) => format!("{service_app_name}:{tenant_id}:{key}"),
        None => format!("{service_app_name}:global:{key}"),
    }
}

fn emit_service_config_audit_event(
    ctx: &ServiceConfigAuditCtx<'_>,
    scope_tenant_id: Option<uuid::Uuid>,
    target_tenant_id: Option<uuid::Uuid>,
    key: &str,
    request_id: &str,
    outcome: AuditOutcome,
    details: serde_json::Value,
) {
    let target_id = service_config_target_id(ctx.service_app_name, target_tenant_id, key);
    let mut builder = AuditEntry::builder(ctx.action_type)
        .actor_service(ctx.service_id)
        .target("service_config", target_id, Some(key.to_string()))
        .outcome(outcome)
        .details(details)
        .request_id_opt(Some(request_id.to_string()));
    builder = if let Some(tenant_id) = scope_tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };

    match builder.build() {
        Ok(entry) => ctx.state.audit_log_dispatcher.dispatch(entry),
        Err(error) => tracing::warn!(
            error = %error,
            action_type = %ctx.action_type,
            service_app_name = ctx.service_app_name,
            key,
            "failed to build semantic audit entry for service config mutation"
        ),
    }
}

fn emit_service_config_delivery_audit_event(
    ctx: &ServiceScopeCtx<'_>,
    delivered_entry_count: usize,
    tenant_entry_count: usize,
    global_entry_count: usize,
    outcome: AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let mut details = serde_json::json!({
        "service_app_name": ctx.service_app_name,
        "delivered_entry_count": delivered_entry_count,
        "tenant_entry_count": tenant_entry_count,
        "global_entry_count": global_entry_count,
    });
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::Value::String(reason_code.to_string());
    }

    let mut builder = AuditEntry::builder(AuditActionType::SERVICE_CONFIG_DELIVER)
        .actor_service(ctx.service_id)
        .actor_display_opt(Some(ctx.service_app_name.to_string()))
        .target(
            "service",
            ctx.service_id.to_string(),
            Some(ctx.service_app_name.to_string()),
        )
        .outcome(outcome)
        .details(details);
    builder = if ctx.is_system {
        builder.system_scope()
    } else {
        builder.tenant_scope(
            ctx.service_tenant_id
                .expect("tenant service config delivery requires tenant scope"),
        )
    };

    match builder.build() {
        Ok(entry) => ctx.state.audit_log_dispatcher.dispatch(entry),
        Err(error) => tracing::warn!(
            error = %error,
            service_id = %ctx.service_id,
            service_app_name = ctx.service_app_name,
            outcome = outcome.as_str(),
            "failed to build semantic audit entry for service config delivery"
        ),
    }
}

pub(super) fn emit_service_config_scope_denied_audit_event(
    ctx: ServiceConfigAuditCtx<'_>,
    service_tenant_id: uuid::Uuid,
    requested_tenant_id: Option<uuid::Uuid>,
    key: &str,
    request_id: &str,
    reason_code: &'static str,
) {
    let service_app_name = ctx.service_app_name;
    emit_service_config_audit_event(
        &ctx,
        Some(service_tenant_id),
        requested_tenant_id,
        key,
        request_id,
        AuditOutcome::Denied,
        serde_json::json!({
            "service_app_name": service_app_name,
            "key": key,
            "requested_scope": service_config_scope_label(requested_tenant_id),
            "service_tenant_id": service_tenant_id,
            "requested_tenant_id": requested_tenant_id,
            "reason_code": reason_code
        }),
    );
}

/// Handle a `StoreServiceConfig` message: upsert the entry, ACK, and broadcast.
pub(super) async fn handle_store_service_config(
    state: &Arc<AppState>,
    service_app_name: &str,
    service_id: uuid::Uuid,
    payload: StoreServiceConfigPayload,
) -> ProcessorResponse {
    let result = crate::queries::service_config::upsert(
        state.db(),
        service_app_name,
        payload.tenant_id,
        &payload.key,
        payload.value.clone(),
        payload.sensitive,
    )
    .await;

    let request_id = payload.request_id.clone();
    let tenant_id = payload.tenant_id;
    let key = payload.key.clone();
    let scope = service_config_scope_label(tenant_id);

    match result {
        Ok(plaintext_value) => {
            // ACK to the requesting service.
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::success(
                payload.request_id,
            ));

            // Broadcast ServiceConfigUpdated to all OTHER instances of the same service.
            let update = ControllerMessage::ServiceConfigUpdated(ServiceConfigUpdatedPayload::new(
                vec![ServiceConfigEntry::new(
                    payload.tenant_id,
                    payload.key.clone(),
                    plaintext_value,
                )],
                vec![],
            ));
            state
                .service_connections
                .broadcast_to_app_except(service_app_name, service_id, update)
                .await;

            emit_service_config_audit_event(
                &ServiceConfigAuditCtx {
                    state,
                    action_type: AuditActionType::SERVICE_CONFIG_STORE,
                    service_id,
                    service_app_name,
                },
                tenant_id,
                tenant_id,
                &key,
                &request_id,
                AuditOutcome::Success,
                serde_json::json!({
                    "service_app_name": service_app_name,
                    "key": key,
                    "scope": scope,
                    "sensitive": payload.sensitive
                }),
            );

            tracing::debug!(
                service_app_name,
                key = %payload.key,
                "stored service config entry"
            );
            ProcessorResponse::reply(ack)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %payload.key,
                "failed to store service config entry"
            );
            let error = e.to_string();
            emit_service_config_audit_event(
                &ServiceConfigAuditCtx {
                    state,
                    action_type: AuditActionType::SERVICE_CONFIG_STORE,
                    service_id,
                    service_app_name,
                },
                tenant_id,
                tenant_id,
                &key,
                &request_id,
                AuditOutcome::Failed,
                serde_json::json!({
                    "service_app_name": service_app_name,
                    "key": key,
                    "scope": scope,
                    "sensitive": payload.sensitive,
                    "error": error.clone()
                }),
            );
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::error(
                payload.request_id,
                error,
            ));
            ProcessorResponse::reply(ack)
        }
    }
}

/// Handle a `DeleteServiceConfig` message: delete the entry, ACK, and broadcast.
pub(super) async fn handle_delete_service_config(
    state: &Arc<AppState>,
    service_app_name: &str,
    service_id: uuid::Uuid,
    payload: DeleteServiceConfigPayload,
) -> ProcessorResponse {
    let result = crate::queries::service_config::delete(
        state.db(),
        service_app_name,
        payload.tenant_id,
        &payload.key,
    )
    .await;

    let request_id = payload.request_id.clone();
    let tenant_id = payload.tenant_id;
    let key = payload.key.clone();
    let scope = service_config_scope_label(tenant_id);

    match result {
        Ok(deleted) => {
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::success(
                payload.request_id,
            ));

            // Broadcast ServiceConfigUpdated to all OTHER instances.
            let update = ControllerMessage::ServiceConfigUpdated(ServiceConfigUpdatedPayload::new(
                vec![],
                vec![ServiceConfigKey::new(
                    payload.tenant_id,
                    payload.key.clone(),
                )],
            ));
            state
                .service_connections
                .broadcast_to_app_except(service_app_name, service_id, update)
                .await;

            emit_service_config_audit_event(
                &ServiceConfigAuditCtx {
                    state,
                    action_type: AuditActionType::SERVICE_CONFIG_DELETE,
                    service_id,
                    service_app_name,
                },
                tenant_id,
                tenant_id,
                &key,
                &request_id,
                AuditOutcome::Success,
                serde_json::json!({
                    "service_app_name": service_app_name,
                    "key": key,
                    "scope": scope,
                    "deleted": deleted
                }),
            );

            tracing::debug!(
                service_app_name,
                key = %payload.key,
                "deleted service config entry"
            );
            ProcessorResponse::reply(ack)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %payload.key,
                "failed to delete service config entry"
            );
            let error = e.to_string();
            emit_service_config_audit_event(
                &ServiceConfigAuditCtx {
                    state,
                    action_type: AuditActionType::SERVICE_CONFIG_DELETE,
                    service_id,
                    service_app_name,
                },
                tenant_id,
                tenant_id,
                &key,
                &request_id,
                AuditOutcome::Failed,
                serde_json::json!({
                    "service_app_name": service_app_name,
                    "key": key,
                    "scope": scope,
                    "error": error.clone()
                }),
            );
            let ack = ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::error(
                payload.request_id,
                error,
            ));
            ProcessorResponse::reply(ack)
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;

    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use tokio::time::{Duration, Instant};
    use uptrakit_audit_log::{AuditActorType, AuditLogBackend, AuditLogDispatcher, AuditLogError};

    #[derive(Default)]
    struct RecordingAuditBackend {
        entries: parking_lot::Mutex<Vec<AuditEntry>>,
    }

    impl RecordingAuditBackend {
        fn snapshot(&self) -> Vec<AuditEntry> {
            self.entries.lock().clone()
        }
    }

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

    #[async_trait::async_trait]
    impl AuditLogBackend for RecordingAuditBackend {
        async fn write(&self, entry: &AuditEntry) -> std::result::Result<(), AuditLogError> {
            self.entries.lock().push(entry.clone());
            Ok(())
        }
    }

    async fn build_test_state_with_recording_backend(
        backend: Arc<RecordingAuditBackend>,
    ) -> (Arc<AppState>, uuid::Uuid) {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let backend_dyn: Arc<dyn AuditLogBackend> = backend;
        (
            Arc::new(AppState {
                audit_log_dispatcher: AuditLogDispatcher::new(backend_dyn),
                ..(*state).clone()
            }),
            tenant_id,
        )
    }

    async fn wait_for_first_audit_entry(backend: &RecordingAuditBackend) -> AuditEntry {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let entries = backend.snapshot();
            if let Some(first) = entries.first() {
                return first.clone();
            }
            assert!(
                Instant::now() <= deadline,
                "timed out waiting for audit entry"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn assert_ack(
        response: &ProcessorResponse,
        request_id: &str,
        expected_success: bool,
    ) -> Option<String> {
        let [ControllerMessage::ServiceConfigAck(ack)] = response.replies.as_slice() else {
            panic!("expected exactly one ServiceConfigAck reply");
        };
        assert_eq!(ack.request_id, request_id);
        assert_eq!(ack.success, expected_success);
        ack.error.clone()
    }

    #[tokio::test]
    async fn deliver_service_config_emits_success_audit_entry() {
        let backend = Arc::new(RecordingAuditBackend::default());
        let (state, tenant_id) =
            build_test_state_with_recording_backend(Arc::clone(&backend)).await;
        crate::queries::service_config::upsert(
            state.db(),
            "uptrakit-mqtt",
            None,
            "smtp.global",
            serde_json::json!({"host": "smtp.internal"}),
            true,
        )
        .await
        .expect("insert global service config row");
        crate::queries::service_config::upsert(
            state.db(),
            "uptrakit-mqtt",
            Some(tenant_id),
            "clients.primary",
            serde_json::json!({"enabled": true}),
            false,
        )
        .await
        .expect("insert tenant service config row");

        let service_id = uuid::Uuid::now_v7();
        let mut sink = TestMessageSink::recording();
        let mut out_seq = OutgoingSeq::default();

        let result = deliver_service_config_with_sink(
            &mut sink,
            &state,
            service_id,
            false,
            Some(tenant_id),
            "uptrakit-mqtt",
            &mut out_seq,
        )
        .await;

        assert_eq!(result, Some(()));
        assert_eq!(sink.sent_messages.len(), 1);

        let entry = wait_for_first_audit_entry(backend.as_ref()).await;
        assert_eq!(entry.action_type, AuditActionType::SERVICE_CONFIG_DELIVER);
        assert_eq!(entry.outcome, AuditOutcome::Success);
        assert_eq!(entry.actor_type, AuditActorType::Service);
        assert_eq!(entry.actor_id, Some(service_id));
        assert_eq!(entry.tenant_id, Some(tenant_id));
        assert_eq!(entry.target_type.as_deref(), Some("service"));
        assert_eq!(entry.target_display.as_deref(), Some("uptrakit-mqtt"));
        let details = entry
            .details_json
            .as_ref()
            .expect("delivery audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["delivered_entry_count"], 2);
        assert_eq!(details["tenant_entry_count"], 1);
        assert_eq!(details["global_entry_count"], 1);
        assert!(details.get("reason_code").is_none());
    }

    #[tokio::test]
    async fn deliver_service_config_emits_failed_audit_entry_when_write_fails() {
        let backend = Arc::new(RecordingAuditBackend::default());
        let (state, tenant_id) =
            build_test_state_with_recording_backend(Arc::clone(&backend)).await;
        crate::queries::service_config::upsert(
            state.db(),
            "uptrakit-mqtt",
            Some(tenant_id),
            "clients.primary",
            serde_json::json!({"enabled": true}),
            true,
        )
        .await
        .expect("insert service config row");

        let service_id = uuid::Uuid::now_v7();
        let mut sink = TestMessageSink::failing();
        let mut out_seq = OutgoingSeq::default();

        let result = deliver_service_config_with_sink(
            &mut sink,
            &state,
            service_id,
            false,
            Some(tenant_id),
            "uptrakit-mqtt",
            &mut out_seq,
        )
        .await;

        assert_eq!(result, None);
        assert!(sink.sent_messages.is_empty());

        let entry = wait_for_first_audit_entry(backend.as_ref()).await;
        assert_eq!(entry.action_type, AuditActionType::SERVICE_CONFIG_DELIVER);
        assert_eq!(entry.outcome, AuditOutcome::Failed);
        assert_eq!(entry.actor_type, AuditActorType::Service);
        assert_eq!(entry.actor_id, Some(service_id));
        assert_eq!(entry.tenant_id, Some(tenant_id));
        let details = entry
            .details_json
            .as_ref()
            .expect("delivery failure audit should include details");
        assert_eq!(details["delivered_entry_count"], 1);
        assert_eq!(details["reason_code"], "websocket_write_failed");
    }

    #[tokio::test]
    async fn deliver_service_config_emits_failed_audit_entry_when_load_fails() {
        let backend = Arc::new(RecordingAuditBackend::default());
        let (state, tenant_id) =
            build_test_state_with_recording_backend(Arc::clone(&backend)).await;
        state
            .db()
            .clone()
            .close()
            .await
            .expect("test db close should succeed");

        let service_id = uuid::Uuid::now_v7();
        let mut sink = TestMessageSink::recording();
        let mut out_seq = OutgoingSeq::default();

        let result = deliver_service_config_with_sink(
            &mut sink,
            &state,
            service_id,
            false,
            Some(tenant_id),
            "uptrakit-mqtt",
            &mut out_seq,
        )
        .await;

        assert_eq!(result, Some(()));
        assert!(sink.sent_messages.is_empty());

        let entry = wait_for_first_audit_entry(backend.as_ref()).await;
        assert_eq!(entry.action_type, AuditActionType::SERVICE_CONFIG_DELIVER);
        assert_eq!(entry.outcome, AuditOutcome::Failed);
        assert_eq!(entry.actor_type, AuditActorType::Service);
        assert_eq!(entry.actor_id, Some(service_id));
        assert_eq!(entry.tenant_id, Some(tenant_id));
        let details = entry
            .details_json
            .as_ref()
            .expect("delivery load failure audit should include details");
        assert_eq!(details["delivered_entry_count"], 0);
        assert_eq!(details["tenant_entry_count"], 0);
        assert_eq!(details["global_entry_count"], 0);
        assert_eq!(details["reason_code"], "load_failed");
    }

    #[tokio::test]
    async fn handle_store_service_config_emits_success_audit_entry() {
        let backend = Arc::new(RecordingAuditBackend::default());
        let (state, tenant_id) =
            build_test_state_with_recording_backend(Arc::clone(&backend)).await;
        let tenant_id = Some(tenant_id);
        let service_id = uuid::Uuid::now_v7();
        let payload = StoreServiceConfigPayload::new(
            "req-store-success".to_string(),
            tenant_id,
            "clients.primary".to_string(),
            serde_json::json!({"enabled": true}),
            true,
        );

        let response =
            handle_store_service_config(&state, "uptrakit-mqtt", service_id, payload).await;
        let error = assert_ack(&response, "req-store-success", true);
        assert!(error.is_none());

        let entry = wait_for_first_audit_entry(backend.as_ref()).await;
        assert_eq!(entry.action_type, AuditActionType::SERVICE_CONFIG_STORE);
        assert_eq!(entry.outcome, AuditOutcome::Success);
        assert_eq!(entry.actor_type, AuditActorType::Service);
        assert_eq!(entry.actor_id, Some(service_id));
        assert_eq!(entry.tenant_id, tenant_id);
        assert_eq!(entry.target_type.as_deref(), Some("service_config"));
        assert_eq!(entry.target_display.as_deref(), Some("clients.primary"));
        assert_eq!(
            entry.target_id.as_ref(),
            Some(&service_config_target_id(
                "uptrakit-mqtt",
                tenant_id,
                "clients.primary"
            ))
        );
        assert_eq!(entry.request_id.as_deref(), Some("req-store-success"));
        let details = entry
            .details_json
            .as_ref()
            .expect("store audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["scope"], "tenant");
        assert_eq!(details["sensitive"], true);
    }

    #[tokio::test]
    async fn handle_store_service_config_emits_failed_audit_entry_when_db_write_fails() {
        let backend = Arc::new(RecordingAuditBackend::default());
        let (state, _tenant_id) =
            build_test_state_with_recording_backend(Arc::clone(&backend)).await;
        let service_id = uuid::Uuid::now_v7();
        state
            .db()
            .clone()
            .close()
            .await
            .expect("test db close should succeed");

        let response = handle_store_service_config(
            &state,
            "uptrakit-mqtt",
            service_id,
            StoreServiceConfigPayload::new(
                "req-store-failure".to_string(),
                None,
                "clients.primary".to_string(),
                serde_json::json!({"enabled": true}),
                false,
            ),
        )
        .await;
        let error = assert_ack(&response, "req-store-failure", false);
        assert!(error.is_some());

        let entry = wait_for_first_audit_entry(backend.as_ref()).await;
        assert_eq!(entry.action_type, AuditActionType::SERVICE_CONFIG_STORE);
        assert_eq!(entry.outcome, AuditOutcome::Failed);
        assert_eq!(entry.actor_type, AuditActorType::Service);
        assert_eq!(entry.actor_id, Some(service_id));
        assert_eq!(entry.tenant_id, None);
        let details = entry
            .details_json
            .as_ref()
            .expect("store failure audit should include details");
        assert_eq!(details["scope"], "global");
        assert!(
            details["error"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "store failure audit should include a non-empty error"
        );
    }

    #[tokio::test]
    async fn handle_delete_service_config_emits_success_audit_entry() {
        let backend = Arc::new(RecordingAuditBackend::default());
        let (state, tenant_id) =
            build_test_state_with_recording_backend(Arc::clone(&backend)).await;
        let tenant_id = Some(tenant_id);
        let service_id = uuid::Uuid::now_v7();
        let payload = DeleteServiceConfigPayload::new(
            "req-delete-success".to_string(),
            tenant_id,
            "clients.primary".to_string(),
        );

        let response =
            handle_delete_service_config(&state, "uptrakit-mqtt", service_id, payload).await;
        let error = assert_ack(&response, "req-delete-success", true);
        assert!(error.is_none());

        let entry = wait_for_first_audit_entry(backend.as_ref()).await;
        assert_eq!(entry.action_type, AuditActionType::SERVICE_CONFIG_DELETE);
        assert_eq!(entry.outcome, AuditOutcome::Success);
        assert_eq!(entry.actor_type, AuditActorType::Service);
        assert_eq!(entry.actor_id, Some(service_id));
        assert_eq!(entry.tenant_id, tenant_id);
        assert_eq!(entry.target_type.as_deref(), Some("service_config"));
        assert_eq!(entry.target_display.as_deref(), Some("clients.primary"));
        assert_eq!(entry.request_id.as_deref(), Some("req-delete-success"));
        let details = entry
            .details_json
            .as_ref()
            .expect("delete audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["scope"], "tenant");
        assert_eq!(details["deleted"], false);
    }

    #[tokio::test]
    async fn handle_delete_service_config_emits_failed_audit_entry_when_db_write_fails() {
        let backend = Arc::new(RecordingAuditBackend::default());
        let (state, _tenant_id) =
            build_test_state_with_recording_backend(Arc::clone(&backend)).await;
        let service_id = uuid::Uuid::now_v7();
        state
            .db()
            .clone()
            .close()
            .await
            .expect("test db close should succeed");

        let response = handle_delete_service_config(
            &state,
            "uptrakit-mqtt",
            service_id,
            DeleteServiceConfigPayload::new(
                "req-delete-failure".to_string(),
                None,
                "clients.primary".to_string(),
            ),
        )
        .await;
        let error = assert_ack(&response, "req-delete-failure", false);
        assert!(error.is_some());

        let entry = wait_for_first_audit_entry(backend.as_ref()).await;
        assert_eq!(entry.action_type, AuditActionType::SERVICE_CONFIG_DELETE);
        assert_eq!(entry.outcome, AuditOutcome::Failed);
        assert_eq!(entry.actor_type, AuditActorType::Service);
        assert_eq!(entry.actor_id, Some(service_id));
        assert_eq!(entry.tenant_id, None);
        let details = entry
            .details_json
            .as_ref()
            .expect("delete failure audit should include details");
        assert_eq!(details["scope"], "global");
        assert!(
            details["error"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "delete failure audit should include a non-empty error"
        );
    }
}
