//! Batch progress: dispatch next update and batch completion detection.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, Set, TransactionTrait, sea_query::Expr,
};
use time::OffsetDateTime;
use uptrakit_internal_wire::{OutputStreamType, UpdateFinalStatus};
use uptrakit_shared_db::entity::{prelude::*, update_batch, update_history, update_output_line};
use uptrakit_shared_types::BatchStatus;
use uuid::Uuid;

use crate::notifier::ServiceNotifier;
use crate::queries::update_dispatch::{
    DispatchUpdateParams, TriggerUpdateError, load_target_for_dispatch,
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
    notifier: &dyn ServiceNotifier,
    host_id: Uuid,
    tenant_id: Uuid,
) -> std::result::Result<(), rootcause::Report<TriggerUpdateError>> {
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

    // CAS: Queued -> Pending. The partial unique index on (host_id) WHERE
    // status IN ('pending', 'in_progress') prevents double-dispatch.
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
            "CAS missed: another controller already promoted this queued item, skipping"
        );
        return Ok(());
    }

    match load_target_for_dispatch(
        db,
        tenant_id,
        next_record.host_id,
        next_record.software_item_id,
    )
    .await
    {
        Ok(target) => {
            let _ = super::super::update_dispatch::dispatch_update_to_agent(
                notifier,
                &target,
                DispatchUpdateParams {
                    update_history_id: next_record.id,
                    to_version: next_record.to_version.unwrap_or_default(),
                    release_info: None,
                    interactive: next_record.interactive,
                },
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(
                update_id = %next_record.id,
                %host_id,
                error = %e,
                "failed to load dispatch data for next queued update, marking as failed"
            );
            let mut active: update_history::ActiveModel = next_record.into();
            active.status = Set(update_history::UpdateStatus::Failed);
            active.completed_at = Set(Some(OffsetDateTime::now_utc()));
            active.output = Set(format!("dispatch failed: {e}"));
            active.update(db).await.context_to()?;
        }
    }

    Ok(())
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
    notifier: &dyn ServiceNotifier,
    batch_id: Uuid,
    host_id: Uuid,
    tenant_id: Uuid,
) -> std::result::Result<Option<BatchCompletionInfo>, rootcause::Report<TriggerUpdateError>> {
    // Dispatch the next queued update for this host (FIFO across all batches
    // and non-batch updates). This supersedes the previous batch-scoped query
    // so that a queued non-batch update is not skipped when a batch completes.
    dispatch_next_queued_for_host(db, notifier, host_id, tenant_id).await?;

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
            failed.push(record);
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
            failed.push(record);
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
    _stream: OutputStreamType,
    _output: &str,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(owned_in_progress_condition(service_id, runtime_instance_id))
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::col(update_history::Column::OutputBytes),
        )
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected)
}

/// Guarded post-start finalization for a single update.
pub async fn finalize_update_result_if_owned(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    service_id: Uuid,
    runtime_instance_id: Option<Uuid>,
    status: UpdateFinalStatus,
    error: Option<String>,
    output: String,
    from_version: Option<String>,
    to_version: Option<String>,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let final_output = if output.is_empty() {
        error.clone().unwrap_or_default()
    } else {
        output
    };

    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(owned_in_progress_condition(service_id, runtime_instance_id))
        .col_expr(
            update_history::Column::Status,
            Expr::value(match status {
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
            Expr::value(from_version),
        )
        .col_expr(update_history::Column::ToVersion, Expr::value(to_version))
        .exec(db)
        .await
        .context_to()?;

    Ok(result.rows_affected)
}

/// Guarded post-start finalization for one batch item.
pub async fn finalize_batch_item_if_owned(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    service_id: Uuid,
    runtime_instance_id: Option<Uuid>,
    status: UpdateFinalStatus,
    error: Option<String>,
    output: String,
    installed_version: Option<String>,
) -> std::result::Result<u64, rootcause::Report<TriggerUpdateError>> {
    let final_output = if output.is_empty() {
        error.clone().unwrap_or_default()
    } else {
        output
    };

    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(owned_in_progress_condition(service_id, runtime_instance_id))
        .col_expr(
            update_history::Column::Status,
            Expr::value(match status {
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
            Expr::value(installed_version),
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
    use super::*;
    use crate::queries::update_batches::tests::{
        Fixture, NoopNotifier, insert_base_fixture, insert_second_item, setup_db,
    };
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
    use time::OffsetDateTime;
    use uptrakit_internal_wire::{OutputStreamType, UpdateFinalStatus};
    use uptrakit_shared_db::entity::{host, update_batch, update_history, update_output_line};
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
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(Some(batch_id)),
            interactive: Set(false),
            output_truncated: Set(false),
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
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(Some(batch_id)),
            interactive: Set(false),
            output_truncated: Set(false),
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
        let result = dispatch_next_in_batch(&db, &NoopNotifier, batch_id, f.host_id, f.tenant_id)
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
        let result = dispatch_next_in_batch(&db, &NoopNotifier, batch_id, f.host_id, f.tenant_id)
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

    // -- owner-aware reconnect cleanup and claim/replay --

    /// Helper: insert a minimal update_history record with the given status.
    async fn insert_update_record(
        db: &DatabaseConnection,
        f: &Fixture,
        status: update_history::UpdateStatus,
    ) -> update_history::Model {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(id),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
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
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
        }
        .insert(db)
        .await
        .unwrap()
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
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
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

        assert_eq!(rows, 0);
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
            record.id,
            f.service_id,
            Some(runtime_instance_id),
            UpdateFinalStatus::Completed,
            None,
            String::new(),
            None,
            Some("1.1.0".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(rows, 0);
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
            record.id,
            f.service_id,
            Some(runtime_instance_id),
            UpdateFinalStatus::Completed,
            None,
            String::new(),
            Some("1.1.0".to_string()),
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
}
