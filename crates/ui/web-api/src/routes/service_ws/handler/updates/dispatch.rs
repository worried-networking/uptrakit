//! Dispatch of successor updates (batch + queue) and reconnect-failure notify.

use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uptrakit_shared_db::entity::{service, update_history};
use uptrakit_web_api_types::events::AdminEvent;

use super::{
    ReconnectSuccessorDispatchMode, ReplayPreparationNotifier, emit_batch_progress_from_db,
    handle_batch_completion,
};
use crate::AppState;

/// Notify all subscribers about a single update that was marked failed on reconnect.
///
/// Sends the output-stream completion, broadcasts the SSE event, and dispatches
/// the next update in the queue (batch or standalone).
pub(super) async fn notify_failed_reconnect_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    record: &update_history::Model,
    reason: &str,
    successor_dispatch_mode: ReconnectSuccessorDispatchMode,
) {
    tracing::warn!(
        update_id = %record.id,
        host_id = %record.host_id,
        "in-progress update marked interrupted due to agent restart"
    );

    state
        .broadcast
        .update_output_broadcaster
        .send_completed(
            record.id,
            "interrupted".to_string(),
            Some(reason.to_string()),
        )
        .await;

    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            AdminEvent::UpdateCompleted {
                update_history_id: record.id,
                host_id: record.host_id,
                software_item_id: record.software_item_id,
                status: "interrupted".to_string(),
            },
        )
        .await;

    match successor_dispatch_mode {
        ReconnectSuccessorDispatchMode::Immediate => {
            if let Some(batch_id) = record.batch_id {
                dispatch_next_batch_update(state, service_id, batch_id, record.host_id).await;
            } else {
                dispatch_next_queued_update(state, service_id, record.host_id).await;
            }
        }
        ReconnectSuccessorDispatchMode::ReplayPrepared => {
            if let Some(batch_id) = record.batch_id {
                dispatch_next_batch_update_for_replay(state, service_id, batch_id, record.host_id)
                    .await;
            } else {
                dispatch_next_queued_update_for_replay(state, service_id, record.host_id).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Batch dispatch helper
// ---------------------------------------------------------------------------

/// Dispatch the next pending update within a batch for the given host.
///
/// Resolves the service's tenant_id, calls `dispatch_next_in_batch`, and logs
/// any errors without failing the calling handler. If the batch just completed,
/// dispatches a notification event.
pub(crate) async fn dispatch_next_batch_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    batch_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    dispatch_next_batch_update_with_notifier(
        state,
        service_id,
        batch_id,
        host_id,
        &state.notification.notification_service,
        state.controller_update_protection(),
    )
    .await;
}

async fn dispatch_next_batch_update_for_replay(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    batch_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    let notifier = ReplayPreparationNotifier;
    dispatch_next_batch_update_with_notifier(
        state,
        service_id,
        batch_id,
        host_id,
        &notifier,
        state.controller_update_protection(),
    )
    .await;
}

async fn dispatch_next_batch_update_with_notifier(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    batch_id: uuid::Uuid,
    host_id: uuid::Uuid,
    notifier: &dyn crate::ServiceNotifier,
    protection: Option<
        Arc<dyn uptrakit_plugin_infrastructure_registry::ControllerUpdateProtection>,
    >,
) {
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        _ => return,
    };

    match crate::queries::update_batches::dispatch_next_in_batch(
        state.db(),
        crate::queries::update_dispatch::DispatchContext {
            notifier,
            protection,
            #[cfg(feature = "plugin-ops")]
            hook: state.controller_update_hook(),
            #[cfg(feature = "plugin-ops")]
            notification_ops: Some(state.plugin.plugin_ops.as_ref()),
        },
        batch_id,
        host_id,
        tenant_id,
    )
    .await
    {
        Ok(Some(completion)) => {
            handle_batch_completion(state, batch_id, &completion).await;
        }
        Ok(None) => {
            // Batch still in progress — emit updated progress summary.
            emit_batch_progress_from_db(state, batch_id).await;
        }
        Err(e) => {
            tracing::warn!(
                %batch_id,
                %host_id,
                error = %e,
                "failed to dispatch next batch update or update batch status"
            );
        }
    }
}

/// Promote the next Queued update for the given host to Pending during reconnect
/// replay preparation, without spawning the orchestrator.
///
/// Called from `notify_failed_reconnect_update` when dispatching under
/// `ReplayPrepared` mode. The outer `prepare_pending_replay_messages` loop
/// will pick up the newly-Pending record on its next iteration and hand it
/// off to the orchestrator.
async fn dispatch_next_queued_update_for_replay(
    state: &Arc<AppState>,
    _service_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    // CAS: Queued -> Pending. Do NOT spawn the orchestrator — the outer
    // prepare_pending_replay_messages loop handles that on retry.
    let next = match update_history::Entity::find()
        .filter(update_history::Column::HostId.eq(host_id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
        .order_by_asc(update_history::Column::Id)
        .one(state.db())
        .await
    {
        Ok(Some(record)) => record,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(
                %host_id,
                error = %e,
                "failed to query next queued update during replay recovery"
            );
            return;
        }
    };

    if let Err(e) = update_history::Entity::update_many()
        .filter(update_history::Column::Id.eq(next.id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
        .col_expr(
            update_history::Column::Status,
            sea_orm::sea_query::Expr::value(update_history::UpdateStatus::Pending),
        )
        .exec(state.db())
        .await
    {
        tracing::warn!(
            update_id = %next.id,
            %host_id,
            error = %e,
            "failed to promote queued update to Pending during replay recovery"
        );
    }
}

/// Dispatch the next queued update for the given host after a non-batch
/// update completes.
///
/// Finds the next Queued record for the host, CAS-promotes it to Pending,
/// loads the dispatch target, and spawns the orchestrator for protection + dispatch.
pub(super) async fn dispatch_next_queued_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    dispatch_next_queued_update_with_notifier(state, service_id, host_id).await;
}

/// Mark a queued update as `Failed` after its dispatch target could not be
/// loaded. Best-effort: a secondary update error is logged, not propagated.
async fn fail_dispatch_target_load(
    state: &Arc<AppState>,
    next: &update_history::Model,
    host_id: uuid::Uuid,
    error: impl std::fmt::Display,
) {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};

    tracing::warn!(
        update_id = %next.id,
        %host_id,
        error = %error,
        "failed to load dispatch data for queued update, marking as failed"
    );
    let mut active: update_history::ActiveModel = next.clone().into();
    active.status = Set(update_history::UpdateStatus::Failed);
    active.completed_at = Set(Some(time::OffsetDateTime::now_utc()));
    let output = format!("dispatch failed: {error}");
    active.output_bytes = Set(output.len() as i64);
    active.output = Set(output);
    if let Err(upd_err) = active.update(state.db()).await {
        tracing::warn!(
            update_id = %next.id,
            error = %upd_err,
            "failed to mark queued update as failed after load_target_for_dispatch error"
        );
    }
}

async fn dispatch_next_queued_update_with_notifier(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        _ => return,
    };

    loop {
        // Find the oldest Queued update for this host.
        let next = match update_history::Entity::find()
            .filter(update_history::Column::HostId.eq(host_id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
            .order_by_asc(update_history::Column::Id)
            .one(state.db())
            .await
        {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(
                    %host_id,
                    error = %e,
                    "failed to query next queued update for host"
                );
                return;
            }
        };

        // CAS: Queued -> Pending.
        let cas_result = match update_history::Entity::update_many()
            .filter(update_history::Column::Id.eq(next.id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
            .col_expr(
                update_history::Column::Status,
                sea_orm::sea_query::Expr::value(update_history::UpdateStatus::Pending),
            )
            .exec(state.db())
            .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(
                    update_id = %next.id,
                    %host_id,
                    error = %e,
                    "CAS Queued->Pending failed for queued update"
                );
                return;
            }
        };

        if cas_result.rows_affected == 0 {
            tracing::debug!(
                update_id = %next.id,
                %host_id,
                "CAS missed: another controller already promoted this queued item, retrying"
            );
            continue;
        }

        let target = match crate::queries::update_dispatch::load_target_for_dispatch(
            state.db(),
            tenant_id,
            next.host_id,
            next.software_item_id,
        )
        .await
        {
            Ok(target) => target,
            Err(e) => {
                fail_dispatch_target_load(state, &next, host_id, e).await;
                return;
            }
        };

        let work = crate::queries::update_triggers::PendingProtectionWork {
            target,
            update_history_id: next.id,
            to_version: next.to_version.clone().unwrap_or_default(),
            release_info: None,
            interactive: next.interactive,
        };
        state.update_dispatcher.spawn_pending_protection(work);
        return;
    }
}
