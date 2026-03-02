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
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;
use uptrakit_internal_wire::{BatchHostPackageUpdate, ControllerMessage, ExecuteBatchHostPackageUpdatePayload};
use uptrakit_shared_db::entity::{
    host, host_package, host_package_update_history, host_software_item, host_software_item_plugin,
    plugin_config, prelude::*, service, service_host, software_item, update_batch, update_history,
};
use uptrakit_shared_types::BatchStatus;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::TriggerUpdateStatus;
use uptrakit_web_api_types::update_batches::{
    BatchSkippedItem, BatchUpdateItem, BatchUpdateResponse, UpdateBatchDetailResponse,
    UpdateBatchItemSummary, UpdateBatchListQuery, UpdateBatchSummaryResponse,
};
use uuid::Uuid;

use crate::auth::token::generate_uuid;
use crate::notification_service::NotificationService;
use crate::queries::update_triggers::{
    CreateUpdateRecordParams, DispatchUpdateParams, TriggerUpdateError, ValidatedUpdateTarget,
};
use crate::tenant_db::TenantDb;

type Result<T> = std::result::Result<T, rootcause::Report<TriggerUpdateError>>;

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
        .filter(software_item::Column::Enabled.eq(true))
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
        .filter(software_item::Column::Enabled.eq(true))
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
    pub batch_type: &'a str,
    pub actor_type: &'a str,
    pub actor_id: &'a str,
}

/// Create a batch with associated update_history records.
///
/// For each candidate, validates preconditions, creates an `update_history`
/// record, and dispatches the first pending update per host.
///
/// Returns the `BatchUpdateResponse`. If zero candidates are eligible, returns
/// a response with `batch_id: None` and `total_created: 0`.
pub async fn create_batch(
    db: &DatabaseConnection,
    notifier: &NotificationService,
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
        batch_type: Set(params.batch_type.to_string()),
        status: Set(BatchStatus::InProgress),
        total_count: Set(validated.len() as i32),
        actor_type: Set(params.actor_type.to_string()),
        actor_id: Set(params.actor_id.to_string()),
        created_at: Set(now),
        completed_at: Set(None),
    };
    batch_record.insert(&txn).await.context_to()?;

    // Collect (history_id, index) inside the transaction, then dispatch after commit.
    let mut history_ids: Vec<Uuid> = Vec::with_capacity(validated.len());
    for (candidate, _target) in &validated {
        let update_history_id = super::update_triggers::create_update_history_record(
            &txn,
            &CreateUpdateRecordParams {
                host_id: candidate.host_id,
                item_id: candidate.software_item_id,
                to_version: &candidate.latest_version,
                actor_type: params.actor_type,
                actor_id: params.actor_id,
                update_category: &candidate.update_category,
                batch_id: Some(batch_id),
            },
        )
        .await?;
        history_ids.push(update_history_id);
    }

    txn.commit().await.context_to()?;

    // Dispatch the first pending update per host (outside the transaction —
    // WebSocket sends are not transactional).
    let mut updates: Vec<BatchUpdateItem> = Vec::new();
    let mut dispatched_hosts: HashSet<Uuid> = HashSet::new();

    for ((candidate, target), update_history_id) in validated.iter().zip(history_ids) {
        let trigger_status = if !dispatched_hosts.contains(&candidate.host_id) {
            dispatched_hosts.insert(candidate.host_id);
            let connected = super::update_triggers::dispatch_update_to_agent(
                notifier,
                target,
                DispatchUpdateParams {
                    update_history_id,
                    to_version: candidate.latest_version.clone(),
                    release_info: None,
                },
            )
            .await?;
            if connected {
                TriggerUpdateStatus::Pending
            } else {
                TriggerUpdateStatus::Queued
            }
        } else {
            // Subsequent items for the same host are queued; they will be
            // dispatched sequentially as earlier items complete.
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

/// Called after an update completes in a batch. Dispatches the next pending
/// update for the same host within the batch, and checks if the batch is done.
///
/// Returns `Some(BatchCompletionInfo)` if the batch just transitioned to a
/// terminal status, so the caller can dispatch a notification event.
pub async fn dispatch_next_in_batch(
    db: &DatabaseConnection,
    notifier: &NotificationService,
    batch_id: Uuid,
    host_id: Uuid,
    tenant_id: Uuid,
) -> std::result::Result<Option<BatchCompletionInfo>, rootcause::Report<TriggerUpdateError>> {
    // Find the next Pending update in this batch for this host (ordered by id)
    let next = UpdateHistory::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .filter(update_history::Column::HostId.eq(host_id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .order_by_asc(update_history::Column::Id)
        .one(db)
        .await
        .context_to()?;

    if let Some(next_record) = next {
        // Validate and dispatch
        match super::update_triggers::validate_update_preconditions(
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
                        to_version: next_record.to_version,
                        release_info: None,
                    },
                )
                .await;
            }
            Err(e) => {
                tracing::warn!(
                    update_id = %next_record.id,
                    batch_id = %batch_id,
                    error = %e,
                    "failed to dispatch next batch update, marking as failed"
                );
                // Mark the update as failed so the batch can progress
                let mut active: update_history::ActiveModel = next_record.into();
                active.status = Set(update_history::UpdateStatus::Failed);
                active.completed_at = Set(Some(OffsetDateTime::now_utc()));
                active.output = Set(format!("dispatch failed: {e}"));
                active.update(db).await.context_to()?;
            }
        }
    }

    // Check if all items in the batch are terminal
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
    if matches!(batch.status, BatchStatus::Completed | BatchStatus::PartiallyCompleted) {
        return Ok(None);
    }

    let total_count = batch.total_count;
    let mut active: update_batch::ActiveModel = batch.into();
    active.status = Set(new_status);
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
// Batch queries
// ---------------------------------------------------------------------------

/// List batches for a tenant (paginated).
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

    // Load child status counts for all batches in one pass
    let batch_ids: Vec<Uuid> = records.iter().map(|b| b.id).collect();
    let child_records = UpdateHistory::find()
        .filter(update_history::Column::BatchId.is_in(batch_ids))
        .all(tenant_db.db())
        .await?;

    let mut counts: HashMap<Uuid, (i64, i64, i64)> = HashMap::new();
    for child in &child_records {
        if let Some(batch_id) = child.batch_id {
            let entry = counts.entry(batch_id).or_default();
            match child.status {
                update_history::UpdateStatus::Completed => entry.0 += 1,
                update_history::UpdateStatus::Failed => entry.1 += 1,
                update_history::UpdateStatus::Pending
                | update_history::UpdateStatus::InProgress => entry.2 += 1,
                _ => {
                    tracing::warn!("Unknown update status {:?}, counting as pending", child.status);
                    entry.2 += 1;
                }
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
                update_history::UpdateStatus::Pending
                | update_history::UpdateStatus::InProgress => pending_count += 1,
                _ => {
                    tracing::warn!("Unknown update status {:?}, counting as pending", child.status);
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
                to_version: child.to_version,
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
// MQTT-triggered host package batch update
// ---------------------------------------------------------------------------

/// Trigger a batch update for all outdated host packages on the given host.
///
/// Groups outdated packages by `plugin_config_id` and dispatches one
/// [`ExecuteBatchHostPackageUpdate`](ControllerMessage::ExecuteBatchHostPackageUpdate)
/// message per plugin config group to the host's linked agent.
///
/// Returns `None` if no outdated packages exist (nothing to do), or
/// `Some(batch_id)` when at least one dispatch was issued.
///
/// # Errors
///
/// Returns an error if:
/// - The host is not found or deactivated.
/// - Any pending or in-progress host package update already exists for this host.
/// - No agent is linked to the host, or the agent is not approved.
/// - A database error occurs.
pub async fn trigger_all_host_package_updates_for_host(
    db: &DatabaseConnection,
    notifier: &NotificationService,
    tenant_id: Uuid,
    host_id: Uuid,
    actor_type: &str,
    actor_id: &str,
) -> Result<Option<Uuid>> {
    // 1. Verify host exists and belongs to tenant.
    let host_record = Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotFound))?;

    // 2. Find all outdated packages (enabled, non-deactivated, both versions
    //    known and differing).
    let all_packages: Vec<host_package::Model> = HostPackage::find()
        .filter(host_package::Column::HostId.eq(host_id))
        .filter(host_package::Column::Enabled.eq(true))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .filter(host_package::Column::InstalledVersion.is_not_null())
        .filter(host_package::Column::LatestVersion.is_not_null())
        .all(db)
        .await
        .context_to()?;

    let outdated: Vec<host_package::Model> = all_packages
        .into_iter()
        .filter(|p| {
            matches!(
                (&p.installed_version, &p.latest_version),
                (Some(i), Some(l)) if i != l
            )
        })
        .collect();

    if outdated.is_empty() {
        return Ok(None);
    }

    // 3. Guard: reject if any pending/in-progress update already exists.
    let active_count = HostPackageUpdateHistory::find()
        .filter(host_package_update_history::Column::HostId.eq(host_id))
        .filter(
            Condition::any()
                .add(host_package_update_history::Column::Status.eq("pending"))
                .add(host_package_update_history::Column::Status.eq("in_progress")),
        )
        .count(db)
        .await
        .context_to()?;

    if active_count > 0 {
        bail!(TriggerUpdateError::UpdateAlreadyActive);
    }

    // 4. Find the agent linked to this host.
    let agent_link = ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::NoAgent))?;

    let agent = Service::find_by_id(agent_link.service_id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::NoAgent))?;

    if agent.status != service::ServiceStatus::Approved {
        bail!(TriggerUpdateError::AgentNotApproved);
    }

    // 5. Load plugin_configs for all plugin_config_ids referenced by outdated packages.
    let plugin_config_ids: Vec<Uuid> = {
        let mut seen = HashSet::new();
        outdated
            .iter()
            .filter(|p| seen.insert(p.plugin_config_id))
            .map(|p| p.plugin_config_id)
            .collect()
    };

    let configs: HashMap<Uuid, plugin_config::Model> = PluginConfig::find()
        .filter(plugin_config::Column::Id.is_in(plugin_config_ids))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();

    let now = OffsetDateTime::now_utc();
    let batch_id = generate_uuid();
    let total_count = outdated.len() as i32;

    // 6. Insert update_batch + all history records in one transaction so that
    //    a mid-flight failure cannot leave orphaned history records.
    let txn = db.begin().await.context_to()?;

    let batch_record = update_batch::ActiveModel {
        id: Set(batch_id),
        tenant_id: Set(tenant_id),
        batch_type: Set("host_package".to_string()),
        status: Set(BatchStatus::InProgress),
        total_count: Set(total_count),
        actor_type: Set(actor_type.to_string()),
        actor_id: Set(actor_id.to_string()),
        created_at: Set(now),
        completed_at: Set(None),
    };
    batch_record.insert(&txn).await.context_to()?;

    // Collect (history_id, package) pairs for dispatch after commit.
    let mut history_items: Vec<(Uuid, &host_package::Model)> = Vec::with_capacity(total_count as usize);
    for pkg in &outdated {
        let history_id = generate_uuid();
        let history = host_package_update_history::ActiveModel {
            id: Set(history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            host_package_id: Set(pkg.id),
            from_version: Set(pkg.installed_version.clone()),
            to_version: Set(pkg.latest_version.clone()),
            status: Set("pending".to_string()),
            output: Set(None),
            output_bytes: Set(0),
            actor_type: Set(actor_type.to_string()),
            actor_id: Set(actor_id.to_string()),
            update_category: Set(pkg.update_category.clone()),
            started_at: Set(None),
            completed_at: Set(None),
            created_at: Set(now),
            batch_id: Set(Some(batch_id)),
        };
        history.insert(&txn).await.context_to()?;
        history_items.push((history_id, pkg));
    }

    txn.commit().await.context_to()?;

    // 7. Group packages by plugin_config_id and dispatch one payload per group.
    //    This happens outside the transaction since WebSocket sends cannot be
    //    rolled back.
    let mut by_config: HashMap<Uuid, Vec<(Uuid, &host_package::Model)>> = HashMap::new();
    for (history_id, pkg) in &history_items {
        by_config
            .entry(pkg.plugin_config_id)
            .or_default()
            .push((*history_id, pkg));
    }

    for (config_id, items) in by_config {
        let Some(config) = configs.get(&config_id) else {
            tracing::warn!(
                %config_id,
                %batch_id,
                "plugin_config not found for host package group; skipping dispatch"
            );
            continue;
        };

        let plugin_type: uptrakit_internal_wire::PluginType = match serde_json::from_value(
            serde_json::Value::String(config.plugin_type.clone()),
        ) {
            Ok(pt) => pt,
            Err(_) => {
                tracing::warn!(
                    plugin_type = %config.plugin_type,
                    %batch_id,
                    "unknown plugin type for host package group; skipping dispatch"
                );
                continue;
            }
        };

        let updates: Vec<BatchHostPackageUpdate> = items
            .iter()
            .map(|(history_id, pkg)| BatchHostPackageUpdate {
                host_package_id: pkg.id,
                update_history_id: *history_id,
                package_identifier: pkg.package_identifier.clone(),
                to_version: pkg.latest_version.clone().unwrap_or_default(),
                release_info: None,
            })
            .collect();

        let msg = ControllerMessage::ExecuteBatchHostPackageUpdate(Box::new(
            ExecuteBatchHostPackageUpdatePayload {
                host_machine_id: host_record.machine_id.clone(),
                batch_id,
                plugin_type,
                plugin_config: config.config.clone(),
                updates,
                pre_update_hooks: vec![],
                post_update_hooks: vec![],
                timeout_seconds: 7200,
            },
        ));

        notifier.send(&agent.id, msg).await;
    }

    Ok(Some(batch_id))
}
