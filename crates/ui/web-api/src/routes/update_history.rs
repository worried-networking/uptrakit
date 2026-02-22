use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use uptrakit_shared_db::entity::{host, prelude::*, update_history, update_output_line};

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::update_history::{
    UpdateHistoryQuery, UpdateHistoryResponse, UpdateStatus,
};

// --- Helpers ---

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
    update_history_id: uuid::Uuid,
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

/// Collect all host IDs belonging to a tenant (for tenant scoping).
async fn tenant_host_ids(
    tenant_db: &TenantDb,
) -> Result<Vec<uuid::Uuid>, sea_orm::DbErr> {
    let hosts = tenant_db.find::<host::Entity>()
        .all(tenant_db.db())
        .await?;
    Ok(hosts.into_iter().map(|h| h.id).collect())
}

/// Look up a host name by ID, returning a fallback if not found.
async fn resolve_host_name(db: &sea_orm::DatabaseConnection, host_id: uuid::Uuid) -> String {
    match Host::find_by_id(host_id).one(db).await {
        Ok(Some(h)) => h.friendly_name,
        _ => "Unknown Host".to_string(),
    }
}

/// Look up a software item name by ID, returning a fallback if not found.
async fn resolve_software_item_name(
    db: &sea_orm::DatabaseConnection,
    software_item_id: uuid::Uuid,
) -> String {
    match SoftwareItem::find_by_id(software_item_id).one(db).await {
        Ok(Some(si)) => si.name,
        _ => "Unknown Software Item".to_string(),
    }
}

// --- Endpoints ---

/// List update history records (filterable by host_id, software_item_id, status).
#[utoipa::path(
    get,
    path = "/api/v1/update-history",
    params(
        ("host_id" = Option<String>, Query, description = "Filter by host UUID"),
        ("software_item_id" = Option<String>, Query, description = "Filter by software item UUID"),
        ("status" = Option<String>, Query, description = "Filter by status (pending, in_progress, completed, failed)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of update history records", body = PaginatedResponse<UpdateHistoryResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Update History",
    security(("bearer_token" = []))
)]
pub async fn list_update_history(
    tenant_db: TenantDb,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<UpdateHistoryQuery>,
) -> Response {
    if !user.has_permission(Permission::ViewSoftware) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let pagination = query.pagination().resolve();

    // Tenant scoping: get all host IDs belonging to this tenant
    let host_ids = match tenant_host_ids(&tenant_db).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!("Failed to load tenant hosts: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if host_ids.is_empty() {
        return (
            StatusCode::OK,
            Json(PaginatedResponse::<UpdateHistoryResponse>::new(
                vec![],
                0,
                pagination,
            )),
        )
            .into_response();
    }

    let mut q = UpdateHistory::find().filter(update_history::Column::HostId.is_in(host_ids));

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

    let total = match base_query.clone().count(tenant_db.db()).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to count update history: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let records = match base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to list update history: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut items = Vec::with_capacity(records.len());
    for record in records {
        let host_name = resolve_host_name(tenant_db.db(), record.host_id).await;
        let si_name = resolve_software_item_name(tenant_db.db(), record.software_item_id).await;
        items.push(build_response(
            &record,
            host_name,
            si_name,
            record.output.clone(),
        ));
    }

    (
        StatusCode::OK,
        Json(PaginatedResponse::new(items, total, pagination)),
    )
        .into_response()
}

/// Get a single update history record by ID.
#[utoipa::path(
    get,
    path = "/api/v1/update-history/{id}",
    params(("id" = String, Path, description = "Update history record UUID")),
    responses(
        (status = 200, description = "Update history record", body = UpdateHistoryResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Record not found")
    ),
    tag = "Update History",
    security(("bearer_token" = []))
)]
pub async fn get_update_history(
    tenant_db: TenantDb,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ViewSoftware) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let record_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let record = match UpdateHistory::find_by_id(record_id).one(tenant_db.db()).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Update history record not found");
        }
        Err(e) => {
            tracing::error!("Failed to load update history record: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Tenant scoping: verify the record's host belongs to this tenant
    let host = match tenant_db.find_by_id::<host::Entity, _>(record.host_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Update history record not found");
        }
        Err(e) => {
            tracing::error!("Failed to verify host tenant: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let si_name = resolve_software_item_name(tenant_db.db(), record.software_item_id).await;
    let output = if record.output.is_empty() {
        match load_output_lines(tenant_db.db(), record.id).await {
            Ok(output) => output,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load update output lines");
                String::new()
            }
        }
    } else {
        record.output.clone()
    };
    let resp = build_response(&record, host.friendly_name, si_name, output);
    (StatusCode::OK, Json(resp)).into_response()
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
