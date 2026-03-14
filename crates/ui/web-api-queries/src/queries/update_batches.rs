//! Query helpers for batch update operations.
//!
//! Provides functions for:
//! - Finding outdated items on a host (for host-wide batch updates)
//! - Finding outdated hosts for an item (for item-wide rollouts)
//! - Creating a batch with associated update_history records
//! - Querying batch status and child updates
//! - Dispatching the next pending update in a batch for a host

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait, sea_query::Expr,
};
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, prelude::*, software_item, update_batch,
    update_history, update_output_line,
};
use uptrakit_shared_types::BatchStatus;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::TriggerUpdateStatus;
use uptrakit_web_api_types::update_batches::{
    BatchSkippedItem, BatchUpdateItem, BatchUpdateResponse, UpdateBatchDetailResponse,
    UpdateBatchItemSummary, UpdateBatchListQuery, UpdateBatchSummaryResponse,
};
use uuid::Uuid;

use crate::notifier::ServiceNotifier;
use crate::queries::update_triggers::{
    CreateUpdateRecordParams, DispatchUpdateParams, TriggerUpdateError, ValidatedUpdateTarget,
    has_active_update_for_host, load_target_for_dispatch,
};
use crate::queries::update_types::{ActorType, BatchType};
use crate::tenant_db::TenantDb;
use crate::token_utils::generate_uuid;

type Result<T> = std::result::Result<T, rootcause::Report<TriggerUpdateError>>;

// ---------------------------------------------------------------------------
// Private query result types
// ---------------------------------------------------------------------------

/// Aggregated status count row returned by the `list_batches` GROUP BY query.
#[derive(Debug, FromQueryResult)]
struct BatchStatusCount {
    batch_id: Option<Uuid>,
    status: String,
    count: i64,
}

// ---------------------------------------------------------------------------
// Candidate discovery
// ---------------------------------------------------------------------------

/// A software item that is outdated on a particular host.
pub struct BatchUpdateCandidate {
    pub software_item_id: Uuid,
    pub software_item_name: String,
    pub host_id: Uuid,
    pub host_name: String,
    pub installed_version: String,
    pub latest_version: String,
    pub update_category: String,
}

/// Find all outdated items for a host, optionally filtered by update category.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn find_outdated_items_for_host(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    category_filter: Option<&str>,
    exclude_item_ids: Option<&[Uuid]>,
) -> Result<Vec<BatchUpdateCandidate>> {
    // Verify host exists and belongs to tenant
    let host_record = Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotFound))?;

    // Load all host_software_items for this host that have both versions set and differ
    let mut query = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::InstalledVersion.is_not_null())
        .filter(host_software_item::Column::LatestVersion.is_not_null());

    if let Some(cat) = category_filter {
        query = query.filter(host_software_item::Column::UpdateCategory.eq(cat));
    }

    let links = query.all(db).await.context_to()?;

    if links.is_empty() {
        return Ok(vec![]);
    }

    // Batch-load active software items for all links
    let link_ids: Vec<Uuid> = links.iter().map(|l| l.software_item_id).collect();
    let items: HashMap<Uuid, software_item::Model> = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::Id.is_in(link_ids.clone()))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|i| (i.id, i))
        .collect();

    // Batch-load execute_update plugin assignments for this host
    let execute_plugin_item_ids: HashSet<Uuid> = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(link_ids))
        .filter(host_software_item_plugin::Column::Role.eq("execute_update"))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|p| p.software_item_id)
        .collect();

    // Filter to only outdated items with an execute_update plugin
    let mut candidates = Vec::new();
    for link in links {
        let installed = match link.installed_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        let latest = match link.latest_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        if installed == latest {
            continue;
        }

        // Exclude if requested
        if let Some(excludes) = exclude_item_ids
            && excludes.contains(&link.software_item_id)
        {
            continue;
        }

        // Skip inactive or missing software items
        let Some(item) = items.get(&link.software_item_id) else {
            continue;
        };

        // Skip items without an execute_update plugin
        if !execute_plugin_item_ids.contains(&link.software_item_id) {
            continue;
        }

        candidates.push(BatchUpdateCandidate {
            software_item_id: link.software_item_id,
            software_item_name: item.name.clone(),
            host_id,
            host_name: host_record.friendly_name.clone(),
            installed_version: installed.clone(),
            latest_version: latest.clone(),
            update_category: link.update_category.clone(),
        });
    }

    Ok(candidates)
}

/// Find all hosts where a software item is outdated.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn find_outdated_hosts_for_item(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
    host_ids: Option<&[Uuid]>,
) -> Result<Vec<BatchUpdateCandidate>> {
    // Verify software item exists and is active
    let item = SoftwareItem::find_by_id(item_id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::SoftwareItemNotFound))?;

    // Load all host_software_items for this software item
    let mut query = HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item::Column::InstalledVersion.is_not_null())
        .filter(host_software_item::Column::LatestVersion.is_not_null());

    if let Some(ids) = host_ids {
        query = query.filter(host_software_item::Column::HostId.is_in(ids.to_vec()));
    }

    let links = query.all(db).await.context_to()?;

    if links.is_empty() {
        return Ok(vec![]);
    }

    // Batch-load host records
    let host_record_ids: Vec<Uuid> = links.iter().map(|l| l.host_id).collect();
    let hosts: HashMap<Uuid, host::Model> = Host::find()
        .filter(host::Column::Id.is_in(host_record_ids.clone()))
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|h| (h.id, h))
        .collect();

    // Batch-load execute_update plugin assignments for this item across all hosts
    let execute_plugin_host_ids: HashSet<Uuid> = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::HostId.is_in(host_record_ids))
        .filter(host_software_item_plugin::Column::Role.eq("execute_update"))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|p| p.host_id)
        .collect();

    let mut candidates = Vec::new();
    for link in links {
        let installed = match link.installed_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        let latest = match link.latest_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        if installed == latest {
            continue;
        }

        let Some(host_record) = hosts.get(&link.host_id) else {
            continue;
        };

        // Skip hosts without an execute_update plugin for this item
        if !execute_plugin_host_ids.contains(&link.host_id) {
            continue;
        }

        candidates.push(BatchUpdateCandidate {
            software_item_id: item_id,
            software_item_name: item.name.clone(),
            host_id: link.host_id,
            host_name: host_record.friendly_name.clone(),
            installed_version: installed.clone(),
            latest_version: latest.clone(),
            update_category: link.update_category.clone(),
        });
    }

    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Batch creation
// ---------------------------------------------------------------------------

/// Parameters for creating a batch update.
pub struct CreateBatchParams<'a> {
    pub tenant_id: Uuid,
    /// The batch category.
    pub batch_type: BatchType,
    /// Who initiated the batch.
    pub actor_type: ActorType,
    pub actor_id: &'a str,
}

/// Create a batch with associated update_history records.
///
/// For each candidate, validates preconditions, creates an `update_history`
/// record, and dispatches the first pending update per host.
///
/// Returns the `BatchUpdateResponse`. If zero candidates are eligible, returns
/// a response with `batch_id: None` and `total_created: 0`.
#[tracing::instrument(skip_all)]
pub async fn create_batch(
    db: &DatabaseConnection,
    notifier: &dyn ServiceNotifier,
    params: &CreateBatchParams<'_>,
    candidates: Vec<BatchUpdateCandidate>,
) -> Result<BatchUpdateResponse> {
    if candidates.is_empty() {
        return Ok(BatchUpdateResponse {
            batch_id: None,
            total_created: 0,
            updates: vec![],
            skipped: vec![],
        });
    }

    let now = OffsetDateTime::now_utc();
    let batch_id = generate_uuid();

    // Validate all candidates and partition into valid + skipped
    let mut validated: Vec<(BatchUpdateCandidate, ValidatedUpdateTarget)> = Vec::new();
    let mut skipped: Vec<BatchSkippedItem> = Vec::new();

    for candidate in candidates {
        match super::update_triggers::validate_update_preconditions(
            db,
            params.tenant_id,
            candidate.host_id,
            candidate.software_item_id,
        )
        .await
        {
            Ok(target) => {
                validated.push((candidate, target));
            }
            Err(e) => {
                skipped.push(BatchSkippedItem {
                    software_item_id: candidate.software_item_id,
                    software_item_name: candidate.software_item_name,
                    host_id: candidate.host_id,
                    host_name: candidate.host_name,
                    reason: e.to_string(),
                });
            }
        }
    }

    if validated.is_empty() {
        return Ok(BatchUpdateResponse {
            batch_id: None,
            total_created: 0,
            updates: vec![],
            skipped,
        });
    }

    // Insert the batch record and all update_history rows atomically so that a
    // mid-flight failure cannot leave a batch record with an incorrect total_count.
    // Dispatch (WebSocket sends) happens outside the transaction because it cannot
    // be rolled back.
    let txn = db.begin().await.context_to()?;

    let batch_record = update_batch::ActiveModel {
        id: Set(batch_id),
        tenant_id: Set(params.tenant_id),
        batch_type: Set(params.batch_type.as_str().to_string()),
        status: Set(BatchStatus::InProgress),
        total_count: Set(validated.len() as i32),
        actor_type: Set(params.actor_type.as_str().to_string()),
        actor_id: Set(params.actor_id.to_string()),
        output: Set(String::new()),
        output_bytes: Set(0),
        created_at: Set(now),
        completed_at: Set(None),
    };
    batch_record.insert(&txn).await.context_to()?;

    // Determine initial status per host:
    // - If the host already has an active (Pending/InProgress) update outside
    //   this batch, ALL items on that host start as Queued.
    // - Among hosts that are free, only the first item per host is Pending;
    //   subsequent items on the same host are Queued.
    let mut externally_busy_hosts: HashSet<Uuid> = HashSet::new();
    for (candidate, _) in &validated {
        if !externally_busy_hosts.contains(&candidate.host_id) {
            let busy = has_active_update_for_host(db, candidate.host_id)
                .await
                .unwrap_or(false);
            if busy {
                externally_busy_hosts.insert(candidate.host_id);
            }
        }
    }

    let mut first_per_free_host: HashSet<Uuid> = HashSet::new();

    // Collect (history_id, should_dispatch) pairs inside the transaction, then
    // dispatch eligible items after commit.
    let mut history_ids: Vec<(Uuid, bool)> = Vec::with_capacity(validated.len());
    for (candidate, target) in &validated {
        let (initial_status, should_dispatch) =
            if externally_busy_hosts.contains(&candidate.host_id) {
                (update_history::UpdateStatus::Queued, false)
            } else {
                let is_first = first_per_free_host.insert(candidate.host_id);
                if is_first {
                    (update_history::UpdateStatus::Pending, true)
                } else {
                    (update_history::UpdateStatus::Queued, false)
                }
            };
        let update_history_id = super::update_triggers::create_update_history_record(
            &txn,
            &CreateUpdateRecordParams {
                tenant_id: params.tenant_id,
                host_id: candidate.host_id,
                item_id: candidate.software_item_id,
                host_software_item_id: Some(target.hsi_link.id),
                to_version: &candidate.latest_version,
                from_version: Some(candidate.installed_version.clone()),
                actor_type: params.actor_type,
                actor_id: params.actor_id,
                update_category: &candidate.update_category,
                batch_id: Some(batch_id),
                initial_status,
                interactive: false,
            },
        )
        .await?;
        history_ids.push((update_history_id, should_dispatch));
    }

    txn.commit().await.context_to()?;

    // Dispatch only Pending items — Queued items wait for
    // dispatch_next_in_batch to promote them.
    let mut updates: Vec<BatchUpdateItem> = Vec::new();

    for ((candidate, target), (update_history_id, should_dispatch)) in
        validated.iter().zip(history_ids)
    {
        let trigger_status = if should_dispatch {
            let connected = super::update_triggers::dispatch_update_to_agent(
                notifier,
                target,
                DispatchUpdateParams {
                    update_history_id,
                    to_version: candidate.latest_version.clone(),
                    release_info: None,
                    interactive: false,
                },
            )
            .await?;
            if connected {
                TriggerUpdateStatus::Pending
            } else {
                TriggerUpdateStatus::Queued
            }
        } else {
            // Host busy or subsequent item — queued for sequential dispatch.
            TriggerUpdateStatus::Queued
        };

        updates.push(BatchUpdateItem {
            update_history_id,
            software_item_id: candidate.software_item_id,
            software_item_name: candidate.software_item_name.clone(),
            host_id: candidate.host_id,
            host_name: candidate.host_name.clone(),
            to_version: candidate.latest_version.clone(),
            trigger_status,
        });
    }

    Ok(BatchUpdateResponse {
        batch_id: Some(batch_id),
        total_created: updates.len(),
        updates,
        skipped,
    })
}

// ---------------------------------------------------------------------------
// Batch progress: dispatch next and update batch status
// ---------------------------------------------------------------------------

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
/// [`dispatch_next_in_batch`] — the `Queued → Pending` transition is
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

    // CAS: Queued → Pending. The partial unique index on (host_id) WHERE
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
            let _ = super::update_triggers::dispatch_update_to_agent(
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
/// which performs the `Queued → Pending` CAS atomically. If another controller
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

    // CAS guard: only fail rows that are still InProgress — prevents
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

// ---------------------------------------------------------------------------
// Batch queries
// ---------------------------------------------------------------------------

/// List batches for a tenant (paginated).
#[tracing::instrument(skip_all)]
pub async fn list_batches(
    tenant_db: &TenantDb,
    query: &UpdateBatchListQuery,
) -> std::result::Result<PaginatedResponse<UpdateBatchSummaryResponse>, sea_orm::DbErr> {
    let pagination = query.pagination().resolve();

    let mut q = tenant_db
        .find::<update_batch::Entity>()
        .order_by_desc(update_batch::Column::CreatedAt);

    if let Some(ref status) = query.status {
        q = q.filter(update_batch::Column::Status.eq(status.as_str()));
    }

    let total = q.clone().count(tenant_db.db()).await?;

    let records = q
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    if records.is_empty() {
        return Ok(PaginatedResponse::new(vec![], total, pagination));
    }

    // Aggregate child status counts per batch in a single GROUP BY query,
    // avoiding loading all child records into application memory.
    let batch_ids: Vec<Uuid> = records.iter().map(|b| b.id).collect();
    let status_rows: Vec<BatchStatusCount> = {
        use sea_orm::sea_query::ExprTrait;
        UpdateHistory::find()
            .select_only()
            .column(update_history::Column::BatchId)
            .column(update_history::Column::Status)
            .column_as(
                sea_orm::sea_query::Expr::col(update_history::Column::BatchId).count(),
                "count",
            )
            .filter(update_history::Column::BatchId.is_in(batch_ids))
            .group_by(update_history::Column::BatchId)
            .group_by(update_history::Column::Status)
            .into_model::<BatchStatusCount>()
            .all(tenant_db.db())
            .await?
    };

    let mut counts: HashMap<Uuid, (i64, i64, i64)> = HashMap::new();
    for row in status_rows {
        if let Some(batch_id) = row.batch_id {
            let entry = counts.entry(batch_id).or_default();
            match row.status.as_str() {
                "completed" => entry.0 += row.count,
                "failed" => entry.1 += row.count,
                _ => entry.2 += row.count,
            }
        }
    }

    let items: Vec<UpdateBatchSummaryResponse> = records
        .into_iter()
        .map(|batch| {
            let (completed, failed, pending) = counts.get(&batch.id).copied().unwrap_or_default();
            UpdateBatchSummaryResponse {
                id: batch.id,
                batch_type: batch.batch_type,
                status: batch.status.as_str().to_string(),
                total_count: batch.total_count,
                completed_count: completed,
                failed_count: failed,
                pending_count: pending,
                actor_type: batch.actor_type,
                actor_id: batch.actor_id,
                created_at: batch.created_at,
                completed_at: batch.completed_at,
            }
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Get a single batch with its child update details.
#[tracing::instrument(skip_all, fields(%batch_id))]
pub async fn get_batch_with_items(
    tenant_db: &TenantDb,
    batch_id: Uuid,
) -> std::result::Result<Option<UpdateBatchDetailResponse>, sea_orm::DbErr> {
    let Some(batch) = tenant_db
        .find_by_id::<update_batch::Entity, _>(batch_id)
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };

    let children = UpdateHistory::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .order_by_asc(update_history::Column::Id)
        .all(tenant_db.db())
        .await?;

    // Batch-load host names and software item names
    let host_ids: Vec<Uuid> = children
        .iter()
        .map(|c| c.host_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let si_ids: Vec<Uuid> = children
        .iter()
        .map(|c| c.software_item_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let host_names: HashMap<Uuid, String> = Host::find()
        .filter(host::Column::Id.is_in(host_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|h| (h.id, h.friendly_name))
        .collect();

    let si_names: HashMap<Uuid, String> = SoftwareItem::find()
        .filter(software_item::Column::Id.is_in(si_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|si| (si.id, si.name))
        .collect();

    let mut completed_count: i64 = 0;
    let mut failed_count: i64 = 0;
    let mut pending_count: i64 = 0;

    let updates: Vec<UpdateBatchItemSummary> = children
        .into_iter()
        .map(|child| {
            match child.status {
                update_history::UpdateStatus::Completed => completed_count += 1,
                update_history::UpdateStatus::Failed => failed_count += 1,
                update_history::UpdateStatus::Queued
                | update_history::UpdateStatus::Pending
                | update_history::UpdateStatus::InProgress => pending_count += 1,
                _ => {
                    tracing::warn!(
                        "Unknown update status {:?}, counting as pending",
                        child.status
                    );
                    pending_count += 1;
                }
            }
            UpdateBatchItemSummary {
                update_history_id: child.id,
                host_id: child.host_id,
                host_name: host_names
                    .get(&child.host_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown Host".to_string()),
                software_item_id: child.software_item_id,
                software_item_name: si_names
                    .get(&child.software_item_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown Software Item".to_string()),
                to_version: child.to_version.unwrap_or_default(),
                status: child.status.to_string(),
                update_category: child.update_category,
            }
        })
        .collect();

    Ok(Some(UpdateBatchDetailResponse {
        id: batch.id,
        batch_type: batch.batch_type,
        status: batch.status.as_str().to_string(),
        total_count: batch.total_count,
        completed_count,
        failed_count,
        pending_count,
        actor_type: batch.actor_type,
        actor_id: batch.actor_id,
        updates,
        created_at: batch.created_at,
        completed_at: batch.completed_at,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_internal_wire::ControllerMessage;
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, plugin_config, service, service_host,
        software_item, tenant, update_history,
    };
    use uptrakit_shared_types::ServiceStatus;
    use uuid::Uuid;

    /// A no-op notifier for tests — always returns `true` (agent locally connected).
    struct NoopNotifier;

    #[async_trait::async_trait]
    impl crate::notifier::ServiceNotifier for NoopNotifier {
        async fn send_to_service(&self, _service_id: &Uuid, _msg: ControllerMessage) -> bool {
            true
        }
    }

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    struct Fixture {
        tenant_id: Uuid,
        item_id: Uuid,
        host_id: Uuid,
    }

    /// Insert a minimal valid fixture: tenant, one software item, one host, one agent
    /// (Approved), service_host link, host_software_item (installed="1.0.0",
    /// latest="1.1.0"), plugin_config, and an execute_update plugin assignment.
    async fn insert_base_fixture(db: &DatabaseConnection) -> Fixture {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let service_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();

        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(format!("test-{tenant_id}")),
            is_default: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("test-app".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set("machine-001".to_string()),
            hostname: Set("host-001".to_string()),
            friendly_name: Set("Host 001".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("agent-host".to_string()),
            friendly_name: Set("Agent 001".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        plugin_config::ActiveModel {
            id: Set(plugin_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-plugin".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        let hsi_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(plugin_config_id)),
            package_identifier: Set(Some("test-pkg".to_string())),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(None),
            latest_version: Set(Some("1.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("security".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(hsi_id),
            plugin_config_id: Set(Some(plugin_config_id)),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        let _ = plugin_config_id; // used in DB only; not needed by callers
        Fixture {
            tenant_id,
            item_id,
            host_id,
        }
    }

    // ── find_outdated_items_for_host ────────────────────────────────────

    #[tokio::test]
    async fn find_outdated_items_empty_when_versions_match() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(f.host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(f.item_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: host_software_item::ActiveModel = hsi.into();
        active.installed_version = Set(Some("1.1.0".to_string())); // same as latest
        active.update(&db).await.unwrap();

        let candidates = find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, None)
            .await
            .unwrap();
        assert!(
            candidates.is_empty(),
            "expected empty; got {}",
            candidates.len()
        );
    }

    #[tokio::test]
    async fn find_outdated_items_returns_candidate_when_outdated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;

        let candidates = find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, None)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].software_item_id, f.item_id);
        assert_eq!(candidates[0].installed_version, "1.0.0");
        assert_eq!(candidates[0].latest_version, "1.1.0");
    }

    #[tokio::test]
    async fn find_outdated_items_filters_by_category() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await; // item_id has category "security"
        let now = OffsetDateTime::now_utc();

        // Add a second software item and link it to the same host with category "feature".
        let item2_id = Uuid::now_v7();
        let pc2_id = Uuid::now_v7();
        software_item::ActiveModel {
            id: Set(item2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-app-2".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        plugin_config::ActiveModel {
            id: Set(pc2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-plugin-2".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        let hsi2_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi2_id),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(pc2_id)),
            package_identifier: Set(Some("test-app-2".to_string())),
            installed_version: Set(Some("2.0.0".to_string())),
            installed_version_detected_at: Set(None),
            latest_version: Set(Some("2.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("feature".to_string()),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            host_software_item_id: Set(hsi2_id),
            plugin_config_id: Set(Some(pc2_id)),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo2".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();

        // Filter by "security" — should return only the first item.
        let candidates =
            find_outdated_items_for_host(&db, f.tenant_id, f.host_id, Some("security"), None)
                .await
                .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].software_item_id, f.item_id);

        // Filter by "feature" — should return only the second item.
        let candidates_feature =
            find_outdated_items_for_host(&db, f.tenant_id, f.host_id, Some("feature"), None)
                .await
                .unwrap();
        assert_eq!(candidates_feature.len(), 1);
        assert_eq!(candidates_feature[0].software_item_id, item2_id);
    }

    #[tokio::test]
    async fn find_outdated_items_excludes_specified_ids() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;

        let candidates =
            find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, Some(&[f.item_id]))
                .await
                .unwrap();
        assert!(
            candidates.is_empty(),
            "excluded item must not appear in results"
        );
    }

    // ── find_outdated_hosts_for_item ────────────────────────────────────

    #[tokio::test]
    async fn find_outdated_hosts_empty_when_up_to_date() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(f.host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(f.item_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: host_software_item::ActiveModel = hsi.into();
        active.installed_version = Set(Some("1.1.0".to_string())); // same as latest
        active.update(&db).await.unwrap();

        let candidates = find_outdated_hosts_for_item(&db, f.tenant_id, f.item_id, None)
            .await
            .unwrap();
        assert!(
            candidates.is_empty(),
            "expected empty; got {}",
            candidates.len()
        );
    }

    #[tokio::test]
    async fn find_outdated_hosts_returns_candidate_when_outdated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;

        let candidates = find_outdated_hosts_for_item(&db, f.tenant_id, f.item_id, None)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].host_id, f.host_id);
        assert_eq!(candidates[0].software_item_id, f.item_id);
        assert_eq!(candidates[0].installed_version, "1.0.0");
        assert_eq!(candidates[0].latest_version, "1.1.0");
    }

    // ── create_batch (per-host Queued status) ───────────────────────────

    /// Helper: insert a second software item + host_software_item + plugin assignment
    /// on the same host as the base fixture. Returns (item2_id).
    async fn insert_second_item(db: &DatabaseConnection, f: &Fixture) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let item2_id = Uuid::now_v7();
        let pc2_id = Uuid::now_v7();

        software_item::ActiveModel {
            id: Set(item2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-app-2".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        plugin_config::ActiveModel {
            id: Set(pc2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-plugin-2".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        let hsi2_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi2_id),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(pc2_id)),
            package_identifier: Set(Some("test-app-2".to_string())),
            installed_version: Set(Some("2.0.0".to_string())),
            installed_version_detected_at: Set(None),
            latest_version: Set(Some("2.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("security".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            host_software_item_id: Set(hsi2_id),
            plugin_config_id: Set(Some(pc2_id)),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo2".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        item2_id
    }

    /// When a batch contains two outdated items on the same host, the first must
    /// be inserted as `Pending` and the second as `Queued`.
    #[tokio::test]
    async fn create_batch_multiple_items_same_host_queued() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let item2_id = insert_second_item(&db, &f).await;

        let candidates = vec![
            BatchUpdateCandidate {
                software_item_id: f.item_id,
                software_item_name: "test-app".to_string(),
                host_id: f.host_id,
                host_name: "Host 001".to_string(),
                installed_version: "1.0.0".to_string(),
                latest_version: "1.1.0".to_string(),
                update_category: "security".to_string(),
            },
            BatchUpdateCandidate {
                software_item_id: item2_id,
                software_item_name: "test-app-2".to_string(),
                host_id: f.host_id,
                host_name: "Host 001".to_string(),
                installed_version: "2.0.0".to_string(),
                latest_version: "2.1.0".to_string(),
                update_category: "security".to_string(),
            },
        ];

        let resp = create_batch(
            &db,
            &NoopNotifier,
            &CreateBatchParams {
                tenant_id: f.tenant_id,
                batch_type: BatchType::HostUpdate,
                actor_type: ActorType::User,
                actor_id: "test-user",
            },
            candidates,
        )
        .await
        .unwrap();

        assert_eq!(resp.total_created, 2);
        assert!(resp.batch_id.is_some());

        // Verify DB: exactly one Pending and one Queued.
        let all_records = UpdateHistory::find()
            .filter(update_history::Column::BatchId.eq(resp.batch_id.unwrap()))
            .all(&db)
            .await
            .unwrap();

        assert_eq!(all_records.len(), 2);

        let pending_count = all_records
            .iter()
            .filter(|r| r.status == update_history::UpdateStatus::Pending)
            .count();
        let queued_count = all_records
            .iter()
            .filter(|r| r.status == update_history::UpdateStatus::Queued)
            .count();

        assert_eq!(pending_count, 1, "expected exactly one Pending item");
        assert_eq!(queued_count, 1, "expected exactly one Queued item");

        // The first item (by insertion order) must be Pending.
        let first = all_records
            .iter()
            .find(|r| r.software_item_id == f.item_id)
            .unwrap();
        assert_eq!(
            first.status,
            update_history::UpdateStatus::Pending,
            "first item must be Pending"
        );

        let second = all_records
            .iter()
            .find(|r| r.software_item_id == item2_id)
            .unwrap();
        assert_eq!(
            second.status,
            update_history::UpdateStatus::Queued,
            "second item must be Queued"
        );
    }

    // ── dispatch_next_in_batch ──────────────────────────────────────────

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

        // Simulate another controller already promoting the Queued → Pending.
        let queued = UpdateHistory::find_by_id(queued_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active2: update_history::ActiveModel = queued.into();
        active2.status = Set(update_history::UpdateStatus::Pending);
        active2.update(&db).await.unwrap();

        // dispatch_next_in_batch must succeed (no panic, no error) because there
        // are no more Queued items — the CAS finds nothing to promote.
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

    // ── mark_in_progress_as_failed ──────────────────────────────────────

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
