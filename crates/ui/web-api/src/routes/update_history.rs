use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_db::entity::{host, prelude::*, update_history};

pub use uptrakit_web_api_types::update_history::{
    UpdateHistoryQuery, UpdateHistoryResponse, UpdateStatus,
};

// --- Helpers ---

fn format_rfc3339(dt: OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn db_status_to_api(status: &update_history::UpdateStatus) -> UpdateStatus {
    match status {
        update_history::UpdateStatus::Pending => UpdateStatus::Pending,
        update_history::UpdateStatus::InProgress => UpdateStatus::InProgress,
        update_history::UpdateStatus::Completed => UpdateStatus::Completed,
        update_history::UpdateStatus::Failed => UpdateStatus::Failed,
    }
}

fn build_response(
    record: update_history::Model,
    host_name: String,
    software_item_name: String,
) -> UpdateHistoryResponse {
    UpdateHistoryResponse {
        id: record.id.to_string(),
        host_id: record.host_id.to_string(),
        host_name,
        software_item_id: record.software_item_id.to_string(),
        software_item_name,
        from_version: record.from_version,
        to_version: record.to_version,
        status: db_status_to_api(&record.status),
        output: record.output,
        initiated_by: record.initiated_by,
        started_at: format_rfc3339(record.started_at),
        completed_at: record.completed_at.map(format_rfc3339),
        created_at: format_rfc3339(record.created_at),
    }
}

/// Collect all host IDs belonging to a tenant (for tenant scoping).
async fn tenant_host_ids(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> Result<Vec<uuid::Uuid>, sea_orm::DbErr> {
    let hosts = Host::find()
        .filter(host::Column::TenantId.eq(tenant_id))
        .all(db)
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
        ("status" = Option<String>, Query, description = "Filter by status (pending, in_progress, completed, failed)")
    ),
    responses(
        (status = 200, description = "List of update history records", body = Vec<UpdateHistoryResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Update History",
    security(("bearer_token" = []))
)]
pub async fn list_update_history(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<UpdateHistoryQuery>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    // Tenant scoping: get all host IDs belonging to this tenant
    let host_ids = match tenant_host_ids(&state.db, tenant.tenant_id).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!("Failed to load tenant hosts: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if host_ids.is_empty() {
        return (StatusCode::OK, Json(Vec::<UpdateHistoryResponse>::new())).into_response();
    }

    let mut q = UpdateHistory::find().filter(update_history::Column::HostId.is_in(host_ids));

    if let Some(ref host_id_str) = query.host_id {
        match uuid::Uuid::parse_str(host_id_str) {
            Ok(id) => {
                q = q.filter(update_history::Column::HostId.eq(id));
            }
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host_id UUID"),
        }
    }

    if let Some(ref si_id_str) = query.software_item_id {
        match uuid::Uuid::parse_str(si_id_str) {
            Ok(id) => {
                q = q.filter(update_history::Column::SoftwareItemId.eq(id));
            }
            Err(_) => {
                return error_response(StatusCode::BAD_REQUEST, "Invalid software_item_id UUID");
            }
        }
    }

    if let Some(ref status) = query.status {
        q = q.filter(update_history::Column::Status.eq(status.as_str()));
    }

    let records = match q
        .order_by_desc(update_history::Column::CreatedAt)
        .all(&state.db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to list update history: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut response = Vec::with_capacity(records.len());
    for record in records {
        let host_name = resolve_host_name(&state.db, record.host_id).await;
        let si_name = resolve_software_item_name(&state.db, record.software_item_id).await;
        response.push(build_response(record, host_name, si_name));
    }

    (StatusCode::OK, Json(response)).into_response()
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
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let record_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let record = match UpdateHistory::find_by_id(record_id).one(&state.db).await {
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
    let host = match Host::find_by_id(record.host_id)
        .filter(host::Column::TenantId.eq(tenant.tenant_id))
        .one(&state.db)
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

    let si_name = resolve_software_item_name(&state.db, record.software_item_id).await;
    let resp = build_response(record, host.friendly_name, si_name);
    (StatusCode::OK, Json(resp)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            initiated_by: "user-123".to_string(),
            started_at: now,
            completed_at: Some(now),
            created_at: now,
        };

        let resp = build_response(record, "Web Server".to_string(), "Node.js".to_string());

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
            initiated_by: "scheduler".to_string(),
            started_at: now,
            completed_at: Some(now),
            created_at: now,
        };

        let resp = build_response(record, "DB Server".to_string(), "PostgreSQL".to_string());

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
            initiated_by: "mqtt".to_string(),
            started_at: now,
            completed_at: None,
            created_at: now,
        };

        let resp = build_response(record, "App Host".to_string(), "Redis".to_string());

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
