//! Batch progress: dispatch next update and batch completion detection.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{prelude::*, update_batch, update_history, update_output_line};
use uptrakit_shared_types::BatchStatus;
use uptrakit_wire::{OutputStreamType, UpdateFinalStatus};
use uuid::Uuid;

use crate::queries::update_dispatch::{
    DispatchContext, DispatchUpdateParams, PreUpdateProtectionOutcome, TriggerUpdateError,
    finalize_post_update, load_target_for_dispatch, prepare_pre_update_protection,
};

/// Information about a batch that just transitioned to a terminal status.
pub struct BatchCompletionInfo {
    pub batch_id: Uuid,
    pub tenant_id: Uuid,
    pub status: BatchStatus,
    pub total_count: i32,
    pub completed_count: i64,
    pub failed_count: i64,
}

/// Preloaded identity for update-start outcomes.
pub struct ClaimExecutionInfo {
    pub batch_id: Option<Uuid>,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub tenant_id: Uuid,
}

/// Result of attempting to claim or replay update execution.
pub enum ClaimExecutionOutcome {
    Claimed(ClaimExecutionInfo),
    Replay(ClaimExecutionInfo),
    Rejected,
}

/// Maximum stored output bytes per update (50 MB).
///
/// Must stay aligned with the WebSocket handler/API cap.
const UPDATE_OUTPUT_BYTES_CAP: i64 = 52_428_800;
const OUTPUT_TRUNCATION_NOTICE: &str = "\n[Output truncated: this update produced more than 50 MB of output. Only the first 50 MB is stored.]\n";

/// A persisted output line that is safe to fan out to subscribers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedUpdateOutputLine {
    pub id: Uuid,
    pub stream: OutputStreamType,
    pub output: String,
    pub created_at: OffsetDateTime,
}

/// Result of attempting to persist streaming update output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendUpdateOutputOutcome {
    Ignored,
    Appended(PersistedUpdateOutputLine),
    TruncationNotice(PersistedUpdateOutputLine),
    AppendedWithTruncation {
        line: PersistedUpdateOutputLine,
        notice: PersistedUpdateOutputLine,
    },
}

impl AppendUpdateOutputOutcome {
    pub fn is_truncation_notice(&self) -> bool {
        matches!(
            self,
            Self::TruncationNotice(_) | Self::AppendedWithTruncation { .. }
        )
    }

    pub fn into_persisted_lines(self) -> Vec<PersistedUpdateOutputLine> {
        match self {
            Self::Ignored => Vec::new(),
            Self::Appended(line) | Self::TruncationNotice(line) => vec![line],
            Self::AppendedWithTruncation { line, notice } => vec![line, notice],
        }
    }
}

fn truncate_to_char_boundary(output: &str, max_bytes: usize) -> &str {
    if output.len() <= max_bytes {
        return output;
    }

    let mut boundary = max_bytes;
    while boundary > 0 && !output.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &output[..boundary]
}

/// Dispatches the next `Queued` update for the given host, across all batches
/// and non-batch updates (FIFO by `id`).
///
/// **Multi-controller safety:** Uses the same CAS pattern as
/// [`dispatch_next_in_batch`] -- the `Queued -> Pending` transition is
/// performed atomically. If another controller already promoted the row,
/// `rows_affected == 0` and the call exits without double-dispatching.
///
/// This is called:
/// 1. After a non-batch update completes on a host.
/// 2. By [`dispatch_next_in_batch`] to dequeue the next item (which may
///    belong to a different batch or to no batch at all).
#[tracing::instrument(skip_all, fields(%host_id))]
pub async fn dispatch_next_queued_for_host(
    db: &DatabaseConnection,
    dispatch: DispatchContext<'_>,
    host_id: Uuid,
    tenant_id: Uuid,
) -> std::result::Result<(), rootcause::Report<TriggerUpdateError>> {
    loop {
        // Find the oldest Queued update for this host (any batch or no batch).
        let next = UpdateHistory::find()
            .filter(update_history::Column::HostId.eq(host_id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
            .order_by_asc(update_history::Column::Id)
            .one(db)
            .await
            .context_to()?;

        let Some(next_record) = next else {
            return Ok(());
        };

        // CAS: Queued -> Pending.
        let cas_result = UpdateHistory::update_many()
            .filter(update_history::Column::Id.eq(next_record.id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
            .col_expr(
                update_history::Column::Status,
                Expr::value(update_history::UpdateStatus::Pending),
            )
            .exec(db)
            .await
            .context_to()?;

        if cas_result.rows_affected == 0 {
            tracing::debug!(
                update_id = %next_record.id,
                %host_id,
                "CAS missed: another controller already promoted this queued item, retrying"
            );
            continue;
        }

        let target = match load_target_for_dispatch(
            db,
            tenant_id,
            next_record.host_id,
            next_record.software_item_id,
        )
        .await
        {
            Ok(target) => target,
            Err(e) => {
                tracing::warn!(
                    update_id = %next_record.id,
                    %host_id,
                    error = %e,
                    "failed to load dispatch data for queued update, marking as failed"
                );
                let mut active: update_history::ActiveModel = next_record.clone().into();
                active.status = Set(update_history::UpdateStatus::Failed);
                active.completed_at = Set(Some(OffsetDateTime::now_utc()));
                let output = format!("dispatch failed: {e}");
                active.output = Set(output.clone());
                active.output_bytes = Set(output.len() as i64);
                active.update(db).await.context_to()?;
                if let Some(batch_id) = next_record.batch_id {
                    let _ = maybe_complete_batch(db, batch_id, next_record.tenant_id).await?;
                }
                continue;
            }
        };

        let pre_update_outcome = prepare_pre_update_protection(
            db,
            dispatch.protection.clone(),
            &target,
            next_record.id,
            None,
        )
        .await?;

        if matches!(pre_update_outcome, PreUpdateProtectionOutcome::Failed) {
            if let Some(batch_id) = next_record.batch_id {
                let _ = maybe_complete_batch(db, batch_id, next_record.tenant_id).await?;
            }
            continue;
        }

        let dispatch_result = super::super::update_dispatch::dispatch_update_to_agent(
            dispatch.notifier,
            &target,
            DispatchUpdateParams {
                update_history_id: next_record.id,
                to_version: next_record.to_version.clone().unwrap_or_default(),
                release_info: None,
                interactive: next_record.interactive,
            },
        )
        .await;

        match dispatch_result {
            Ok(_) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    update_id = %next_record.id,
                    %host_id,
                    error = %e,
                    "failed to dispatch queued update, marking as failed"
                );
                let failed_batch_id = next_record.batch_id;
                let failed_tenant_id = next_record.tenant_id;
                let mut active: update_history::ActiveModel = next_record.into();
                active.status = Set(update_history::UpdateStatus::Failed);
                active.completed_at = Set(Some(OffsetDateTime::now_utc()));
                let output = format!("dispatch failed: {e}");
                active.output = Set(output.clone());
                active.output_bytes = Set(output.len() as i64);
                active.update(db).await.context_to()?;
                if let Some(batch_id) = failed_batch_id {
                    let _ = maybe_complete_batch(db, batch_id, failed_tenant_id).await?;
                }
                continue;
            }
        }
    }
}

/// Called after an update completes in a batch. Dispatches the next queued
/// update for the same host (FIFO across all batches and non-batch updates),
/// and checks if the batch is done.
///
/// **Multi-controller safety:** Delegates to [`dispatch_next_queued_for_host`]
/// which performs the `Queued -> Pending` CAS atomically. If another controller
/// already promoted the same row, dispatch is skipped without double-dispatching.
///
/// Returns `Some(BatchCompletionInfo)` if the batch just transitioned to a
/// terminal status, so the caller can dispatch a notification event.
#[tracing::instrument(skip_all, fields(%batch_id, %host_id))]
pub async fn dispatch_next_in_batch(
    db: &DatabaseConnection,
    dispatch: DispatchContext<'_>,
    batch_id: Uuid,
    host_id: Uuid,
    tenant_id: Uuid,
) -> std::result::Result<Option<BatchCompletionInfo>, rootcause::Report<TriggerUpdateError>> {
    // Dispatch the next queued update for this host (FIFO across all batches
    // and non-batch updates). This supersedes the previous batch-scoped query
    // so that a queued non-batch update is not skipped when a batch completes.
    dispatch_next_queued_for_host(db, dispatch, host_id, tenant_id).await?;

    // Check if all items in this batch are now in a terminal state.
    maybe_complete_batch(db, batch_id, tenant_id).await
}

/// Check if all items in a batch are terminal and update batch status if so.
///
/// All reads and the batch status UPDATE are performed inside a single
/// database transaction so that concurrent completions cannot both observe
/// `pending_count == 0` and produce a double-write with an incorrect
/// `completed_at` timestamp.
///
/// Returns `Some(BatchCompletionInfo)` when the batch just transitioned to
/// a terminal status, `None` if still in progress.
async fn maybe_complete_batch(
    db: &DatabaseConnection,
    batch_id: Uuid,
    tenant_id: Uuid,
) -> std::result::Result<Option<BatchCompletionInfo>, rootcause::Report<TriggerUpdateError>> {
    let txn = db.begin().await.context_to()?;

    let pending_count = UpdateHistory::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Queued,
            update_history::UpdateStatus::Pending,
            update_history::UpdateStatus::InProgress,
        ]))
        .count(&txn)
        .await
        .context_to()?;

    if pending_count > 0 {
        // txn auto-rollbacks on drop; nothing to commit.
        return Ok(None);
    }

    // All items are terminal. Check if any failed.
    let failed_count = UpdateHistory::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Failed))
        .count(&txn)
        .await
        .context_to()? as i64;

    let completed_count = UpdateHistory::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Completed))
        .count(&txn)
        .await
        .context_to()? as i64;

    let new_status = if failed_count > 0 {
        BatchStatus::PartiallyCompleted
    } else {
        BatchStatus::Completed
    };

    let Some(batch) = UpdateBatch::find_by_id(batch_id)
        .one(&txn)
        .await
        .context_to()?
    else {
        return Ok(None);
    };

    // Guard against double-write: if the batch is already in a terminal
    // status another concurrent call already committed its update.
    if matches!(
        &batch.status,
        BatchStatus::Completed | BatchStatus::PartiallyCompleted
    ) {
        return Ok(None);
    }

    let total_count = batch.total_count;
    let mut active: update_batch::ActiveModel = batch.into();
    active.status = Set(new_status.clone());
    active.completed_at = Set(Some(OffsetDateTime::now_utc()));
    active.update(&txn).await.context_to()?;

    txn.commit().await.context_to()?;

    Ok(Some(BatchCompletionInfo {
        batch_id,
        tenant_id,
        status: new_status,
        total_count,
        completed_count,
        failed_count,
    }))
}

fn claim_execution_info(record: &update_history::Model) -> ClaimExecutionInfo {
    ClaimExecutionInfo {
        batch_id: record.batch_id,
        host_id: record.host_id,
        software_item_id: record.software_item_id,
        tenant_id: record.tenant_id,
    }
}

fn owned_in_progress_condition(service_id: Uuid, runtime_instance_id: Option<Uuid>) -> Condition {
    let cond = Condition::all()
        .add(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .add(update_history::Column::ExecutionOwnerServiceId.eq(service_id));

    match runtime_instance_id {
        Some(id) => cond.add(update_history::Column::ExecutionOwnerInstanceId.eq(id)),
        None => cond.add(update_history::Column::ExecutionOwnerInstanceId.is_null()),
    }
}

async fn load_owned_reconnect_candidates(
    db: &DatabaseConnection,
    service_id: Uuid,
    runtime_instance_id: Option<Uuid>,
) -> std::result::Result<Vec<update_history::Model>, rootcause::Report<TriggerUpdateError>> {
    let query = UpdateHistory::find()
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .filter(update_history::Column::ExecutionOwnerServiceId.eq(service_id));

    match runtime_instance_id {
        Some(current) => query
            .filter(
                Condition::any()
                    .add(update_history::Column::ExecutionOwnerInstanceId.is_null())
                    .add(update_history::Column::ExecutionOwnerInstanceId.ne(current)),
            )
            .all(db)
            .await
            .context_to(),
        None => query
            .filter(update_history::Column::ExecutionOwnerInstanceId.is_null())
            .all(db)
            .await
            .context_to(),
    }
}

/// Mark only updates owned by a previous runtime instance of the same service as failed.
pub async fn mark_owned_in_progress_as_failed_on_reconnect(
    db: &DatabaseConnection,
    service_id: Uuid,
    runtime_instance_id: Option<Uuid>,
) -> std::result::Result<Vec<update_history::Model>, rootcause::Report<TriggerUpdateError>> {
    let candidates = load_owned_reconnect_candidates(db, service_id, runtime_instance_id).await?;
    if candidates.is_empty() {
        return Ok(vec![]);
    }

    let txn = db.begin().await.context_to()?;
    let now = OffsetDateTime::now_utc();
    let reason = "Update interrupted: agent restarted";
    let mut failed = Vec::new();

    for record in candidates {
        let mut update = UpdateHistory::update_many()
            .filter(update_history::Column::Id.eq(record.id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
            .filter(update_history::Column::ExecutionOwnerServiceId.eq(service_id));

        update = match runtime_instance_id {
            Some(current) => update.filter(
                Condition::any()
                    .add(update_history::Column::ExecutionOwnerInstanceId.is_null())
                    .add(update_history::Column::ExecutionOwnerInstanceId.ne(current)),
            ),
            None => update.filter(update_history::Column::ExecutionOwnerInstanceId.is_null()),
        };

        let result = update
            .col_expr(
                update_history::Column::Status,
                Expr::value(update_history::UpdateStatus::Failed),
            )
            .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
            .col_expr(
                update_history::Column::Output,
                Expr::value(reason.to_string()),
            )
            .col_expr(
                update_history::Column::OutputBytes,
                Expr::value(reason.len() as i64),
            )
            .col_expr(update_history::Column::OutputTruncated, Expr::value(false))
            .exec(&txn)
            .await
            .context_to()?;

        if result.rows_affected == 1 {
            let mut updated_record = record;
            updated_record.status = update_history::UpdateStatus::Failed;
            updated_record.completed_at = Some(now);
            updated_record.output = reason.to_string();
            updated_record.output_bytes = reason.len() as i64;
            failed.push(updated_record);
        }
    }

    if !failed.is_empty() {
        let ids: Vec<Uuid> = failed.iter().map(|record| record.id).collect();
        UpdateOutputLine::delete_many()
            .filter(update_output_line::Column::UpdateHistoryId.is_in(ids))
            .exec(&txn)
            .await
            .context_to()?;
    }

    txn.commit().await.context_to()?;
    Ok(failed)
}

/// Rollout repair hook: fail every pre-existing in-progress row.
pub async fn mark_all_in_progress_as_failed_for_rollout(
    db: &DatabaseConnection,
) -> std::result::Result<Vec<update_history::Model>, rootcause::Report<TriggerUpdateError>> {
    let candidates = UpdateHistory::find()
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .all(db)
        .await
        .context_to()?;

    if candidates.is_empty() {
        return Ok(vec![]);
    }

    let txn = db.begin().await.context_to()?;
    let now = OffsetDateTime::now_utc();
    let reason = "Update interrupted: owner-aware rollout";
    let mut failed = Vec::new();

    for record in candidates {
        let result = UpdateHistory::update_many()
            .filter(update_history::Column::Id.eq(record.id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
            .col_expr(
                update_history::Column::Status,
                Expr::value(update_history::UpdateStatus::Failed),
            )
            .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
            .col_expr(
                update_history::Column::Output,
                Expr::value(reason.to_string()),
            )
            .col_expr(
                update_history::Column::OutputBytes,
                Expr::value(reason.len() as i64),
            )
            .col_expr(update_history::Column::OutputTruncated, Expr::value(false))
            .exec(&txn)
            .await
            .context_to()?;

        if result.rows_affected == 1 {
            let mut updated_record = record;
            updated_record.status = update_history::UpdateStatus::Failed;
            updated_record.completed_at = Some(now);
            updated_record.output = reason.to_string();
            updated_record.output_bytes = reason.len() as i64;
            failed.push(updated_record);
        }
    }

    if !failed.is_empty() {
        let ids: Vec<Uuid> = failed.iter().map(|record| record.id).collect();
        UpdateOutputLine::delete_many()
            .filter(update_output_line::Column::UpdateHistoryId.is_in(ids))
            .exec(&txn)
            .await
            .context_to()?;
    }

    txn.commit().await.context_to()?;
    Ok(failed)
}

/// Fail all orchestrator-owned InProgress records for the given hosts on agent reconnect.
///
/// Orchestrator-owned means `execution_owner_service_id IS NULL` + `status = InProgress`.
/// These records were mid-protection or mid-dispatch when the controller restarted.
/// The user must re-trigger; Proxmox protection will re-run.
///
/// Called after `mark_owned_in_progress_as_failed_on_reconnect` so agent-owned rows
/// are handled first.
pub async fn mark_orchestrator_inprogress_as_failed_on_reconnect(
    db: &DatabaseConnection,
    host_ids: &[Uuid],
) -> std::result::Result<(), rootcause::Report<TriggerUpdateError>> {
    if host_ids.is_empty() {
        return Ok(());
    }
    let now = OffsetDateTime::now_utc();
    let reason = "Protection interrupted: controller restarted";
    UpdateHistory::update_many()
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .filter(update_history::Column::ExecutionOwnerServiceId.is_null())
        .filter(update_history::Column::HostId.is_in(host_ids.to_vec()))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::Failed),
        )
        .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
        .col_expr(
            update_history::Column::Output,
            Expr::value(reason.to_string()),
        )
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(reason.len() as i64),
        )
        .col_expr(update_history::Column::OutputTruncated, Expr::value(false))
        .col_expr(
            update_history::Column::PreUpdateProtectionStatus,
            Expr::value(Some("failed".to_string())),
        )
        .exec(db)
        .await
        .context_to()?;
    Ok(())
}

/// Claim a pending update for execution or accept a same-owner replay.
pub async fn claim_or_replay_update_start_db(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    service_id: Uuid,
    runtime_instance_id: Option<Uuid>,
    interactive: bool,
) -> std::result::Result<ClaimExecutionOutcome, rootcause::Report<TriggerUpdateError>> {
    let Some(record) = UpdateHistory::find_by_id(update_history_id)
        .one(db)
        .await
        .context_to()?
    else {
        return Ok(ClaimExecutionOutcome::Rejected);
    };

    if record.status == update_history::UpdateStatus::Pending
        && record.execution_owner_service_id.is_none()
        && record.execution_owner_instance_id.is_none()
    {
        let started_at = OffsetDateTime::now_utc();
        let txn = db.begin().await.context_to()?;
        let claimed = UpdateHistory::update_many()
            .filter(update_history::Column::Id.eq(record.id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
            .filter(update_history::Column::ExecutionOwnerServiceId.is_null())
            .filter(update_history::Column::ExecutionOwnerInstanceId.is_null())
            .col_expr(
                update_history::Column::Status,
                Expr::value(update_history::UpdateStatus::InProgress),
            )
            .col_expr(
                update_history::Column::StartedAt,
                Expr::value(Some(started_at)),
            )
            .col_expr(
                update_history::Column::ExecutionOwnerServiceId,
                Expr::value(Some(service_id)),
            )
            .col_expr(
                update_history::Column::ExecutionOwnerInstanceId,
                Expr::value(runtime_instance_id),
            )
            .col_expr(
                update_history::Column::Interactive,
                Expr::value(interactive),
            )
            .col_expr(update_history::Column::Output, Expr::value(String::new()))
            .col_expr(update_history::Column::OutputBytes, Expr::value(0_i64))
            .col_expr(update_history::Column::OutputTruncated, Expr::value(false))
            .exec(&txn)
            .await
            .context_to()?;

        if claimed.rows_affected == 0 {
            txn.rollback().await.context_to()?;
            return Ok(ClaimExecutionOutcome::Rejected);
        }

        UpdateOutputLine::delete_many()
            .filter(update_output_line::Column::UpdateHistoryId.eq(record.id))
            .exec(&txn)
            .await
            .context_to()?;
        txn.commit().await.context_to()?;

        return Ok(ClaimExecutionOutcome::Claimed(claim_execution_info(
            &record,
        )));
    }

    // Orchestrator-owned InProgress: agent confirms an update whose record was
    // already transitioned by the orchestrator. Claim ownership atomically.
    if record.status == update_history::UpdateStatus::InProgress
        && record.execution_owner_service_id.is_none()
    {
        let txn = db.begin().await.context_to()?;
        let claimed = UpdateHistory::update_many()
            .filter(update_history::Column::Id.eq(record.id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
            .filter(update_history::Column::ExecutionOwnerServiceId.is_null()) // CAS guard
            .col_expr(
                update_history::Column::ExecutionOwnerServiceId,
                Expr::value(Some(service_id)),
            )
            .col_expr(
                update_history::Column::ExecutionOwnerInstanceId,
                Expr::value(runtime_instance_id),
            )
            .col_expr(
                update_history::Column::Interactive,
                Expr::value(interactive),
            )
            .exec(&txn)
            .await
            .context_to()?;

        if claimed.rows_affected == 0 {
            txn.rollback().await.context_to()?;
            return Ok(ClaimExecutionOutcome::Rejected);
        }

        txn.commit().await.context_to()?;
        // NOTE: No UpdateOutputLine::delete_many() — protection output lines are kept.
        // NOTE: started_at is NOT reset — it was set by set_inprogress_for_orchestrator.
        return Ok(ClaimExecutionOutcome::Claimed(claim_execution_info(
            &record,
        )));
    }

    if record.status == update_history::UpdateStatus::InProgress
        && record.execution_owner_service_id == Some(service_id)
        && record.execution_owner_instance_id == runtime_instance_id
    {
        return Ok(ClaimExecutionOutcome::Replay(claim_execution_info(&record)));
    }

    Ok(ClaimExecutionOutcome::Rejected)
}

/// Guarded post-start output append marker used by the handler layer.
pub async fn append_update_output_if_owned(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    service_id: Uuid,
    runtime_instance_id: Option<Uuid>,
    stream: OutputStreamType,
    output: &str,
) -> std::result::Result<AppendUpdateOutputOutcome, rootcause::Report<TriggerUpdateError>> {
    let txn = db.begin().await.context_to()?;
    let line_len = output.len() as i64;
    let Some(record) = UpdateHistory::find()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(owned_in_progress_condition(service_id, runtime_instance_id))
        .one(&txn)
        .await
        .context_to()?
    else {
        txn.rollback().await.context_to()?;
        return Ok(AppendUpdateOutputOutcome::Ignored);
    };

    if record.output_truncated {
        txn.rollback().await.context_to()?;
        return Ok(AppendUpdateOutputOutcome::Ignored);
    }

    let remaining_bytes = if record.output_bytes >= UPDATE_OUTPUT_BYTES_CAP {
        0
    } else {
        UPDATE_OUTPUT_BYTES_CAP - record.output_bytes
    };
    let stored_prefix = truncate_to_char_boundary(output, remaining_bytes as usize);

    if remaining_bytes > 0 && line_len <= remaining_bytes {
        let result = UpdateHistory::update_many()
            .filter(update_history::Column::Id.eq(update_history_id))
            .filter(owned_in_progress_condition(service_id, runtime_instance_id))
            .filter(update_history::Column::OutputTruncated.eq(false))
            .col_expr(
                update_history::Column::OutputBytes,
                Expr::col(update_history::Column::OutputBytes).add(line_len),
            )
            .exec(&txn)
            .await
            .context_to()?;

        if result.rows_affected != 1 {
            txn.rollback().await.context_to()?;
            return Ok(AppendUpdateOutputOutcome::Ignored);
        }

        let line = PersistedUpdateOutputLine {
            id: Uuid::now_v7(),
            stream,
            output: output.to_string(),
            created_at: OffsetDateTime::now_utc(),
        };

        UpdateOutputLine::insert(update_output_line::ActiveModel {
            id: Set(line.id),
            update_history_id: Set(update_history_id),
            stream: Set(line.stream),
            output: Set(line.output.clone()),
            created_at: Set(line.created_at),
        })
        .exec(&txn)
        .await
        .context_to()?;

        txn.commit().await.context_to()?;
        return Ok(AppendUpdateOutputOutcome::Appended(line));
    }

    let mark_result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(owned_in_progress_condition(service_id, runtime_instance_id))
        .filter(update_history::Column::OutputTruncated.eq(false))
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(record.output_bytes + stored_prefix.len() as i64),
        )
        .col_expr(update_history::Column::OutputTruncated, Expr::value(true))
        .exec(&txn)
        .await
        .context_to()?;

    if mark_result.rows_affected != 1 {
        txn.rollback().await.context_to()?;
        return Ok(AppendUpdateOutputOutcome::Ignored);
    }

    let appended_line = (!stored_prefix.is_empty()).then_some(PersistedUpdateOutputLine {
        id: Uuid::now_v7(),
        stream,
        output: stored_prefix.to_string(),
        created_at: OffsetDateTime::now_utc(),
    });

    if let Some(line) = &appended_line {
        UpdateOutputLine::insert(update_output_line::ActiveModel {
            id: Set(line.id),
            update_history_id: Set(update_history_id),
            stream: Set(line.stream),
            output: Set(line.output.clone()),
            created_at: Set(line.created_at),
        })
        .exec(&txn)
        .await
        .context_to()?;
    }

    let notice = PersistedUpdateOutputLine {
        id: Uuid::now_v7(),
        stream: OutputStreamType::System,
        output: OUTPUT_TRUNCATION_NOTICE.to_string(),
        created_at: OffsetDateTime::now_utc(),
    };

    UpdateOutputLine::insert(update_output_line::ActiveModel {
        id: Set(notice.id),
        update_history_id: Set(update_history_id),
        stream: Set(notice.stream),
        output: Set(notice.output.clone()),
        created_at: Set(notice.created_at),
    })
    .exec(&txn)
    .await
    .context_to()?;

    txn.commit().await.context_to()?;
    if let Some(line) = appended_line {
        return Ok(AppendUpdateOutputOutcome::AppendedWithTruncation { line, notice });
    }

    Ok(AppendUpdateOutputOutcome::TruncationNotice(notice))
}

/// Input bundle for guarded post-start finalization of a single update.
pub struct FinalizeUpdateResultIfOwnedArgs {
    pub update_history_id: Uuid,
    pub service_id: Uuid,
    pub runtime_instance_id: Option<Uuid>,
    pub status: UpdateFinalStatus,
    pub error: Option<String>,
    pub output: String,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
}

/// Guarded post-start finalization for a single update.
pub async fn finalize_update_result_if_owned(
    db: &DatabaseConnection,
    args: FinalizeUpdateResultIfOwnedArgs,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let final_output = if args.output.is_empty() {
        args.error.clone().unwrap_or_default()
    } else {
        args.output
    };

    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(args.update_history_id))
        .filter(owned_in_progress_condition(
            args.service_id,
            args.runtime_instance_id,
        ))
        .col_expr(
            update_history::Column::Status,
            Expr::value(match args.status {
                UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
                _ => update_history::UpdateStatus::Failed,
            }),
        )
        .col_expr(
            update_history::Column::CompletedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .col_expr(
            update_history::Column::Output,
            Expr::value(final_output.clone()),
        )
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(final_output.len() as i64),
        )
        .col_expr(
            update_history::Column::FromVersion,
            Expr::value(args.from_version),
        )
        .col_expr(
            update_history::Column::ToVersion,
            Expr::value(args.to_version),
        )
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected)
}

/// Fail a `Pending` update that has no claimed owner.
///
/// Called when an agent sends `UpdateResult(Failed)` before it ever sent
/// `UpdateStarted` (for example, an SSH connection failure before the task
/// could be spawned). In that case the row remains `Pending`, so the normal
/// owner-guarded finalization path correctly affects zero rows.
///
/// Returns the number of rows transitioned (0 if already claimed or finalized).
pub async fn fail_pending_unowned_update(
    db: &DatabaseConnection,
    protection: Option<
        Arc<dyn uptrakit_plugin_infrastructure_registry::ControllerUpdateProtection>,
    >,
    update_history_id: Uuid,
    error: Option<String>,
    output: String,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let existing = UpdateHistory::find_by_id(update_history_id)
        .one(db)
        .await
        .context_to()?;
    let Some(existing) = existing else {
        return Ok(0);
    };

    let final_output = if output.is_empty() {
        error.clone().unwrap_or_default()
    } else {
        output
    };

    let completed_at = OffsetDateTime::now_utc();

    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .filter(update_history::Column::ExecutionOwnerServiceId.is_null())
        .filter(update_history::Column::ExecutionOwnerInstanceId.is_null())
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::Failed),
        )
        .col_expr(
            update_history::Column::CompletedAt,
            Expr::value(Some(completed_at)),
        )
        .col_expr(
            update_history::Column::Output,
            Expr::value(final_output.clone()),
        )
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(final_output.len() as i64),
        )
        .exec(db)
        .await
        .context_to()?;

    if result.rows_affected == 1 {
        let mut failed = existing;
        failed.status = update_history::UpdateStatus::Failed;
        failed.completed_at = Some(completed_at);
        failed.output = final_output.clone();
        failed.output_bytes = final_output.len() as i64;
        if let Err(error) = finalize_post_update(db, protection, &failed).await {
            tracing::warn!(
                update_id = %update_history_id,
                error = %error,
                "post-update finalization failed while failing unowned pending update"
            );
        }
    }

    Ok(result.rows_affected)
}

/// Input bundle for guarded post-start finalization of one batch item.
pub struct FinalizeBatchItemIfOwnedArgs {
    pub update_history_id: Uuid,
    pub service_id: Uuid,
    pub runtime_instance_id: Option<Uuid>,
    pub status: UpdateFinalStatus,
    pub error: Option<String>,
    pub output: String,
    pub installed_version: Option<String>,
}

/// Guarded post-start finalization for one batch item.
pub async fn finalize_batch_item_if_owned(
    db: &DatabaseConnection,
    args: FinalizeBatchItemIfOwnedArgs,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let final_output = if args.output.is_empty() {
        args.error.clone().unwrap_or_default()
    } else {
        args.output
    };

    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(args.update_history_id))
        .filter(owned_in_progress_condition(
            args.service_id,
            args.runtime_instance_id,
        ))
        .col_expr(
            update_history::Column::Status,
            Expr::value(match args.status {
                UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
                _ => update_history::UpdateStatus::Failed,
            }),
        )
        .col_expr(
            update_history::Column::CompletedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .col_expr(
            update_history::Column::Output,
            Expr::value(final_output.clone()),
        )
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(final_output.len() as i64),
        )
        .col_expr(
            update_history::Column::ToVersion,
            Expr::value(args.installed_version),
        )
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected)
}

/// CAS transition from `InProgress` to `AwaitingRestart` for the resumable-update flow.
///
/// Filters: `id = update_history_id AND status = 'in_progress' AND
/// execution_owner_service_id = service_id`.
///
/// Sets: `status = 'awaiting_restart'`, `awaiting_restart_since = now()`.
///
/// Returns the number of rows affected (`0` indicates the CAS lost a race —
/// caller must skip dispatch progression and post-finalization side-effects).
pub async fn transition_to_awaiting_restart(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    service_id: Uuid,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let now = OffsetDateTime::now_utc();
    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .filter(update_history::Column::ExecutionOwnerServiceId.eq(service_id))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::AwaitingRestart),
        )
        .col_expr(
            update_history::Column::AwaitingRestartSince,
            Expr::value(Some(now)),
        )
        .exec(db)
        .await
        .context_to()?;
    Ok(result.rows_affected)
}

/// Guarded no-op used to reject stale `StdinAttention` from non-owners.
pub async fn touch_stdin_attention_if_owned(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    service_id: Uuid,
    runtime_instance_id: Option<Uuid>,
    _hint: Option<String>,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(owned_in_progress_condition(service_id, runtime_instance_id))
        .col_expr(
            update_history::Column::Interactive,
            Expr::col(update_history::Column::Interactive),
        )
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;
    use crate::queries::update_batches::tests::{
        FailFirstProtection, Fixture, NoopNotifier, insert_base_fixture, insert_second_item,
        setup_db,
    };
    use async_trait::async_trait;
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_registry::{
        ControllerPostUpdateContext, ControllerProtectionContext, ControllerProtectionDecision,
        ControllerUpdateProtection, PluginError, PluginResult, PostUpdateOutcome,
    };
    use uptrakit_shared_db::entity::{host, update_batch, update_history, update_output_line};
    use uptrakit_shared_types::PluginTypeId;
    use uptrakit_wire::{OutputStreamType, UpdateFinalStatus};
    use uuid::Uuid;

    /// Helper: insert a batch with two items on the same host (one Pending, one Queued).
    /// Returns `(batch_id, pending_id, queued_id)`.
    async fn setup_two_item_batch(db: &DatabaseConnection, f: &Fixture) -> (Uuid, Uuid, Uuid) {
        let item2_id = insert_second_item(db, f).await;
        let now = OffsetDateTime::now_utc();
        let batch_id = Uuid::now_v7();
        let pending_id = Uuid::now_v7();
        let queued_id = Uuid::now_v7();

        update_batch::ActiveModel {
            id: Set(batch_id),
            tenant_id: Set(f.tenant_id),
            batch_type: Set("host_software_item".to_string()),
            status: Set(uptrakit_shared_types::BatchStatus::InProgress),
            total_count: Set(2),
            actor_type: Set("user".to_string()),
            actor_id: Set("test".to_string()),
            output: Set(String::new()),
            output_bytes: Set(0),
            created_at: Set(now),
            completed_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        update_history::ActiveModel {
            id: Set(pending_id),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(Some(batch_id)),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        update_history::ActiveModel {
            id: Set(queued_id),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("2.0.0".to_string())),
            to_version: Set(Some("2.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Queued),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(Some(batch_id)),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        (batch_id, pending_id, queued_id)
    }

    // -- dispatch_next_in_batch --

    /// After the first item completes, `dispatch_next_in_batch` must promote
    /// the Queued item to Pending.
    #[tokio::test]
    async fn dispatch_next_in_batch_sequences_correctly() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let (batch_id, pending_id, queued_id) = setup_two_item_batch(&db, &f).await;

        // Simulate completion of the first item.
        let first = UpdateHistory::find_by_id(pending_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: update_history::ActiveModel = first.into();
        active.status = Set(update_history::UpdateStatus::Completed);
        active.completed_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.unwrap();

        // dispatch_next_in_batch should promote the Queued item to Pending.
        let result = dispatch_next_in_batch(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: None,
            },
            batch_id,
            f.host_id,
            f.tenant_id,
        )
        .await
        .unwrap();

        // Batch is not yet complete (queued item was just promoted).
        assert!(result.is_none(), "batch should still be in progress");

        let queued_record = UpdateHistory::find_by_id(queued_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            queued_record.status,
            update_history::UpdateStatus::Pending,
            "queued item must be promoted to Pending"
        );
    }

    /// When the Queued record was already promoted by another controller
    /// (CAS returns 0 rows_affected), dispatch must be skipped without error.
    #[tokio::test]
    async fn dispatch_next_in_batch_noop_when_cas_loses() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let (batch_id, pending_id, queued_id) = setup_two_item_batch(&db, &f).await;

        // Simulate first item completing.
        let first = UpdateHistory::find_by_id(pending_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: update_history::ActiveModel = first.into();
        active.status = Set(update_history::UpdateStatus::Completed);
        active.completed_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.unwrap();

        // Simulate another controller already promoting the Queued -> Pending.
        let queued = UpdateHistory::find_by_id(queued_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active2: update_history::ActiveModel = queued.into();
        active2.status = Set(update_history::UpdateStatus::Pending);
        active2.update(&db).await.unwrap();

        // dispatch_next_in_batch must succeed (no panic, no error) because there
        // are no more Queued items -- the CAS finds nothing to promote.
        let result = dispatch_next_in_batch(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: None,
            },
            batch_id,
            f.host_id,
            f.tenant_id,
        )
        .await
        .unwrap();

        // The record was already Pending (not Queued), so no Queued record
        // was found; function falls through to maybe_complete_batch which
        // returns None (still in progress).
        assert!(result.is_none());

        // The previously-queued item must still be Pending (untouched by us).
        let record = UpdateHistory::find_by_id(queued_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, update_history::UpdateStatus::Pending);
    }

    #[tokio::test]
    async fn dispatch_next_queued_for_host_continues_after_protection_failure() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let item2_id = insert_second_item(&db, &f).await;
        let protection = Arc::new(FailFirstProtection::new());

        let first =
            insert_update_record_for_item(&db, &f, f.item_id, update_history::UpdateStatus::Queued)
                .await;
        let second =
            insert_update_record_for_item(&db, &f, item2_id, update_history::UpdateStatus::Queued)
                .await;

        dispatch_next_queued_for_host(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: Some(protection.clone()),
            },
            f.host_id,
            f.tenant_id,
        )
        .await
        .unwrap();

        let first_row = UpdateHistory::find_by_id(first.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let second_row = UpdateHistory::find_by_id(second.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(first_row.status, update_history::UpdateStatus::Failed);
        assert_eq!(
            first_row.pre_update_protection_status.as_deref(),
            Some("failed")
        );
        assert_eq!(second_row.status, update_history::UpdateStatus::Failed);
        assert!(
            protection.call_count() >= 2,
            "queued promotion must continue to next sibling after failure"
        );
    }

    #[tokio::test]
    async fn reconnect_or_startup_finalize_timeout_does_not_block_queue_progression() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let item2_id = insert_second_item(&db, &f).await;
        let protection = Arc::new(SlowFinalizeProtection);

        let failed = insert_update_record(&db, &f, update_history::UpdateStatus::Failed).await;
        let queued =
            insert_update_record_for_item(&db, &f, item2_id, update_history::UpdateStatus::Queued)
                .await;

        let started = std::time::Instant::now();
        let finalize_result = crate::queries::update_dispatch::finalize_post_update_with_timeout(
            &db,
            Some(protection),
            &failed,
            Duration::from_millis(20),
        )
        .await;
        assert!(
            finalize_result.is_err(),
            "slow finalization should time out"
        );
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "timeout path should not block queue progression"
        );

        dispatch_next_queued_for_host(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: None,
            },
            f.host_id,
            f.tenant_id,
        )
        .await
        .unwrap();

        let queued_row = UpdateHistory::find_by_id(queued.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queued_row.status, update_history::UpdateStatus::Pending);
    }

    // -- owner-aware reconnect cleanup and claim/replay --

    /// Helper: insert a minimal update_history record with the given status.
    async fn insert_update_record(
        db: &DatabaseConnection,
        f: &Fixture,
        status: update_history::UpdateStatus,
    ) -> update_history::Model {
        insert_update_record_for_item(db, f, f.item_id, status).await
    }

    async fn insert_update_record_for_item(
        db: &DatabaseConnection,
        f: &Fixture,
        software_item_id: Uuid,
        status: update_history::UpdateStatus,
    ) -> update_history::Model {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(id),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(status),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
    }

    struct SlowFinalizeProtection;

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for SlowFinalizeProtection {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("infra_test_controller_update_protection")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for SlowFinalizeProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> PluginResult<ControllerProtectionDecision> {
            Err(rootcause::report!(PluginError::PluginInternal(
                "controller protection failed".to_string()
            )))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> PluginResult<PostUpdateOutcome> {
            tokio::time::sleep(Duration::from_millis(250)).await;
            Ok(PostUpdateOutcome::new(Some("slow".to_string())))
        }
    }

    struct FinalizeErrorProtection;

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for FinalizeErrorProtection {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("infra_test_controller_update_protection_finalize_error")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for FinalizeErrorProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> PluginResult<ControllerProtectionDecision> {
            Ok(ControllerProtectionDecision::skipped(None))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> PluginResult<PostUpdateOutcome> {
            Err(rootcause::report!(PluginError::PluginInternal(
                "finalize failure".to_string()
            )))
        }
    }

    async fn seed_update_output_line(
        db: &DatabaseConnection,
        update_history_id: Uuid,
        output: &str,
    ) {
        update_output_line::ActiveModel {
            id: Set(Uuid::now_v7()),
            update_history_id: Set(update_history_id),
            stream: Set(OutputStreamType::Stdout),
            output: Set(output.to_string()),
            created_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_owned_in_progress_record(
        db: &DatabaseConnection,
        f: &Fixture,
        owner_service_id: Uuid,
        owner_instance_id: Option<Uuid>,
    ) -> update_history::Model {
        let now = OffsetDateTime::now_utc();
        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::InProgress),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(Some(owner_service_id)),
            execution_owner_instance_id: Set(owner_instance_id),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
    }

    async fn insert_second_host(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            machine_id: Set("machine-002".to_string()),
            hostname: Set("host-002".to_string()),
            friendly_name: Set("Host 002".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn reconnect_cleanup_legacy_session_fails_only_legacy_owned_rows() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let legacy = insert_owned_in_progress_record(&db, &f, f.service_id, None).await;
        let other_host_id = insert_second_host(&db, f.tenant_id).await;
        let modern = insert_owned_in_progress_record(
            &db,
            &Fixture {
                host_id: other_host_id,
                ..f
            },
            f.service_id,
            Some(Uuid::now_v7()),
        )
        .await;

        let failed = mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, None)
            .await
            .unwrap();

        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].id, legacy.id);
        assert_eq!(
            UpdateHistory::find_by_id(modern.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            update_history::UpdateStatus::InProgress
        );
    }

    #[tokio::test]
    async fn reconnect_cleanup_fails_only_other_instances_of_the_same_service() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let old_instance_id = Uuid::now_v7();
        let current_instance_id = Uuid::now_v7();
        let old =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(old_instance_id)).await;
        let current = insert_owned_in_progress_record(
            &db,
            &Fixture {
                host_id: insert_second_host(&db, f.tenant_id).await,
                ..f
            },
            f.service_id,
            Some(current_instance_id),
        )
        .await;

        let failed = mark_owned_in_progress_as_failed_on_reconnect(
            &db,
            f.service_id,
            Some(current_instance_id),
        )
        .await
        .unwrap();

        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].id, old.id);
        assert_eq!(
            UpdateHistory::find_by_id(current.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            update_history::UpdateStatus::InProgress
        );
    }

    #[tokio::test]
    async fn reconnect_cleanup_leaves_other_service_rows_untouched() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let owner =
            insert_owned_in_progress_record(&db, &f, Uuid::now_v7(), Some(Uuid::now_v7())).await;

        let failed =
            mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, Some(Uuid::now_v7()))
                .await
                .unwrap();

        assert!(failed.is_empty());
        assert_eq!(
            UpdateHistory::find_by_id(owner.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            update_history::UpdateStatus::InProgress
        );
    }

    #[tokio::test]
    async fn reconnect_cleanup_leaves_same_live_instance_untouched() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;

        let failed = mark_owned_in_progress_as_failed_on_reconnect(
            &db,
            f.service_id,
            Some(runtime_instance_id),
        )
        .await
        .unwrap();

        assert!(failed.is_empty());
        assert_eq!(
            UpdateHistory::find_by_id(record.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            update_history::UpdateStatus::InProgress
        );
    }

    #[tokio::test]
    async fn reconnect_cleanup_never_touches_pending_rows() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let pending = insert_update_record(&db, &f, update_history::UpdateStatus::Pending).await;

        let failed =
            mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, Some(Uuid::now_v7()))
                .await
                .unwrap();

        assert!(failed.is_empty());
        assert_eq!(
            UpdateHistory::find_by_id(pending.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            update_history::UpdateStatus::Pending
        );
    }

    #[tokio::test]
    async fn reconnect_cleanup_never_touches_terminal_rows() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let completed =
            insert_update_record(&db, &f, update_history::UpdateStatus::Completed).await;

        let failed =
            mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, Some(Uuid::now_v7()))
                .await
                .unwrap();

        assert!(failed.is_empty());
        assert_eq!(
            UpdateHistory::find_by_id(completed.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            update_history::UpdateStatus::Completed
        );
    }

    #[tokio::test]
    async fn rollout_cleanup_fails_preexisting_in_progress_rows_regardless_of_owner() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_owned_in_progress_record(&db, &f, Uuid::now_v7(), None).await;

        let failed = mark_all_in_progress_as_failed_for_rollout(&db)
            .await
            .unwrap();

        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].id, record.id);
        assert_eq!(
            UpdateHistory::find_by_id(record.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            update_history::UpdateStatus::Failed
        );
    }

    #[tokio::test]
    async fn claim_start_succeeds_from_pending_when_unowned() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_update_record(&db, &f, update_history::UpdateStatus::Pending).await;

        let outcome = claim_or_replay_update_start_db(
            &db,
            record.id,
            f.service_id,
            Some(Uuid::now_v7()),
            false,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Claimed(_)));
        let row = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        assert_eq!(row.execution_owner_service_id, Some(f.service_id));
    }

    #[tokio::test]
    async fn claim_start_second_claim_loses_cas() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let foreign_service_id = Uuid::now_v7();
        let record = insert_update_record(&db, &f, update_history::UpdateStatus::Pending).await;

        assert!(matches!(
            claim_or_replay_update_start_db(
                &db,
                record.id,
                f.service_id,
                Some(Uuid::now_v7()),
                false
            )
            .await
            .unwrap(),
            ClaimExecutionOutcome::Claimed(_)
        ));
        assert!(matches!(
            claim_or_replay_update_start_db(
                &db,
                record.id,
                foreign_service_id,
                Some(Uuid::now_v7()),
                false,
            )
            .await
            .unwrap(),
            ClaimExecutionOutcome::Rejected
        ));
    }

    #[tokio::test]
    async fn claim_start_same_instance_replay_is_idempotent() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;

        let outcome = claim_or_replay_update_start_db(
            &db,
            record.id,
            f.service_id,
            Some(runtime_instance_id),
            false,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Replay(_)));
    }

    #[tokio::test]
    async fn claim_start_legacy_same_service_replay_is_accepted() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_owned_in_progress_record(&db, &f, f.service_id, None).await;

        let outcome = claim_or_replay_update_start_db(&db, record.id, f.service_id, None, false)
            .await
            .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Replay(_)));
    }

    #[tokio::test]
    async fn claim_start_same_instance_replay_preserves_started_at_and_output_rows() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;
        seed_update_output_line(&db, record.id, "existing\n").await;
        let before = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let before_line_count = UpdateOutputLine::find()
            .filter(update_output_line::Column::UpdateHistoryId.eq(record.id))
            .count(&db)
            .await
            .unwrap();

        let outcome = claim_or_replay_update_start_db(
            &db,
            record.id,
            f.service_id,
            Some(runtime_instance_id),
            false,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Replay(_)));
        let after = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let after_line_count = UpdateOutputLine::find()
            .filter(update_output_line::Column::UpdateHistoryId.eq(record.id))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(
            after.started_at, before.started_at,
            "replay must not rewrite started_at"
        );
        assert_eq!(after.output, before.output, "replay must not clear output");
        assert_eq!(
            after_line_count, before_line_count,
            "replay must not clear output rows"
        );
    }

    #[tokio::test]
    async fn claim_start_different_instance_same_service_is_rejected() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(Uuid::now_v7())).await;

        let outcome = claim_or_replay_update_start_db(
            &db,
            record.id,
            f.service_id,
            Some(Uuid::now_v7()),
            false,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Rejected));
    }

    #[tokio::test]
    async fn claim_start_different_service_is_rejected() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(Uuid::now_v7())).await;

        let outcome = claim_or_replay_update_start_db(
            &db,
            record.id,
            Uuid::now_v7(),
            Some(Uuid::now_v7()),
            false,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Rejected));
    }

    #[tokio::test]
    async fn claim_start_terminal_row_is_rejected() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_update_record(&db, &f, update_history::UpdateStatus::Completed).await;

        let outcome = claim_or_replay_update_start_db(
            &db,
            record.id,
            f.service_id,
            Some(Uuid::now_v7()),
            false,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Rejected));
    }

    #[tokio::test]
    async fn append_update_output_if_owned_rejects_failed_row() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;
        mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, Some(Uuid::now_v7()))
            .await
            .unwrap();

        let rows = append_update_output_if_owned(
            &db,
            record.id,
            f.service_id,
            Some(runtime_instance_id),
            OutputStreamType::Stdout,
            "late output\n",
        )
        .await
        .unwrap();

        assert!(matches!(rows, AppendUpdateOutputOutcome::Ignored));
    }

    #[tokio::test]
    async fn finalize_update_result_if_owned_rejects_failed_row() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;
        mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, Some(Uuid::now_v7()))
            .await
            .unwrap();

        let rows = finalize_update_result_if_owned(
            &db,
            FinalizeUpdateResultIfOwnedArgs {
                update_history_id: record.id,
                service_id: f.service_id,
                runtime_instance_id: Some(runtime_instance_id),
                status: UpdateFinalStatus::Completed,
                error: None,
                output: String::new(),
                from_version: None,
                to_version: Some("1.1.0".to_string()),
            },
        )
        .await
        .unwrap();

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn fail_pending_unowned_update_marks_pending_row_failed() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let pending = insert_update_record(&db, &f, update_history::UpdateStatus::Pending).await;

        let rows = fail_pending_unowned_update(
            &db,
            None,
            pending.id,
            Some("ssh pre-start failure".to_string()),
            String::new(),
        )
        .await
        .unwrap();

        assert_eq!(rows, 1);

        let updated = UpdateHistory::find_by_id(pending.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, update_history::UpdateStatus::Failed);
        assert_eq!(updated.output, "ssh pre-start failure");
        assert!(updated.completed_at.is_some());
    }

    #[tokio::test]
    async fn fail_pending_unowned_update_ignores_post_update_finalization_errors() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let pending = insert_update_record(&db, &f, update_history::UpdateStatus::Pending).await;

        let rows = fail_pending_unowned_update(
            &db,
            Some(Arc::new(FinalizeErrorProtection)),
            pending.id,
            Some("ssh pre-start failure".to_string()),
            String::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            rows, 1,
            "finalization failure must not rollback row transition"
        );
        let updated = UpdateHistory::find_by_id(pending.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, update_history::UpdateStatus::Failed);
    }

    #[tokio::test]
    async fn finalize_batch_item_if_owned_rejects_failed_row() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;
        mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, Some(Uuid::now_v7()))
            .await
            .unwrap();

        let rows = finalize_batch_item_if_owned(
            &db,
            FinalizeBatchItemIfOwnedArgs {
                update_history_id: record.id,
                service_id: f.service_id,
                runtime_instance_id: Some(runtime_instance_id),
                status: UpdateFinalStatus::Completed,
                error: None,
                output: String::new(),
                installed_version: Some("1.1.0".to_string()),
            },
        )
        .await
        .unwrap();

        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn touch_stdin_attention_if_owned_rejects_failed_row() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;
        mark_owned_in_progress_as_failed_on_reconnect(&db, f.service_id, Some(Uuid::now_v7()))
            .await
            .unwrap();

        let rows = touch_stdin_attention_if_owned(
            &db,
            record.id,
            f.service_id,
            Some(runtime_instance_id),
            Some("password prompt".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(rows, 0);
    }

    // -- transition_to_awaiting_restart --

    #[tokio::test]
    async fn test_transition_to_awaiting_restart_updates_status() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;

        let rows = transition_to_awaiting_restart(&db, record.id, f.service_id)
            .await
            .unwrap();

        assert_eq!(rows, 1);
        let after = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, update_history::UpdateStatus::AwaitingRestart);
        assert!(after.awaiting_restart_since.is_some());
    }

    #[tokio::test]
    async fn test_transition_to_awaiting_restart_wrong_service_is_noop() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;
        let other_service_id = Uuid::now_v7();

        let rows = transition_to_awaiting_restart(&db, record.id, other_service_id)
            .await
            .unwrap();

        assert_eq!(rows, 0);
        let after = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, update_history::UpdateStatus::InProgress);
        assert!(after.awaiting_restart_since.is_none());
    }

    #[tokio::test]
    async fn test_awaiting_restart_does_not_trigger_dispatch() {
        // When a record transitions to AwaitingRestart, the record must remain
        // present and non-terminal so that dispatch is blocked for the host.
        // (has_active_update_for_host will be extended to include AwaitingRestart
        // in Task 9; here we verify the precondition: the record is AwaitingRestart
        // and is not in a terminal state that would allow a new dispatch to proceed.)
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let runtime_instance_id = Uuid::now_v7();
        let record =
            insert_owned_in_progress_record(&db, &f, f.service_id, Some(runtime_instance_id)).await;

        let rows = transition_to_awaiting_restart(&db, record.id, f.service_id)
            .await
            .unwrap();
        assert_eq!(rows, 1);

        // Verify AwaitingRestart record exists and is not Completed/Failed (still blocking).
        let after = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, update_history::UpdateStatus::AwaitingRestart);
        assert!(
            after.completed_at.is_none(),
            "AwaitingRestart record must not have completed_at set"
        );

        // The transition must not have created any spurious Pending records for
        // the host — dispatch progression is the caller's responsibility.
        let pending_for_host = UpdateHistory::find()
            .filter(update_history::Column::HostId.eq(f.host_id))
            .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(pending_for_host, 0);
    }

    async fn insert_orchestrator_inprogress_record(
        db: &DatabaseConnection,
        f: &Fixture,
    ) -> update_history::Model {
        let now = OffsetDateTime::now_utc();
        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::InProgress),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            // orchestrator sentinel: owner is NULL
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(Some("protected".to_string())),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn claim_start_orchestrator_inprogress_is_claimed_by_agent() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_orchestrator_inprogress_record(&db, &f).await;
        let service_id = f.service_id;
        let instance_id = Uuid::now_v7();

        let outcome =
            claim_or_replay_update_start_db(&db, record.id, service_id, Some(instance_id), true)
                .await
                .unwrap();

        assert!(
            matches!(outcome, ClaimExecutionOutcome::Claimed(_)),
            "orchestrator-InProgress record must be Claimed by the confirming agent"
        );
        let row = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.execution_owner_service_id, Some(service_id));
        assert_eq!(row.execution_owner_instance_id, Some(instance_id));
        assert!(
            row.interactive,
            "interactive must be updated to agent's value"
        );
    }

    #[tokio::test]
    async fn claim_start_orchestrator_inprogress_race_returns_rejected() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_orchestrator_inprogress_record(&db, &f).await;

        // First agent claims it directly (simulating a concurrent claim).
        UpdateHistory::update_many()
            .filter(update_history::Column::Id.eq(record.id))
            .col_expr(
                update_history::Column::ExecutionOwnerServiceId,
                Expr::value(Some(Uuid::now_v7())),
            )
            .exec(&db)
            .await
            .unwrap();

        // Second agent's claim must lose the CAS.
        let outcome = claim_or_replay_update_start_db(
            &db,
            record.id,
            f.service_id,
            Some(Uuid::now_v7()),
            false,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ClaimExecutionOutcome::Rejected));
    }

    #[tokio::test]
    async fn claim_start_orchestrator_inprogress_preserves_output_lines() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_orchestrator_inprogress_record(&db, &f).await;
        seed_update_output_line(&db, record.id, "snapshot started\n").await;

        claim_or_replay_update_start_db(&db, record.id, f.service_id, Some(Uuid::now_v7()), false)
            .await
            .unwrap();

        let line_count = UpdateOutputLine::find()
            .filter(update_output_line::Column::UpdateHistoryId.eq(record.id))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(
            line_count, 1,
            "protection output lines must be preserved on agent claim"
        );
    }

    #[tokio::test]
    async fn mark_orchestrator_inprogress_as_failed_marks_only_null_owner_rows() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let orchestrator_row = insert_orchestrator_inprogress_record(&db, &f).await;
        // Also insert an agent-owned row on a different host — must not be touched.
        // (The partial unique index allows only one active row per host.)
        let other_host_id = insert_second_host(&db, f.tenant_id).await;
        let agent_row = insert_owned_in_progress_record(
            &db,
            &Fixture {
                host_id: other_host_id,
                ..f
            },
            f.service_id,
            Some(Uuid::now_v7()),
        )
        .await;

        mark_orchestrator_inprogress_as_failed_on_reconnect(&db, &[f.host_id])
            .await
            .unwrap();

        let orch = UpdateHistory::find_by_id(orchestrator_row.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(orch.status, update_history::UpdateStatus::Failed);
        assert_eq!(orch.pre_update_protection_status.as_deref(), Some("failed"));
        assert!(orch.completed_at.is_some());

        let agent = UpdateHistory::find_by_id(agent_row.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            agent.status,
            update_history::UpdateStatus::InProgress,
            "agent-owned row must be untouched"
        );
    }

    #[tokio::test]
    async fn mark_orchestrator_inprogress_as_failed_ignores_empty_host_list() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let row = insert_orchestrator_inprogress_record(&db, &f).await;

        mark_orchestrator_inprogress_as_failed_on_reconnect(&db, &[])
            .await
            .unwrap();

        let status = UpdateHistory::find_by_id(row.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status;
        assert_eq!(
            status,
            update_history::UpdateStatus::InProgress,
            "empty host list must be a no-op"
        );
    }
}
