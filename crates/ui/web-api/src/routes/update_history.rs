use crate::error_response::error_response;
use crate::middleware::permission::CanViewSoftware;
use crate::queries::update_history as uh_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::update_history::{
    UpdateHistoryQuery, UpdateHistoryResponse, UpdateStatus,
};

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
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn list_update_history(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(query): Query<UpdateHistoryQuery>,
) -> Response {
    match uh_queries::list_update_history(&tenant_db, &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list update history: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single update history record by ID.
#[utoipa::path(
    get,
    path = "/api/v1/update-history/{id}",
    params(("id" = Uuid, Path, description = "Update history record UUID")),
    responses(
        (status = 200, description = "Update history record", body = UpdateHistoryResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Record not found")
    ),
    tag = "Update History",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn get_update_history(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(record_id): Path<Uuid>,
) -> Response {
    match uh_queries::get_update_history(&tenant_db, record_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Update history record not found"),
        Err(e) => {
            tracing::error!("Failed to get update history record: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
