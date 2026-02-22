use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use uptrakit_shared_db::entity::{host, prelude::*, update_history, update_output_line};
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
        initiated_by: record.initiated_by.clone(),
        started_at: record.started_at,
        completed_at: record.completed_at,
        created_at: record.created_at,
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

/// Collect all host IDs belonging to a tenant (for tenant scoping of update_history).
async fn tenant_host_ids(
    tenant_db: &TenantDb,
) -> Result<Vec<Uuid>, sea_orm::DbErr> {
    let hosts = tenant_db.find::<host::Entity>().all(tenant_db.db()).await?;
    Ok(hosts.into_iter().map(|h| h.id).collect())
}

async fn resolve_host_name(
    db: &sea_orm::DatabaseConnection,
    host_id: Uuid,
) -> String {
    match Host::find_by_id(host_id).one(db).await {
        Ok(Some(h)) => h.friendly_name,
        _ => "Unknown Host".to_string(),
    }
}

async fn resolve_software_item_name(
    db: &sea_orm::DatabaseConnection,
    software_item_id: Uuid,
) -> String {
    match SoftwareItem::find_by_id(software_item_id).one(db).await {
        Ok(Some(si)) => si.name,
        _ => "Unknown Software Item".to_string(),
    }
}

// --- Public query functions ---

pub async fn list_update_history(
    tenant_db: &TenantDb,
    query: &UpdateHistoryQuery,
) -> Result<PaginatedResponse<UpdateHistoryResponse>, sea_orm::DbErr> {
    let pagination = query.pagination().resolve();

    let host_ids = tenant_host_ids(tenant_db).await?;

    if host_ids.is_empty() {
        return Ok(PaginatedResponse::<UpdateHistoryResponse>::new(
            vec![],
            0,
            pagination,
        ));
    }

    let mut q = UpdateHistory::find()
        .filter(update_history::Column::HostId.is_in(host_ids));

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

    let mut items = Vec::with_capacity(records.len());
    for record in records {
        let host_name = resolve_host_name(tenant_db.db(), record.host_id).await;
        let si_name = resolve_software_item_name(tenant_db.db(), record.software_item_id).await;
        items.push(build_response(&record, host_name, si_name, record.output.clone()));
    }

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if the record is not found or its host does not belong to this tenant.
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

    let si_name = resolve_software_item_name(tenant_db.db(), record.software_item_id).await;
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

    Ok(Some(build_response(&record, host.friendly_name, si_name, output)))
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
            initiated_by: "user-123".to_string(),
            started_at: now,
            completed_at: Some(now),
            created_at: now,
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
        assert_eq!(resp.initiated_by, "user-123");
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
            initiated_by: "scheduler".to_string(),
            started_at: now,
            completed_at: Some(now),
            created_at: now,
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
        assert_eq!(resp.initiated_by, "scheduler");
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
            initiated_by: "mqtt".to_string(),
            started_at: now,
            completed_at: None,
            created_at: now,
        };

        let resp = build_response(
            &record,
            "App Host".to_string(),
            "Redis".to_string(),
            String::new(),
        );

        assert_eq!(resp.status, UpdateStatus::Pending);
        assert!(resp.completed_at.is_none());
        assert_eq!(resp.initiated_by, "mqtt");
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
