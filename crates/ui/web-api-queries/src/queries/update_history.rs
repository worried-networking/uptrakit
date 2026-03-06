use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::HashMap;
use uptrakit_shared_db::entity::{
    host, prelude::*, software_item, update_history, update_output_line,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::update_history::{
    UpdateHistoryQuery, UpdateHistoryResponse, UpdateStatus,
};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

// --- Private helpers ---

fn db_status_to_api(status: &update_history::UpdateStatus) -> UpdateStatus {
    match status {
        update_history::UpdateStatus::Pending => UpdateStatus::Pending,
        update_history::UpdateStatus::InProgress => UpdateStatus::InProgress,
        update_history::UpdateStatus::Completed => UpdateStatus::Completed,
        update_history::UpdateStatus::Failed => UpdateStatus::Failed,
        _ => {
            tracing::warn!("Unknown update status encountered, defaulting to Pending");
            UpdateStatus::Pending
        }
    }
}

const UPDATE_OUTPUT_BYTES_CAP: usize = 1_048_576;

fn build_response(
    record: &update_history::Model,
    host_name: String,
    software_item_name: String,
    output: String,
) -> UpdateHistoryResponse {
    UpdateHistoryResponse {
        id: record.id,
        host_id: record.host_id,
        host_name,
        software_item_id: record.software_item_id,
        software_item_name,
        from_version: record.from_version.clone(),
        to_version: record.to_version.clone(),
        status: db_status_to_api(&record.status),
        output,
        actor_type: record.actor_type.clone(),
        actor_id: record.actor_id.clone(),
        started_at: record.started_at,
        completed_at: record.completed_at,
        created_at: record.created_at,
        update_category: record.update_category.clone(),
    }
}

async fn load_output_lines(
    db: &sea_orm::DatabaseConnection,
    update_history_id: Uuid,
) -> Result<String, sea_orm::DbErr> {
    let lines = update_output_line::Entity::find()
        .filter(update_output_line::Column::UpdateHistoryId.eq(update_history_id))
        .order_by_asc(update_output_line::Column::CreatedAt)
        .order_by_asc(update_output_line::Column::Id)
        .all(db)
        .await?;

    let mut output = String::new();
    for line in lines {
        if output.len() >= UPDATE_OUTPUT_BYTES_CAP {
            break;
        }
        let remaining = UPDATE_OUTPUT_BYTES_CAP.saturating_sub(output.len());
        if line.output.len() <= remaining {
            output.push_str(&line.output);
        } else {
            output.push_str(&line.output[..remaining]);
            break;
        }
    }

    Ok(output)
}

// --- Public query functions ---

#[tracing::instrument(skip_all)]
pub async fn list_update_history(
    tenant_db: &TenantDb,
    query: &UpdateHistoryQuery,
) -> Result<PaginatedResponse<UpdateHistoryResponse>, sea_orm::DbErr> {
    let pagination = query.pagination().resolve();

    // Tenant-scoped subquery: filter update_history by host IDs belonging to this tenant.
    // This avoids loading all host IDs into application memory.
    let host_subquery = Query::select()
        .column(host::Column::Id)
        .from(host::Entity)
        .and_where(Expr::col(host::Column::TenantId).eq(tenant_db.tenant_id))
        .to_owned();

    let mut q =
        UpdateHistory::find().filter(update_history::Column::HostId.in_subquery(host_subquery));

    if let Some(host_id) = query.host_id {
        q = q.filter(update_history::Column::HostId.eq(host_id));
    }
    if let Some(software_item_id) = query.software_item_id {
        q = q.filter(update_history::Column::SoftwareItemId.eq(software_item_id));
    }
    if let Some(ref status) = query.status {
        q = q.filter(update_history::Column::Status.eq(status.as_str()));
    }

    let base_query = q.order_by_desc(update_history::Column::CreatedAt);

    let total = base_query.clone().count(tenant_db.db()).await?;

    let records = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    if records.is_empty() {
        return Ok(PaginatedResponse::new(vec![], total, pagination));
    }

    // Batch-load host names and software item names in two queries (no per-record lookups).
    let host_ids: Vec<uuid::Uuid> = records
        .iter()
        .map(|r| r.host_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let si_ids: Vec<uuid::Uuid> = records
        .iter()
        .map(|r| r.software_item_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let host_names: HashMap<uuid::Uuid, String> = Host::find()
        .filter(host::Column::Id.is_in(host_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|h| (h.id, h.friendly_name))
        .collect();

    let si_names: HashMap<uuid::Uuid, String> = SoftwareItem::find()
        .filter(software_item::Column::Id.is_in(si_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|si| (si.id, si.name))
        .collect();

    // Batch-load output lines for records that used the streaming path
    // (inline `output` column is empty for those).  A single query covers all
    // such records instead of one query per record (N+1 avoidance).
    let streamed_ids: Vec<uuid::Uuid> = records
        .iter()
        .filter(|r| r.output.is_empty())
        .map(|r| r.id)
        .collect();

    let all_lines: HashMap<uuid::Uuid, String> = if streamed_ids.is_empty() {
        HashMap::new()
    } else {
        let rows = update_output_line::Entity::find()
            .filter(update_output_line::Column::UpdateHistoryId.is_in(streamed_ids))
            .order_by_asc(update_output_line::Column::CreatedAt)
            .order_by_asc(update_output_line::Column::Id)
            .all(tenant_db.db())
            .await?;

        let mut map: HashMap<uuid::Uuid, String> = HashMap::new();
        for line in rows {
            let entry = map.entry(line.update_history_id).or_default();
            if entry.len() < UPDATE_OUTPUT_BYTES_CAP {
                let remaining = UPDATE_OUTPUT_BYTES_CAP.saturating_sub(entry.len());
                if line.output.len() <= remaining {
                    entry.push_str(&line.output);
                } else {
                    entry.push_str(&line.output[..remaining]);
                }
            }
        }
        map
    };

    let items: Vec<UpdateHistoryResponse> = records
        .iter()
        .map(|record| {
            let host_name = host_names
                .get(&record.host_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Host".to_string());
            let si_name = si_names
                .get(&record.software_item_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Software Item".to_string());
            let output = if record.output.is_empty() {
                all_lines.get(&record.id).cloned().unwrap_or_default()
            } else {
                record.output.clone()
            };
            build_response(record, host_name, si_name, output)
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if the record is not found or its host does not belong to this tenant.
#[tracing::instrument(skip_all)]
pub async fn get_update_history(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<UpdateHistoryResponse>, sea_orm::DbErr> {
    let Some(record) = UpdateHistory::find_by_id(id).one(tenant_db.db()).await? else {
        return Ok(None);
    };

    // Tenant scoping: verify the record's host belongs to this tenant.
    let Some(host) = tenant_db
        .find_by_id::<host::Entity, _>(record.host_id)
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };

    let si_name = match SoftwareItem::find_by_id(record.software_item_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(si)) => si.name,
        _ => "Unknown Software Item".to_string(),
    };
    let output = if record.output.is_empty() {
        match load_output_lines(tenant_db.db(), record.id).await {
            Ok(out) => out,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load update output lines");
                String::new()
            }
        }
    } else {
        record.output.clone()
    };

    Ok(Some(build_response(
        &record,
        host.friendly_name,
        si_name,
        output,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn build_response_completed_status() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            from_version: Some("1.0.0".to_string()),
            to_version: "2.0.0".to_string(),
            status: update_history::UpdateStatus::Completed,
            output: "Update completed successfully".to_string(),
            output_bytes: 28,
            actor_type: "user".to_string(),
            actor_id: "user-123".to_string(),
            started_at: now,
            completed_at: Some(now),
            created_at: now,
            update_category: "unknown".to_string(),
            batch_id: None,
        };

        let resp = build_response(
            &record,
            "Web Server".to_string(),
            "Node.js".to_string(),
            "Update completed successfully".to_string(),
        );

        assert_eq!(resp.host_name, "Web Server");
        assert_eq!(resp.software_item_name, "Node.js");
        assert_eq!(resp.from_version, Some("1.0.0".to_string()));
        assert_eq!(resp.to_version, "2.0.0");
        assert_eq!(resp.status, UpdateStatus::Completed);
        assert_eq!(resp.output, "Update completed successfully");
        assert_eq!(resp.actor_type, "user");
        assert_eq!(resp.actor_id, "user-123");
        assert!(resp.completed_at.is_some());
    }

    #[test]
    fn build_response_failed_status() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            from_version: None,
            to_version: "3.0.0".to_string(),
            status: update_history::UpdateStatus::Failed,
            output: "Error: package not found".to_string(),
            output_bytes: 25,
            actor_type: "scheduler".to_string(),
            actor_id: "".to_string(),
            started_at: now,
            completed_at: Some(now),
            created_at: now,
            update_category: "security".to_string(),
            batch_id: None,
        };

        let resp = build_response(
            &record,
            "DB Server".to_string(),
            "PostgreSQL".to_string(),
            "Error: package not found".to_string(),
        );

        assert_eq!(resp.host_name, "DB Server");
        assert_eq!(resp.software_item_name, "PostgreSQL");
        assert!(resp.from_version.is_none());
        assert_eq!(resp.to_version, "3.0.0");
        assert_eq!(resp.status, UpdateStatus::Failed);
        assert_eq!(resp.output, "Error: package not found");
        assert_eq!(resp.actor_type, "scheduler");
    }

    #[test]
    fn build_response_pending_no_completed_at() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            from_version: Some("1.0.0".to_string()),
            to_version: "1.1.0".to_string(),
            status: update_history::UpdateStatus::Pending,
            output: String::new(),
            output_bytes: 0,
            actor_type: "mqtt".to_string(),
            actor_id: "".to_string(),
            started_at: now,
            completed_at: None,
            created_at: now,
            update_category: "unknown".to_string(),
            batch_id: None,
        };

        let resp = build_response(
            &record,
            "App Host".to_string(),
            "Redis".to_string(),
            String::new(),
        );

        assert_eq!(resp.status, UpdateStatus::Pending);
        assert!(resp.completed_at.is_none());
        assert_eq!(resp.actor_type, "mqtt");
    }

    #[test]
    fn db_status_to_api_maps_all_variants() {
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::Pending),
            UpdateStatus::Pending
        );
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::InProgress),
            UpdateStatus::InProgress
        );
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::Completed),
            UpdateStatus::Completed
        );
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::Failed),
            UpdateStatus::Failed
        );
    }
}
