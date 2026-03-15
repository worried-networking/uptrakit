//! Batch query functions: listing, detail, and status aggregation.

use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use std::collections::HashMap;
use uptrakit_shared_db::entity::{host, prelude::*, software_item, update_batch, update_history};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::update_batches::{
    UpdateBatchDetailResponse, UpdateBatchItemSummary, UpdateBatchListQuery,
    UpdateBatchSummaryResponse,
};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

/// Aggregated status count row returned by the `list_batches` GROUP BY query.
#[derive(Debug, FromQueryResult)]
struct BatchStatusCount {
    batch_id: Option<Uuid>,
    status: String,
    count: i64,
}

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
