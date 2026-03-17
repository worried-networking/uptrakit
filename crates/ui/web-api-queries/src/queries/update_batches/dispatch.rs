//! Batch progress: dispatch next update and batch completion detection.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set, TransactionTrait, sea_query::Expr,
};
use time::OffsetDateTime;
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

// ---------------------------------------------------------------------------
// Restart recovery
// ---------------------------------------------------------------------------

/// Mark all `InProgress` update records for the given hosts as `Failed`.
///
/// Called during agent reconnect to prevent orphaned in-progress updates when
/// an agent restarts and loses track of the updates it was executing.
///
/// Returns the **pre-update** snapshots of the affected rows so the caller can
/// close any open SSE streams and dispatch follow-up updates.
///
/// The bulk `UPDATE` (with a `status = InProgress` CAS guard) and the
/// `update_output_line` deletion are wrapped in a single transaction to avoid
/// partial cleanup states on connection loss.
#[tracing::instrument(skip_all)]
pub async fn mark_in_progress_as_failed(
    db: &DatabaseConnection,
    host_ids: &[Uuid],
) -> std::result::Result<Vec<update_history::Model>, rootcause::Report<TriggerUpdateError>> {
    if host_ids.is_empty() {
        return Ok(vec![]);
    }

    // Load all InProgress records for these hosts (pre-update snapshot).
    let records = UpdateHistory::find()
        .filter(update_history::Column::HostId.is_in(host_ids.to_vec()))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .all(db)
        .await
        .context_to()?;

    if records.is_empty() {
        return Ok(vec![]);
    }

    let ids: Vec<Uuid> = records.iter().map(|r| r.id).collect();
    let now = OffsetDateTime::now_utc();
    let reason = "Update interrupted: agent restarted";

    let txn = db.begin().await.context_to()?;

    // CAS guard: only fail rows that are still InProgress -- prevents
    // double-failing if two controller replicas race on the same host.
    UpdateHistory::update_many()
        .filter(update_history::Column::Id.is_in(ids.clone()))
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
        .exec(&txn)
        .await
        .context_to()?;

    // Remove streaming output lines that accumulated before the restart.
    UpdateOutputLine::delete_many()
        .filter(update_output_line::Column::UpdateHistoryId.is_in(ids))
        .exec(&txn)
        .await
        .context_to()?;

    txn.commit().await.context_to()?;

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queries::update_batches::tests::{
        Fixture, NoopNotifier, insert_base_fixture, insert_second_item, setup_db,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{host, update_batch, update_history};
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

    // -- mark_in_progress_as_failed --

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

    /// A `Pending` record must be left untouched; function returns an empty vec.
    #[tokio::test]
    async fn no_in_progress_returns_empty() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let pending = insert_update_record(&db, &f, update_history::UpdateStatus::Pending).await;

        let result = mark_in_progress_as_failed(&db, &[f.host_id]).await.unwrap();

        assert!(
            result.is_empty(),
            "expected empty result for no in-progress records"
        );

        // Pending record must be untouched.
        let reloaded = UpdateHistory::find_by_id(pending.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.status, update_history::UpdateStatus::Pending);
    }

    /// An `InProgress` record must be returned and marked `Failed` in the DB.
    #[tokio::test]
    async fn in_progress_marked_failed() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let record = insert_update_record(&db, &f, update_history::UpdateStatus::InProgress).await;

        let result = mark_in_progress_as_failed(&db, &[f.host_id]).await.unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, record.id);

        // DB row must be Failed with correct output and a completed_at.
        let reloaded = UpdateHistory::find_by_id(record.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.status, update_history::UpdateStatus::Failed);
        assert_eq!(reloaded.output, "Update interrupted: agent restarted");
        assert_eq!(
            reloaded.output_bytes,
            "Update interrupted: agent restarted".len() as i64
        );
        assert!(reloaded.completed_at.is_some(), "completed_at must be set");
    }

    /// A mix of `InProgress` and `Pending` records across two hosts: only the
    /// `InProgress` one must be returned and failed; the `Pending` record on
    /// the second host must be untouched.
    ///
    /// A second host is required because the partial unique index on
    /// `(host_id) WHERE status IN ('pending', 'in_progress')` prevents two
    /// active records on the same host.
    #[tokio::test]
    async fn pending_not_touched() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let in_progress =
            insert_update_record(&db, &f, update_history::UpdateStatus::InProgress).await;

        // Insert a second host to hold the Pending record.
        let now = OffsetDateTime::now_utc();
        let host2_id = host::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
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
        .insert(&db)
        .await
        .unwrap()
        .id;

        let f2 = Fixture {
            tenant_id: f.tenant_id,
            item_id: f.item_id,
            host_id: host2_id,
        };
        let pending = insert_update_record(&db, &f2, update_history::UpdateStatus::Pending).await;

        let result = mark_in_progress_as_failed(&db, &[f.host_id, host2_id])
            .await
            .unwrap();

        assert_eq!(
            result.len(),
            1,
            "only the in-progress record should be returned"
        );
        assert_eq!(result[0].id, in_progress.id);

        // InProgress record is now Failed.
        let reloaded_ip = UpdateHistory::find_by_id(in_progress.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded_ip.status, update_history::UpdateStatus::Failed);

        // Pending record on host2 is untouched.
        let reloaded_pending = UpdateHistory::find_by_id(pending.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reloaded_pending.status,
            update_history::UpdateStatus::Pending
        );
    }
}
