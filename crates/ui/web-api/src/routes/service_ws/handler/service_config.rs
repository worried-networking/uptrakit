//! Controller-side handlers for the service config store wire protocol.
//!
//! Provides:
//! - [`deliver_service_config`]: called during session setup to send all stored
//!   entries for this `service_app_name` to the connecting service.
//! - [`handle_store_service_config`]: upsert handler for `StoreServiceConfig`.
//! - [`handle_delete_service_config`]: delete handler for `DeleteServiceConfig`.

#![expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]
#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]

use std::sync::Arc;
use uptrakit_shared_db::begin_immediate;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{Sink, SinkExt};

use uptrakit_audit_log::{
    AbsentView, AuditActionType, AuditActorType, AuditEntry, AuditOutcome, AuditView, Event,
    Stateful,
};
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
    let target_id = match requested_tenant_id {
        Some(tid) => format!("{}:{}:{}", service_app_name, tid, key),
        None => format!("{}:global:{}", service_app_name, key),
    };

    let builder = AuditEntry::<Event>::builder_event(ctx.action_type)
        .actor(AuditActorType::Service, Some(ctx.service_id))
        .target("service_config", target_id, Some(key.to_string()))
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "service_app_name": service_app_name,
            "key": key,
            "requested_scope": service_config_scope_label(requested_tenant_id),
            "service_tenant_id": service_tenant_id,
            "requested_tenant_id": requested_tenant_id,
            "reason_code": reason_code
        }))
        .request_id_opt(Some(request_id.to_string()))
        .tenant_scope(service_tenant_id);

    match builder.build() {
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
        Err(error) => tracing::warn!(
            error = %error,
            action_type = %ctx.action_type,
            service_app_name,
            key,
            "failed to build scope-denied audit entry for service config"
        ),
    }
}

/// Handle a `StoreServiceConfig` message: upsert the entry, ACK, and broadcast.
pub(super) async fn handle_store_service_config(
    state: &Arc<AppState>,
    service_app_name: &str,
    service_id: uuid::Uuid,
    payload: StoreServiceConfigPayload,
) -> ProcessorResponse {
    let tenant_id = payload.tenant_id;
    let key = payload.key.clone();
    let request_id = payload.request_id.clone();
    let scope = service_config_scope_label(tenant_id);

    // Open a BEGIN IMMEDIATE transaction (read-then-write requires IMMEDIATE to
    // avoid SQLITE_BUSY_SNAPSHOT on concurrent writes).
    let tx = match begin_immediate(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %key,
                "failed to begin transaction for service config store"
            );
            let error = e.to_string();
            return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                ServiceConfigAckPayload::error(payload.request_id, error),
            ));
        }
    };

    let (before_opt, after_view, plaintext_value) =
        match crate::queries::service_config::upsert_in_tx(
            &tx,
            service_app_name,
            payload.tenant_id,
            &payload.key,
            payload.value.clone(),
            payload.sensitive,
        )
        .await
        {
            Ok(result) => result,
            Err(e) => {
                drop(tx);
                tracing::warn!(
                    error = %e,
                    service_app_name,
                    key = %key,
                    "failed to store service config entry"
                );
                let error = e.to_string();

                // Emit fire-and-forget failed event via dispatcher.
                let target_id = match tenant_id {
                    Some(tid) => format!("{}:{}:{}", service_app_name, tid, key),
                    None => format!("{}:global:{}", service_app_name, key),
                };
                let failed_builder =
                    AuditEntry::<Event>::builder_event(AuditActionType::SERVICE_CONFIG_STORE)
                        .actor(AuditActorType::Service, Some(service_id))
                        .target("service_config", target_id, Some(key.clone()))
                        .outcome(AuditOutcome::Failed)
                        .details(serde_json::json!({
                            "service_app_name": service_app_name,
                            "key": key,
                            "scope": scope,
                            "sensitive": payload.sensitive,
                            "error": error.clone(),
                        }))
                        .request_id_opt(Some(request_id));
                let failed_builder = if let Some(tid) = tenant_id {
                    failed_builder.tenant_scope(tid)
                } else {
                    failed_builder.system_scope()
                };
                if let Ok(entry) = failed_builder.build() {
                    state.audit_emitter.emit_event(entry);
                }

                return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                    ServiceConfigAckPayload::error(payload.request_id, error),
                ));
            }
        };

    // Build the V2 stateful audit entry.
    let audit_entry = {
        let builder = match before_opt.as_ref() {
            Some(before) => AuditEntry::<Stateful>::service_config_store(before, &after_view),
            None => {
                AuditEntry::<Stateful>::service_config_store(&AbsentView(&after_view), &after_view)
            }
        };
        let builder = if let Some(tid) = tenant_id {
            builder.tenant_scope(tid)
        } else {
            builder.system_scope()
        };
        builder
            .actor(AuditActorType::Service, Some(service_id))
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({
                "service_app_name": service_app_name,
                "key": key,
                "scope": scope,
                "sensitive": payload.sensitive,
            }))
            .request_id_opt(Some(request_id))
            .build()
    };

    let audit_entry = match audit_entry {
        Ok(entry) => entry,
        Err(e) => {
            drop(tx);
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %key,
                "failed to build audit entry for service config store"
            );
            return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                ServiceConfigAckPayload::error(
                    payload.request_id,
                    "internal error: audit entry build failed".to_string(),
                ),
            ));
        }
    };

    let hook = state.audit_emitter.commit_hook();
    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        drop(tx);
        tracing::warn!(
            error = %e,
            service_app_name,
            key = %key,
            "failed to emit stateful audit entry for service config store"
        );
        return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
            ServiceConfigAckPayload::error(
                payload.request_id,
                "internal error: audit emit failed".to_string(),
            ),
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::warn!(
            error = %e,
            service_app_name,
            key = %key,
            "failed to commit service config store transaction"
        );
        return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
            ServiceConfigAckPayload::error(
                payload.request_id,
                "internal error: transaction commit failed".to_string(),
            ),
        ));
    }
    hook.flush_after_commit().await;

    tracing::debug!(
        service_app_name,
        key = %payload.key,
        "stored service config entry"
    );

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

    let ack =
        ControllerMessage::ServiceConfigAck(ServiceConfigAckPayload::success(payload.request_id));
    ProcessorResponse::reply(ack)
}

/// Handle a `DeleteServiceConfig` message: delete the entry, ACK, and broadcast.
pub(super) async fn handle_delete_service_config(
    state: &Arc<AppState>,
    service_app_name: &str,
    service_id: uuid::Uuid,
    payload: DeleteServiceConfigPayload,
) -> ProcessorResponse {
    let tenant_id = payload.tenant_id;
    let key = payload.key.clone();
    let request_id = payload.request_id.clone();
    let scope = service_config_scope_label(tenant_id);

    // Open a BEGIN IMMEDIATE transaction.
    let tx = match begin_immediate(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %key,
                "failed to begin transaction for service config delete"
            );
            let error = e.to_string();
            return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                ServiceConfigAckPayload::error(payload.request_id, error),
            ));
        }
    };

    let before_opt = match crate::queries::service_config::delete_in_tx(
        &tx,
        service_app_name,
        payload.tenant_id,
        &payload.key,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            drop(tx);
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %key,
                "failed to delete service config entry"
            );
            let error = e.to_string();

            // Emit fire-and-forget failed event.
            let del_target_id = match tenant_id {
                Some(tid) => format!("{}:{}:{}", service_app_name, tid, key),
                None => format!("{}:global:{}", service_app_name, key),
            };
            let del_failed_builder =
                AuditEntry::<Event>::builder_event(AuditActionType::SERVICE_CONFIG_DELETE)
                    .actor(AuditActorType::Service, Some(service_id))
                    .target("service_config", del_target_id, Some(key.clone()))
                    .outcome(AuditOutcome::Failed)
                    .details(serde_json::json!({
                        "service_app_name": service_app_name,
                        "key": key,
                        "scope": scope,
                        "error": error.clone(),
                    }))
                    .request_id_opt(Some(request_id));
            let del_failed_builder = if let Some(tid) = tenant_id {
                del_failed_builder.tenant_scope(tid)
            } else {
                del_failed_builder.system_scope()
            };
            if let Ok(entry) = del_failed_builder.build() {
                state.audit_emitter.emit_event(entry);
            }

            return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                ServiceConfigAckPayload::error(payload.request_id, error),
            ));
        }
    };

    // If the row did not exist, emit a denied event (idempotent delete) and
    // return a success ACK — current behaviour is preserved.
    let Some(before_view) = before_opt else {
        drop(tx);

        // Build placeholder view for target metadata.
        let placeholder = crate::queries::service_config::ServiceConfigView {
            service_name: service_app_name.to_string(),
            key: key.clone(),
            tenant_id,
            sensitive: false,
        };

        let not_found_builder =
            AuditEntry::<Event>::builder_event(AuditActionType::SERVICE_CONFIG_DELETE)
                .actor(AuditActorType::Service, Some(service_id))
                .target(
                    "service_config",
                    placeholder.audit_target_id(),
                    placeholder.audit_target_display(),
                )
                .outcome(AuditOutcome::Denied)
                .details(serde_json::json!({
                    "service_app_name": service_app_name,
                    "key": key,
                    "scope": scope,
                    "reason_code": "not_found",
                }))
                .request_id_opt(Some(request_id));
        let not_found_builder = if let Some(tid) = tenant_id {
            not_found_builder.tenant_scope(tid)
        } else {
            not_found_builder.system_scope()
        };
        if let Ok(entry) = not_found_builder.build() {
            state.audit_emitter.emit_event(entry);
        }

        return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
            ServiceConfigAckPayload::success(payload.request_id),
        ));
    };

    // Build the V2 stateful audit entry: before = existing row, after = absent.
    let audit_entry =
        AuditEntry::<Stateful>::service_config_delete(&before_view, &AbsentView(&before_view));
    let builder = if let Some(tid) = tenant_id {
        audit_entry.tenant_scope(tid)
    } else {
        audit_entry.system_scope()
    };
    let audit_entry = match builder
        .actor(AuditActorType::Service, Some(service_id))
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "service_app_name": service_app_name,
            "key": key,
            "scope": scope,
        }))
        .request_id_opt(Some(request_id))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            drop(tx);
            tracing::warn!(
                error = %e,
                service_app_name,
                key = %key,
                "failed to build audit entry for service config delete"
            );
            return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
                ServiceConfigAckPayload::error(
                    payload.request_id,
                    "internal error: audit entry build failed".to_string(),
                ),
            ));
        }
    };

    let hook = state.audit_emitter.commit_hook();
    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        drop(tx);
        tracing::warn!(
            error = %e,
            service_app_name,
            key = %key,
            "failed to emit stateful audit entry for service config delete"
        );
        return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
            ServiceConfigAckPayload::error(
                payload.request_id,
                "internal error: audit emit failed".to_string(),
            ),
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::warn!(
            error = %e,
            service_app_name,
            key = %key,
            "failed to commit service config delete transaction"
        );
        return ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
            ServiceConfigAckPayload::error(
                payload.request_id,
                "internal error: transaction commit failed".to_string(),
            ),
        ));
    }
    hook.flush_after_commit().await;

    tracing::debug!(
        service_app_name,
        key = %payload.key,
        "deleted service config entry"
    );

    // Broadcast ServiceConfigUpdated deletion to all OTHER instances.
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

    ProcessorResponse::reply(ControllerMessage::ServiceConfigAck(
        ServiceConfigAckPayload::success(payload.request_id),
    ))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;

    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_audit_log::AuditActionType;
    use uptrakit_shared_db::entity::audit_log;

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

    /// Wait for an audit row matching `action_type` to appear in the DB.
    async fn wait_for_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..200 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for audit row for action {action_type}");
    }

    /// Same as `wait_for_audit_row` but also waits for `system_audit_logs`.
    async fn wait_for_system_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::system_audit_log::Model {
        for _ in 0..200 {
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
        panic!("timed out waiting for system audit row for action {action_type}");
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

    // ---------------------------------------------------------------------------
    // Deliver tests — these still use the recording pattern via audit_log_dispatcher
    // since deliver stays Event (fire-and-forget via dispatcher).
    // ---------------------------------------------------------------------------

    use uptrakit_audit_log::{
        AuditActorType as ActorType, AuditEntryErased, AuditLogBackend, AuditLogDispatcher,
        AuditLogError,
    };

    #[derive(Default)]
    struct RecordingAuditBackend {
        entries: parking_lot::Mutex<Vec<AuditEntryErased>>,
    }

    impl RecordingAuditBackend {
        fn snapshot(&self) -> Vec<AuditEntryErased> {
            self.entries.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl AuditLogBackend for RecordingAuditBackend {
        async fn write(
            &self,
            entry: &AuditEntryErased,
        ) -> std::result::Result<(), rootcause::Report<AuditLogError>> {
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

    async fn wait_for_first_audit_entry(backend: &RecordingAuditBackend) -> AuditEntryErased {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
        loop {
            let entries = backend.snapshot();
            if let Some(first) = entries.first() {
                return first.clone();
            }
            assert!(
                tokio::time::Instant::now() <= deadline,
                "timed out waiting for audit entry"
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
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
        assert_eq!(entry.actor_type, ActorType::Service);
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
        assert_eq!(entry.actor_type, ActorType::Service);
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
        assert_eq!(entry.actor_type, ActorType::Service);
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

    // ---------------------------------------------------------------------------
    // Store tests — V2: audit row goes to DB via emit_stateful.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn handle_store_service_config_emits_success_audit_entry() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let tenant_id_opt = Some(tenant_id);
        let service_id = uuid::Uuid::now_v7();
        let payload = StoreServiceConfigPayload::new(
            "req-store-success".to_string(),
            tenant_id_opt,
            "clients.primary".to_string(),
            serde_json::json!({"enabled": true}),
            true,
        );

        let response =
            handle_store_service_config(&state, "uptrakit-mqtt", service_id, payload).await;
        let error = assert_ack(&response, "req-store-success", true);
        assert!(error.is_none());

        let row = wait_for_audit_row(&db, AuditActionType::SERVICE_CONFIG_STORE).await;
        assert_eq!(AuditActionType::SERVICE_CONFIG_STORE, row.action_type);
        assert_eq!(row.outcome, AuditOutcome::Success.as_str());
        assert_eq!(row.actor_type, AuditActorType::Service.as_str());
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.tenant_id, tenant_id);
        assert_eq!(row.target_type.as_deref(), Some("service_config"));
        assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
        let expected_target_id = format!("uptrakit-mqtt:{}:clients.primary", tenant_id);
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
        assert_eq!(row.request_id.as_deref(), Some("req-store-success"));
        let details = row
            .details_json
            .expect("store audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["scope"], "tenant");
        assert_eq!(details["sensitive"], true);
        // before_snapshot is {} (insert = absent), after_snapshot has the view fields.
        let before = row.before_snapshot.expect("before_snapshot");
        assert_eq!(before, serde_json::json!({}));
        let after = row.after_snapshot.expect("after_snapshot");
        assert_eq!(after["key"], "clients.primary");
        assert_eq!(after["sensitive"], true);
        assert!(
            after.get("value").is_none(),
            "value must not appear in snapshot"
        );
    }

    #[tokio::test]
    async fn handle_store_service_config_update_emits_before_snapshot() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        // Pre-insert a row.
        crate::queries::service_config::upsert(
            state.db(),
            "uptrakit-mqtt",
            Some(tenant_id),
            "clients.primary",
            serde_json::json!({"enabled": false}),
            false,
        )
        .await
        .expect("pre-insert row");

        let service_id = uuid::Uuid::now_v7();
        let payload = StoreServiceConfigPayload::new(
            "req-store-update".to_string(),
            Some(tenant_id),
            "clients.primary".to_string(),
            serde_json::json!({"enabled": true}),
            true,
        );

        let response =
            handle_store_service_config(&state, "uptrakit-mqtt", service_id, payload).await;
        let error = assert_ack(&response, "req-store-update", true);
        assert!(error.is_none());

        let row = wait_for_audit_row(&db, AuditActionType::SERVICE_CONFIG_STORE).await;
        let before = row
            .before_snapshot
            .expect("before_snapshot must be present for update");
        // before_snapshot should have the old sensitive value (false).
        assert_eq!(before["sensitive"], false);
        let after = row.after_snapshot.expect("after_snapshot");
        assert_eq!(after["sensitive"], true);
    }

    #[tokio::test]
    async fn handle_store_service_config_global_scope_uses_system_scope() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let service_id = uuid::Uuid::now_v7();
        let payload = StoreServiceConfigPayload::new(
            "req-store-global".to_string(),
            None, // global scope
            "smtp.host".to_string(),
            serde_json::json!("smtp.example.com"),
            false,
        );

        let response =
            handle_store_service_config(&state, "uptrakit-mqtt", service_id, payload).await;
        let error = assert_ack(&response, "req-store-global", true);
        assert!(error.is_none());

        // Global scope writes to system_audit_log.
        let row = wait_for_system_audit_row(&db, AuditActionType::SERVICE_CONFIG_STORE).await;
        assert_eq!(AuditActionType::SERVICE_CONFIG_STORE, row.action_type);
        assert_eq!(row.outcome, AuditOutcome::Success.as_str());
    }

    // ---------------------------------------------------------------------------
    // Delete tests — V2: audit row goes to DB via emit_stateful.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn handle_delete_service_config_emits_success_audit_entry() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        // Pre-insert a row to delete.
        crate::queries::service_config::upsert(
            state.db(),
            "uptrakit-mqtt",
            Some(tenant_id),
            "clients.primary",
            serde_json::json!({"enabled": true}),
            false,
        )
        .await
        .expect("pre-insert row");

        let tenant_id_opt = Some(tenant_id);
        let service_id = uuid::Uuid::now_v7();
        let payload = DeleteServiceConfigPayload::new(
            "req-delete-success".to_string(),
            tenant_id_opt,
            "clients.primary".to_string(),
        );

        let response =
            handle_delete_service_config(&state, "uptrakit-mqtt", service_id, payload).await;
        let error = assert_ack(&response, "req-delete-success", true);
        assert!(error.is_none());

        let row = wait_for_audit_row(&db, AuditActionType::SERVICE_CONFIG_DELETE).await;
        assert_eq!(AuditActionType::SERVICE_CONFIG_DELETE, row.action_type);
        assert_eq!(row.outcome, AuditOutcome::Success.as_str());
        assert_eq!(row.actor_type, AuditActorType::Service.as_str());
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.tenant_id, tenant_id);
        assert_eq!(row.target_type.as_deref(), Some("service_config"));
        assert_eq!(row.target_display.as_deref(), Some("clients.primary"));
        assert_eq!(row.request_id.as_deref(), Some("req-delete-success"));
        let details = row
            .details_json
            .expect("delete audit should include details");
        assert_eq!(details["service_app_name"], "uptrakit-mqtt");
        assert_eq!(details["scope"], "tenant");
        // before_snapshot has the old state, after_snapshot is {}.
        let before = row.before_snapshot.expect("before_snapshot");
        assert_eq!(before["key"], "clients.primary");
        let after = row.after_snapshot.expect("after_snapshot");
        assert_eq!(after, serde_json::json!({}));
    }

    #[tokio::test]
    async fn handle_delete_service_config_not_found_emits_denied_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let tenant_id_opt = Some(tenant_id);
        let service_id = uuid::Uuid::now_v7();
        let payload = DeleteServiceConfigPayload::new(
            "req-delete-not-found".to_string(),
            tenant_id_opt,
            "clients.primary".to_string(),
        );

        let response =
            handle_delete_service_config(&state, "uptrakit-mqtt", service_id, payload).await;
        // Idempotent delete still returns success ACK.
        let error = assert_ack(&response, "req-delete-not-found", true);
        assert!(error.is_none());

        // The denied event is emitted via the dispatcher (fire-and-forget), which
        // also writes to the DB backend in the test harness.
        let row = wait_for_audit_row(&db, AuditActionType::SERVICE_CONFIG_DELETE).await;
        assert_eq!(row.outcome, AuditOutcome::Denied.as_str());
        let details = row
            .details_json
            .expect("denied audit should include details");
        assert_eq!(details["reason_code"], "not_found");
    }
}
