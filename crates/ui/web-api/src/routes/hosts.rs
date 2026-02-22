use crate::error_response::error_response;
use crate::middleware::permission::{CanManageHosts, CanViewHosts};
use crate::queries::hosts as host_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub use uptrakit_web_api_types::hosts::{
    HostAgentSummary, HostMessageResponse, HostResponse, UpdateHostRequest,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

// --- Endpoints ---

/// List all non-deactivated hosts
#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of hosts", body = PaginatedResponse<HostResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
pub async fn list_hosts(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Query(params): Query<PaginationParams>,
) -> Response {
    match host_queries::list_hosts(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list hosts: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single host by ID
#[utoipa::path(
    get,
    path = "/api/v1/hosts/{id}",
    params(
        ("id" = String, Path, description = "Host UUID")
    ),
    responses(
        (status = 200, description = "Host details", body = HostResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
pub async fn get_host(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Path(id): Path<String>,
) -> Response {
    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    match host_queries::get_active_host(&tenant_db, host_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a host's friendly name
#[utoipa::path(
    put,
    path = "/api/v1/hosts/{id}",
    params(
        ("id" = String, Path, description = "Host UUID")
    ),
    request_body = UpdateHostRequest,
    responses(
        (status = 200, description = "Host updated", body = HostResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("manage_hosts"))),
    security(("bearer_token" = []))
)]
pub async fn update_host(
    tenant_db: TenantDb,
    CanManageHosts(_user): CanManageHosts,
    Path(id): Path<String>,
    Json(body): Json<UpdateHostRequest>,
) -> Response {
    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    match host_queries::update_host(&tenant_db, host_id, &body).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("Failed to update host: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Deactivate a host (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{id}",
    params(
        ("id" = String, Path, description = "Host UUID")
    ),
    responses(
        (status = 200, description = "Host deactivated", body = HostMessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("manage_hosts"))),
    security(("bearer_token" = []))
)]
pub async fn deactivate_host(
    tenant_db: TenantDb,
    CanManageHosts(_user): CanManageHosts,
    Path(id): Path<String>,
) -> Response {
    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    match host_queries::deactivate_host(&tenant_db, host_id).await {
        Ok(true) => (
            StatusCode::OK,
            Json(HostMessageResponse {
                message: "Host deactivated".to_string(),
            }),
        )
            .into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("Failed to deactivate host: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
