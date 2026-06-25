//! Production implementation of [`UpdateDispatcher`] that runs pre-update
//! protection, then dispatches to the agent.
//!
//! Logic is ported from the now-deleted `update_orchestrator.rs` and
//! `crates/ui/web-api/src/routes/software_items/mod.rs` with AppState
//! references replaced by explicit dependency parameters.

use std::sync::Arc;

use async_trait::async_trait;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use uptrakit_audit_log::{AuditActionType, AuditActorType, AuditEntry, AuditOutcome};
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_shared_db::entity::update_history;
use uptrakit_shared_types::OutputStreamType;
use uptrakit_web_api_queries::queries::update_dispatch::{
    DispatchUpdateParams, PreUpdateProtectionOutcome, consolidate_protection_output,
    dispatch_update_to_agent, fail_before_agent_dispatch, insert_protection_output_line,
    prepare_pre_update_protection, set_inprogress_for_orchestrator,
};
use uptrakit_web_api_queries::queries::update_triggers::{
    PendingProtectionWork, TriggerUpdateParams, trigger_update_for_host,
};
use uptrakit_web_api_queries::queries::update_types::ActorType;
use uptrakit_web_api_types::software_items::TriggerUpdateStatus;
use uptrakit_wire::AdminEvent;
use uuid::Uuid;

use crate::notification::NotificationState;

use super::{
    DispatchOutcome, UpdateDispatchError, UpdateDispatchParams, UpdateDispatchResult,
    UpdateDispatcher, UpdateOutputStream,
};

#[cfg(feature = "plugin-ops")]
use uptrakit_web_api_queries::queries::update_dispatch::prepare_pre_update_hook;

/// Production dispatcher that runs the full protection + agent dispatch pipeline.
///
/// Holds all external dependencies explicitly so it can be constructed without
/// an `AppState` reference. Used by both `web-api` and `mcp` crates.
#[non_exhaustive]
pub struct ControllerUpdateDispatcher {
    db: sea_orm::DatabaseConnection,
    notification: NotificationState,
    output_stream: Arc<dyn UpdateOutputStream>,
    plugin_ops: Arc<dyn PluginOps>,
    audit_emitter: uptrakit_audit_log::AuditEmitter,
}

impl ControllerUpdateDispatcher {
    /// Construct a new dispatcher.
    pub fn new(
        db: sea_orm::DatabaseConnection,
        notification: NotificationState,
        output_stream: Arc<dyn UpdateOutputStream>,
        plugin_ops: Arc<dyn PluginOps>,
        audit_emitter: uptrakit_audit_log::AuditEmitter,
    ) -> Self {
        Self {
            db,
            notification,
            output_stream,
            plugin_ops,
            audit_emitter,
        }
    }
}

#[async_trait]
impl UpdateDispatcher for ControllerUpdateDispatcher {
    #[tracing::instrument(skip_all, fields(
        tenant_id = %params.tenant_id,
        host_id = %params.host_id,
        software_item_id = %params.software_item_id,
    ))]
    async fn dispatch(
        &self,
        params: UpdateDispatchParams,
    ) -> Result<UpdateDispatchResult, rootcause::Report<UpdateDispatchError>> {
        // Deserialise the optional release_info JSON into the wire type.
        let release_info: Option<uptrakit_wire::ReleaseInfo> = match &params.release_info {
            Some(v) => match serde_json::from_value(v.clone()) {
                Ok(ri) => Some(ri),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to deserialise release_info — proceeding without it"
                    );
                    None
                }
            },
            None => None,
        };

        let trigger_result = trigger_update_for_host(
            &self.db,
            TriggerUpdateParams {
                tenant_id: params.tenant_id,
                item_id: params.software_item_id,
                host_id: params.host_id,
                to_version: params.to_version.clone(),
                actor_type: params.actor.actor_type,
                actor_id: params.actor.actor_id.clone(),
                release_info,
                interactive: params.interactive,
            },
        )
        .await;

        let trigger_result = match trigger_result {
            Ok(r) => r,
            Err(e) => {
                let (outcome, reason_code) = e.current_context().trigger_audit_classification();
                emit_update_audit(
                    &self.audit_emitter,
                    &params,
                    params.software_item_id,
                    outcome,
                    serde_json::json!({
                        "host_id": params.host_id,
                        "to_version": params.to_version,
                        "interactive": params.interactive,
                        "reason_code": reason_code,
                    }),
                );
                return Err(e.context_transform(map_trigger_error));
            }
        };

        // Fix 3: Pending → Queued (agent offline, will deliver on reconnect).
        // Sent is only for confirmed agent delivery (not returned by trigger_update_for_host).
        let outcome = match trigger_result.initial_status {
            update_history::UpdateStatus::Pending => DispatchOutcome::Queued,
            update_history::UpdateStatus::Queued => DispatchOutcome::Queued,
            update_history::UpdateStatus::Failed => DispatchOutcome::Failed,
            _ => {
                tracing::warn!("unexpected UpdateStatus after trigger; defaulting to Queued");
                DispatchOutcome::Queued
            }
        };

        // Fix 4: Use TriggerUpdateStatus for the audit dispatch_status string
        // ("pending", "queued", "failed") — matches the web-api route behaviour.
        let dispatch_status_str = match trigger_result.initial_status {
            update_history::UpdateStatus::Pending => TriggerUpdateStatus::Pending.to_string(),
            update_history::UpdateStatus::Failed => TriggerUpdateStatus::Failed.to_string(),
            _ => {
                tracing::warn!(
                    status = ?trigger_result.initial_status,
                    "unexpected initial_status after trigger; defaulting dispatch_status to queued"
                );
                TriggerUpdateStatus::Queued.to_string()
            }
        };

        emit_update_audit(
            &self.audit_emitter,
            &params,
            params.software_item_id,
            classify_dispatch_audit_outcome(trigger_result.initial_status),
            serde_json::json!({
                "host_id": params.host_id,
                "to_version": params.to_version,
                "interactive": params.interactive,
                "update_history_id": trigger_result.update_history_id,
                "dispatch_status": dispatch_status_str,
            }),
        );

        // Fix 2: Push software states for all initial statuses (not just Pending).
        // The web-api trigger_update action always calls push_software_states_for_tenant
        // immediately after trigger_update_for_host succeeds.
        self.notification
            .notification_service
            .push_software_states_for_tenant(&self.db, params.tenant_id)
            .await;

        // Fix 1: Emit AdminEvent::UpdateTriggered so SSE subscribers can reflect
        // the new pending/queued entry in real-time without polling.
        self.notification
            .event_broadcaster
            .send(
                params.tenant_id,
                AdminEvent::UpdateTriggered {
                    update_history_id: trigger_result.update_history_id,
                    host_id: params.host_id,
                    software_item_id: params.software_item_id,
                    status: dispatch_status_str.clone(),
                },
            )
            .await;

        if let Some(work) = trigger_result.pending_protection_work {
            self.spawn_protection_and_dispatch(*work);
        }

        Ok(UpdateDispatchResult {
            update_history_id: trigger_result.update_history_id,
            outcome,
        })
    }

    fn spawn_pending_protection(&self, work: PendingProtectionWork) {
        self.spawn_protection_and_dispatch(work);
    }
}

// ---------------------------------------------------------------------------
// Spawn helper (impl method)
// ---------------------------------------------------------------------------

impl ControllerUpdateDispatcher {
    /// Spawn a background task that runs pre-update protection then dispatches
    /// to the agent.
    fn spawn_protection_and_dispatch(&self, work: PendingProtectionWork) {
        let db = self.db.clone();
        let notification = self.notification.clone();
        let output_stream = self.output_stream.clone();
        let plugin_ops = self.plugin_ops.clone();
        tokio::spawn(run_protection_and_dispatch(
            db,
            notification,
            output_stream,
            plugin_ops,
            work,
        ));
    }
}

// ---------------------------------------------------------------------------
// Core orchestration
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(update_id = %work.update_history_id))]
pub async fn run_protection_and_dispatch(
    db: sea_orm::DatabaseConnection,
    notification: NotificationState,
    output_stream: Arc<dyn UpdateOutputStream>,
    plugin_ops: Arc<dyn PluginOps>,
    work: PendingProtectionWork,
) {
    let update_history_id = work.update_history_id;
    let tenant_id = work.target.item.tenant_id;
    let host_id = work.target.host.id;
    let software_item_id = work.target.item.id;

    // 1. Check agent connectivity — if the agent is offline there is nothing to
    //    dispatch yet. Leave the record as Pending for reconnect recovery.
    if !notification
        .notification_service
        .registry()
        .is_connected(&work.target.agent.id)
        .await
    {
        tracing::debug!(
            update_id = %update_history_id,
            agent_id = %work.target.agent.id,
            "agent offline at orchestration time — leaving record Pending for reconnect recovery"
        );
        return;
    }

    // 2. CAS Pending → InProgress. If the row was already claimed by another
    //    controller or is no longer Pending, bail out silently.
    let rows = match set_inprogress_for_orchestrator(&db, update_history_id).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(
                update_id = %update_history_id,
                error = %e,
                "set_inprogress_for_orchestrator query failed"
            );
            return;
        }
    };

    if rows == 0 {
        tracing::debug!(
            update_id = %update_history_id,
            "CAS Pending→InProgress matched 0 rows — record already claimed or gone"
        );
        return;
    }

    // 3. Create the broadcast channel for real-time output streaming.
    output_stream.create_channel(update_history_id).await;

    // 4. Push MQTT software states so the UI transitions to in_progress.
    notification
        .notification_service
        .push_software_states_for_tenant(&db, tenant_id)
        .await;

    // 5. Emit AdminEvent::UpdateProtectionStarted for SSE subscribers.
    notification
        .event_broadcaster
        .send(
            tenant_id,
            AdminEvent::UpdateProtectionStarted {
                update_history_id,
                host_id,
                software_item_id,
            },
        )
        .await;

    // 6. Create mpsc channel for streaming protection output lines.
    let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // 7. Spawn forwarder task.
    let output_stream_for_fwd = output_stream.clone();
    let forwarder = tokio::spawn(forward_protection_output(
        db.clone(),
        output_stream_for_fwd,
        update_history_id,
        rx,
    ));

    // 8. Run pre-update protection.
    let protection = plugin_ops.controller_update_protection();
    // Clone the sender so the hook step (9a) can also stream output after
    // protection completes.
    #[cfg(feature = "plugin-ops")]
    let hook_tx = tx.clone();
    let outcome = match prepare_pre_update_protection(
        &db,
        protection,
        &work.target,
        update_history_id,
        Some(tx),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            tracing::warn!(
                update_id = %update_history_id,
                error = %e,
                "prepare_pre_update_protection returned an error; marking update failed"
            );
            if let Err(fail_err) =
                fail_before_agent_dispatch(&db, update_history_id, None, None).await
            {
                tracing::warn!(
                    update_id = %update_history_id,
                    error = %fail_err,
                    "fail_before_agent_dispatch also failed after prepare_pre_update_protection error"
                );
            }
            #[cfg(feature = "plugin-ops")]
            drop(hook_tx);
            join_and_consolidate(forwarder, db.clone(), update_history_id).await;
            output_stream
                .send_completed(update_history_id, DispatchOutcome::Failed, None)
                .await;
            notification
                .notification_service
                .push_software_states_for_tenant(&db, tenant_id)
                .await;
            return;
        }
    };

    // 9. Handle outcome.
    match outcome {
        PreUpdateProtectionOutcome::Failed => {
            // Protection failed — record already marked Failed by the query.
            #[cfg(feature = "plugin-ops")]
            drop(hook_tx);
            join_and_consolidate(forwarder, db.clone(), update_history_id).await;
            output_stream
                .send_completed(update_history_id, DispatchOutcome::Failed, None)
                .await;
            notification
                .notification_service
                .push_software_states_for_tenant(&db, tenant_id)
                .await;
        }
        PreUpdateProtectionOutcome::Proceed => {
            // 9a. Run pre-update hook (resource scaling) — fires after protection
            //     succeeds and before the agent receives the dispatch message.
            #[cfg(feature = "plugin-ops")]
            prepare_pre_update_hook(
                &db,
                plugin_ops.controller_update_hook(),
                &work.target,
                update_history_id,
                Some(hook_tx),
            )
            .await;

            // 10. Dispatch to agent.
            let notifier = &notification.notification_service;
            let dispatch_result = dispatch_update_to_agent(
                notifier,
                &work.target,
                DispatchUpdateParams {
                    update_history_id,
                    to_version: work.to_version,
                    release_info: work.release_info,
                    interactive: work.interactive,
                },
            )
            .await;

            match dispatch_result {
                Ok(true) => {
                    // Agent was connected and the message was sent.
                }
                Ok(false) => {
                    // Agent disconnected between the connectivity check and
                    // dispatch. Leave the record in InProgress for the
                    // reconnect recovery path.
                    tracing::debug!(
                        update_id = %update_history_id,
                        agent_id = %work.target.agent.id,
                        "agent disconnected before dispatch — leaving InProgress for reconnect recovery"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        update_id = %update_history_id,
                        error = %e,
                        "dispatch_update_to_agent failed"
                    );
                    if let Err(fail_err) =
                        fail_before_agent_dispatch(&db, update_history_id, None, None).await
                    {
                        tracing::warn!(
                            update_id = %update_history_id,
                            error = %fail_err,
                            "fail_before_agent_dispatch also failed after dispatch error"
                        );
                    }
                    join_and_consolidate(forwarder, db.clone(), update_history_id).await;
                    output_stream
                        .send_completed(update_history_id, DispatchOutcome::Failed, None)
                        .await;
                    notification
                        .notification_service
                        .push_software_states_for_tenant(&db, tenant_id)
                        .await;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Protection output forwarder
// ---------------------------------------------------------------------------

async fn forward_protection_output(
    db: sea_orm::DatabaseConnection,
    output_stream: Arc<dyn UpdateOutputStream>,
    update_history_id: Uuid,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(raw) = rx.recv().await {
        let text = String::from_utf8_lossy(&raw).into_owned();
        let line_id = Uuid::now_v7();
        let timestamp = OffsetDateTime::now_utc();

        if let Err(e) = insert_protection_output_line(
            &db,
            update_history_id,
            line_id,
            text.clone(),
            OutputStreamType::Stdout,
            timestamp,
        )
        .await
        {
            tracing::warn!(
                update_id = %update_history_id,
                error = %e,
                "failed to persist protection output line"
            );
        }

        output_stream
            .send_line(
                update_history_id,
                line_id,
                text,
                OutputStreamType::Stdout,
                timestamp,
            )
            .await;
    }
}

// ---------------------------------------------------------------------------
// Join + consolidate helper
// ---------------------------------------------------------------------------

/// Join the output forwarder, then write the streamed protection lines into the
/// authoritative `update_history.output` column.
///
/// Call ONLY after every channel sender (`tx`, and the cfg-gated `hook_tx`) has
/// been dropped — otherwise `forwarder.await` hangs forever (the channel never
/// closes). Autocommit is safe: joining the forwarder first guarantees no
/// concurrent writer on `update_output_lines`.
async fn join_and_consolidate(
    forwarder: tokio::task::JoinHandle<()>,
    db: sea_orm::DatabaseConnection,
    update_history_id: Uuid,
) {
    if let Err(e) = forwarder.await {
        tracing::warn!(update_id = %update_history_id, error = %e, "protection output forwarder join failed");
    }
    if let Err(e) = consolidate_protection_output(&db, update_history_id).await {
        tracing::warn!(update_id = %update_history_id, error = %e, "consolidating protection output failed");
    }
}

// ---------------------------------------------------------------------------
// Audit helpers
// ---------------------------------------------------------------------------

/// Emit a `SOFTWARE_UPDATE_TRIGGERED` audit entry for dispatcher calls.
///
/// Maps `ActorType` to `AuditActorType`:
/// - `User` / `ApiToken` → UUID parsed from `actor_id` string.
/// - `Service` / `Mqtt` → Service actor with UUID parsed from `actor_id` string.
/// - `Scheduler` / `SystemService` / `System` → System actor (no `actor_id`).
fn emit_update_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
    params: &UpdateDispatchParams,
    item_id: Uuid,
    outcome: AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = actor_audit_pair(&params.actor.actor_type, &params.actor.actor_id);
    let entry = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    )
    .tenant_scope(params.tenant_id)
    .actor(actor_type, actor_id)
    .target("software_item", item_id.to_string(), None)
    .outcome(outcome)
    .details(details)
    .build();

    match entry {
        Ok(e) => audit_emitter.emit_event(e),
        Err(err) => {
            tracing::warn!(
                error = %err,
                %item_id,
                "failed to build software update audit entry"
            );
        }
    }
}

/// Map `ActorType` and its string actor_id to the audit actor pair.
fn actor_audit_pair(actor_type: &ActorType, actor_id_str: &str) -> (AuditActorType, Option<Uuid>) {
    match actor_type {
        ActorType::User => {
            let id = actor_id_str.parse::<Uuid>().ok();
            (AuditActorType::User, id)
        }
        ActorType::ApiToken => {
            let id = actor_id_str.parse::<Uuid>().ok();
            (AuditActorType::ApiToken, id)
        }
        ActorType::Scheduler => (AuditActorType::System, None),
        // Service-originated writes carry a Service UUID in `actor_id` (see
        // `ActorType::from_service_app_name` in web-api-queries). The MQTT Service is the same
        // family with a legacy on-disk spelling, so both map to the audit `Service` actor.
        ActorType::Service | ActorType::Mqtt => {
            let id = actor_id_str.parse::<Uuid>().ok();
            (AuditActorType::Service, id)
        }
        // Instance-wide system paths (no per-Service identity) map to the audit `System` actor.
        ActorType::SystemService | ActorType::System => (AuditActorType::System, None),
    }
}

/// Map an `UpdateStatus` to an `AuditOutcome` for the success path.
fn classify_dispatch_audit_outcome(status: update_history::UpdateStatus) -> AuditOutcome {
    match status {
        update_history::UpdateStatus::Failed => AuditOutcome::Failed,
        _ => AuditOutcome::Success,
    }
}

/// Map a `TriggerUpdateError` to an `UpdateDispatchError`.
///
/// Used with `context_transform` to convert the error type while preserving
/// the rootcause report chain.
fn map_trigger_error(
    err: uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError,
) -> UpdateDispatchError {
    use uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError;
    match err {
        TriggerUpdateError::SoftwareItemNotFound => UpdateDispatchError::SoftwareItemNotFound,
        TriggerUpdateError::HostNotFound => UpdateDispatchError::HostNotFound,
        TriggerUpdateError::UpdateAlreadyActive => UpdateDispatchError::UpdateAlreadyActive,
        TriggerUpdateError::HostNotAssigned
        | TriggerUpdateError::NoExecuteUpdatePlugin
        | TriggerUpdateError::UnknownPluginType(_)
        | TriggerUpdateError::PluginConfigNotFound => UpdateDispatchError::NotConfigured,
        TriggerUpdateError::NoAgent | TriggerUpdateError::AgentNotApproved => {
            UpdateDispatchError::AgentUnavailable
        }
        TriggerUpdateError::PreUpdateProtection(_)
        | TriggerUpdateError::Database(_)
        | TriggerUpdateError::PostUpdateFinalization(_)
        | TriggerUpdateError::PostUpdateFinalizationTimeout => UpdateDispatchError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct NoopOutputStream;

    #[async_trait::async_trait]
    impl UpdateOutputStream for NoopOutputStream {
        async fn create_channel(&self, _update_id: Uuid) {}
        async fn send_line(
            &self,
            _update_id: Uuid,
            _line_id: Uuid,
            _text: String,
            _stream: uptrakit_shared_types::OutputStreamType,
            _ts: OffsetDateTime,
        ) {
        }
        async fn send_completed(
            &self,
            _update_id: Uuid,
            _outcome: DispatchOutcome,
            _error: Option<String>,
        ) {
        }
    }

    async fn make_sqlite_db() -> sea_orm::DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    // `start_paused = true` would be ideal here but is incompatible with SQLite:
    // sqlx's SQLite driver dispatches every query via `spawn_blocking`. Under a
    // paused clock, tokio sees no runnable *async* tasks while the blocking thread
    // runs, auto-advances virtual time, and fires the 5 s timeout before the
    // forwarder finishes — a false alarm. Instead we use a real 5 s wall-clock
    // guard: correct execution completes in <<1 s; a deadlock regression (sender
    // not dropped before `forwarder.await`) hangs and fails after 5 s real time.
    // That's still a deterministic regression signal — just not instant.
    #[tokio::test]
    async fn forwarder_join_completes_and_consolidates_after_senders_dropped() {
        use sea_orm::{ActiveModelTrait, EntityTrait};
        use uptrakit_shared_db::entity::{host, software_item, tenant, update_history};
        let db = make_sqlite_db().await;
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let id = Uuid::now_v7();
        // minimal FK parents
        tenant::ActiveModel {
            id: sea_orm::Set(tenant_id),
            name: sea_orm::Set("t".into()),
            slug: sea_orm::Set(format!("t-{tenant_id}")),
            is_default: sea_orm::Set(false),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            deactivated_at: sea_orm::Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        host::ActiveModel {
            id: sea_orm::Set(host_id),
            tenant_id: sea_orm::Set(tenant_id),
            machine_id: sea_orm::Set(format!("m-{host_id}")),
            hostname: sea_orm::Set("h".into()),
            friendly_name: sea_orm::Set("H".into()),
            os_type: sea_orm::Set(None),
            os_version: sea_orm::Set(None),
            architecture: sea_orm::Set(None),
            ip_address: sea_orm::Set(None),
            host_features: sea_orm::Set(None),
            last_seen_at: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            deactivated_at: sea_orm::Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        software_item::ActiveModel {
            id: sea_orm::Set(software_item_id),
            tenant_id: sea_orm::Set(tenant_id),
            name: sea_orm::Set("s".into()),
            featured: sea_orm::Set(false),
            icon_url: sea_orm::Set(None),
            last_checked_at: sea_orm::Set(None),
            awaiting_restart_timeout: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            deactivated_at: sea_orm::Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        update_history::ActiveModel {
            id: sea_orm::Set(id),
            tenant_id: sea_orm::Set(tenant_id),
            host_id: sea_orm::Set(host_id),
            software_item_id: sea_orm::Set(software_item_id),
            host_software_item_id: sea_orm::Set(None),
            from_version: sea_orm::Set(None),
            to_version: sea_orm::Set(Some("1.0.0".into())),
            status: sea_orm::Set(update_history::UpdateStatus::Failed),
            output: sea_orm::Set(String::new()),
            output_bytes: sea_orm::Set(0),
            actor_type: sea_orm::Set("user".into()),
            actor_id: sea_orm::Set(String::new()),
            execution_owner_service_id: sea_orm::Set(None),
            execution_owner_instance_id: sea_orm::Set(None),
            started_at: sea_orm::Set(Some(now)),
            completed_at: sea_orm::Set(Some(now)),
            awaiting_restart_since: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            update_category: sea_orm::Set("security".into()),
            batch_id: sea_orm::Set(None),
            interactive: sea_orm::Set(false),
            output_truncated: sea_orm::Set(false),
            pre_update_protection_status: sea_orm::Set(Some("failed".into())),
            pre_update_protection_summary: sea_orm::Set(None),
            recovery_hint: sea_orm::Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let stream: std::sync::Arc<dyn UpdateOutputStream> = std::sync::Arc::new(NoopOutputStream);
        let forwarder = tokio::spawn(forward_protection_output(db.clone(), stream, id, rx));

        // Senders confined to a scope that closes before the join (the invariant).
        // `tx` is MOVED into the scope (`let tx = tx;`) so it is dropped at scope
        // exit — `UnboundedSender::send` takes `&self`, so without the move `tx`
        // would outlive the scope, the channel would never close, and the join
        // would deadlock (the very regression this test guards).
        {
            let tx = tx;
            let hook_tx = tx.clone();
            tx.send(b"line one\n".to_vec()).unwrap();
            hook_tx.send(b"line two\n".to_vec()).unwrap();
        } // tx + hook_tx dropped here -> channel closes

        // Must NOT hang: bound the join so a regression fails fast.
        tokio::time::timeout(
            Duration::from_secs(5),
            join_and_consolidate(forwarder, db.clone(), id),
        )
        .await
        .expect("join_and_consolidate must not deadlock");

        let row = update_history::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.output, "line one\nline two\n");
    }
}
