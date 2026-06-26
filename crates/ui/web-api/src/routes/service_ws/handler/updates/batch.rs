//! `BatchUpdateResult` handling, batch completion, and progress emission.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use time::OffsetDateTime;

use uptrakit_shared_db::entity::{host_software_item, service, update_history};
use uptrakit_shared_types::BatchStatus;
use uptrakit_wire::{BatchUpdateResultPayload, UpdateFinalStatus};

use super::super::shared_types::ProcessorResponse;
use super::audit::emit_batch_update_finalized_audit;
use super::finalize::finalize_post_update_best_effort;
use super::result::set_installed_version;
use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};

#[derive(Default)]
pub(super) struct BatchUpdateAuditSummary {
    pub(super) completed_count: u32,
    pub(super) failed_count: u32,
    pub(super) finalize_error_count: u32,
    pub(super) result_count: u32,
    pub(super) stale_count: u32,
}

impl BatchUpdateAuditSummary {
    fn is_total_success(&self) -> bool {
        self.result_count == 0
            || (self.completed_count == self.result_count
                && self.failed_count == 0
                && self.stale_count == 0
                && self.finalize_error_count == 0)
    }

    fn has_partial_signal(&self) -> bool {
        self.completed_count > 0
            || (self.failed_count > 0 && self.stale_count > 0)
            || self.finalize_error_count > 0
    }

    fn all_stale(&self) -> bool {
        self.result_count > 0 && self.stale_count == self.result_count
    }

    fn all_finalize_error(&self) -> bool {
        self.result_count > 0 && self.finalize_error_count == self.result_count
    }

    fn all_failed(&self) -> bool {
        self.result_count > 0 && self.failed_count == self.result_count
    }

    pub(super) fn outcome(&self) -> uptrakit_audit_log::AuditOutcome {
        if self.is_total_success() {
            uptrakit_audit_log::AuditOutcome::Success
        } else if self.has_partial_signal() {
            uptrakit_audit_log::AuditOutcome::Partial
        } else if self.stale_count == self.result_count {
            uptrakit_audit_log::AuditOutcome::Denied
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        }
    }

    pub(super) fn reason_code(&self) -> Option<&'static str> {
        if self.all_stale() {
            Some("not_owned")
        } else if self.all_finalize_error() {
            Some("finalization_error")
        } else if self.all_failed() {
            Some("agent_reported_failure")
        } else {
            None
        }
    }
}

enum BatchResultDisposition {
    Completed,
    Failed,
    FinalizeError,
    Stale,
}

/// Handle a completed batch: emit progress events, send completion, and
/// dispatch a notification if the batch finished or partially finished.
pub(in super::super) async fn handle_batch_completion(
    state: &Arc<AppState>,
    batch_id: uuid::Uuid,
    completion: &crate::queries::update_batches::BatchCompletionInfo,
) {
    // Emit final progress summary via broadcaster.
    emit_batch_progress_event(
        state,
        batch_id,
        crate::batch_progress_broadcaster::BatchProgressEvent::Progress {
            completed: completion.completed_count,
            failed: completion.failed_count,
            pending: 0,
            total: completion.total_count,
        },
    )
    .await;

    // Send batch completed event via broadcaster (removes the channel).
    state
        .broadcast
        .batch_progress_broadcaster
        .send_batch_completed(batch_id, completion.status.as_str().to_string())
        .await;

    let details = match completion.status {
        BatchStatus::Completed => NotificationEventDetails::BatchUpdateCompleted {
            batch_id: completion.batch_id,
            total_count: completion.total_count,
            completed_count: completion.completed_count,
        },
        BatchStatus::PartiallyCompleted => {
            NotificationEventDetails::BatchUpdatePartiallyCompleted {
                batch_id: completion.batch_id,
                total_count: completion.total_count,
                completed_count: completion.completed_count,
                failed_count: completion.failed_count,
            }
        }
        _ => return,
    };

    state
        .notification
        .notification_dispatcher
        .dispatch(NotificationEvent::new(completion.tenant_id, details));
}

// ---------------------------------------------------------------------------
// Batch progress helpers
// ---------------------------------------------------------------------------

/// Send a batch progress event to all SSE subscribers.
pub(in super::super) async fn emit_batch_progress_event(
    state: &Arc<AppState>,
    batch_id: uuid::Uuid,
    event: crate::batch_progress_broadcaster::BatchProgressEvent,
) {
    state
        .broadcast
        .batch_progress_broadcaster
        .send(batch_id, event)
        .await;
}

/// Compute and emit a progress summary from the DB for an in-progress batch.
pub(in super::super) async fn emit_batch_progress_from_db(
    state: &Arc<AppState>,
    batch_id: uuid::Uuid,
) {
    let batch = match update_history::Entity::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .all(state.db())
        .await
    {
        Ok(records) => records,
        Err(_) => return,
    };

    let total = batch.len() as i32;
    let mut completed: i64 = 0;
    let mut failed: i64 = 0;
    let mut pending: i64 = 0;

    for r in &batch {
        match r.status {
            update_history::UpdateStatus::Completed => completed += 1,
            // `Interrupted` is terminal (outcome unknown, non-success); bucket it
            // with failures, not pending.
            update_history::UpdateStatus::Failed | update_history::UpdateStatus::Interrupted => {
                failed += 1
            }
            update_history::UpdateStatus::Queued
            | update_history::UpdateStatus::Pending
            | update_history::UpdateStatus::InProgress
            | update_history::UpdateStatus::AwaitingRestart => {
                pending += 1;
            }
            _ => {
                tracing::warn!("Unknown update status {:?}, counting as pending", r.status);
                pending += 1;
            }
        }
    }

    emit_batch_progress_event(
        state,
        batch_id,
        crate::batch_progress_broadcaster::BatchProgressEvent::Progress {
            completed,
            failed,
            pending,
            total,
        },
    )
    .await;
}

// ---------------------------------------------------------------------------
// handle_batch_update_result
// ---------------------------------------------------------------------------

/// Process a single item result within a batch: validate ownership, persist
/// status/output, and update the installed version on success.
async fn process_single_batch_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    result: &uptrakit_wire::BatchUpdateItemResult,
    linked_host_ids: &HashSet<uuid::Uuid>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> BatchResultDisposition {
    let history_record = match update_history::Entity::find_by_id(result.update_history_id)
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(
                update_history_id = %result.update_history_id,
                "update_history record not found"
            );
            return BatchResultDisposition::Stale;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                update_history_id = %result.update_history_id,
                "failed to look up update_history"
            );
            return BatchResultDisposition::FinalizeError;
        }
    };

    if !linked_host_ids.contains(&history_record.host_id) {
        tracing::warn!(
            %service_id,
            update_history_id = %result.update_history_id,
            host_id = %history_record.host_id,
            "service attempted to update update_history for unlinked host"
        );
        return BatchResultDisposition::Stale;
    }

    let finalized = match crate::queries::update_batches::finalize_batch_item_if_owned(
        state.db(),
        crate::queries::update_batches::FinalizeBatchItemIfOwnedArgs {
            update_history_id: result.update_history_id,
            service_id,
            runtime_instance_id,
            status: result.status.clone(),
            error: result.error.clone(),
            output: result.output.clone(),
            installed_version: result.installed_version.clone(),
        },
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                error = %error,
                update_history_id = %result.update_history_id,
                "failed to finalize batch item"
            );
            return BatchResultDisposition::FinalizeError;
        }
    };

    if finalized == 0 {
        tracing::debug!(
            update_history_id = %result.update_history_id,
            "ignoring stale BatchUpdateResult item"
        );
        return BatchResultDisposition::Stale;
    }

    let mut finalized_record = history_record.clone();
    finalized_record.status = match result.status {
        UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
        _ => update_history::UpdateStatus::Failed,
    };
    finalized_record.completed_at = Some(OffsetDateTime::now_utc());
    finalized_record.output = if result.output.is_empty() {
        result.error.clone().unwrap_or_default()
    } else {
        result.output.clone()
    };
    finalized_record.output_bytes = finalized_record.output.len() as i64;
    finalize_post_update_best_effort(state, &finalized_record, None).await;

    // On success, update installed version by host_software_item ID.
    if result.status == UpdateFinalStatus::Completed
        && let Some(ref new_version) = result.installed_version
    {
        set_installed_version(
            state,
            sea_orm::Condition::all()
                .add(host_software_item::Column::Id.eq(result.host_software_item_id)),
            new_version,
        )
        .await;
    }

    if matches!(result.status, UpdateFinalStatus::Completed) {
        BatchResultDisposition::Completed
    } else {
        BatchResultDisposition::Failed
    }
}

/// Handle a `BatchUpdateResult` message: update per-item
/// `update_history` rows and `host_software_item.installed_version`
/// for successful items.
#[tracing::instrument(skip_all, fields(%service_id, batch_id = %payload.batch_id))]
pub(in super::super) async fn handle_batch_update_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: BatchUpdateResultPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::info!(
        batch_id = %payload.batch_id,
        results = payload.results.len(),
        "batch update result"
    );
    let mut audit_summary = BatchUpdateAuditSummary {
        result_count: payload.results.len() as u32,
        ..BatchUpdateAuditSummary::default()
    };

    for result in &payload.results {
        match process_single_batch_result(
            state,
            service_id,
            result,
            &linked_host_ids,
            runtime_instance_id,
        )
        .await
        {
            BatchResultDisposition::Completed => audit_summary.completed_count += 1,
            BatchResultDisposition::Failed => audit_summary.failed_count += 1,
            BatchResultDisposition::FinalizeError => audit_summary.finalize_error_count += 1,
            BatchResultDisposition::Stale => audit_summary.stale_count += 1,
        }
    }

    // Push updated software states to MQTT so that `in_progress = false`
    // and the new `installed_version` are reflected immediately after the batch
    // completes.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
        emit_batch_update_finalized_audit(
            state,
            service_id,
            svc.tenant_id,
            payload.batch_id,
            &audit_summary,
        )
        .await;
    }

    ProcessorResponse::cont()
}
