use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageHosts, CanManageSoftware, CanViewHosts};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::hosts as host_queries;
use crate::routes::agent_ws::trigger_discovery_for_agent_host;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::sync::Arc;
use uptrakit_shared_db::entity::{host, prelude::*, service_host};

pub use uptrakit_web_api_types::autodiscovery::{
    DiscardDiscoveredResponse, TriggerDiscoveryResponse,
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

// ── Autodiscovery endpoints ───────────────────────────────────────────────────

/// Trigger autodiscovery on a specific host.
///
/// Sends `DiscoverSoftware` to all agents that have this host linked.
#[utoipa::path(
    post,
    path = "/api/v1/hosts/{id}/discover",
    params(("id" = String, Path, description = "Host UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Discovery triggered", body = TriggerDiscoveryResponse),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    security(("bearer_token" = []))
)]
pub async fn discover_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
) -> Response {
    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    // Verify host belongs to tenant.
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Find all agents linked to this host.
    let links = match ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .all(tenant_db.db())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to query service-host links: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let agents_notified = links.len() as u32;
    for link in &links {
        trigger_discovery_for_agent_host(
            &state,
            link.service_id,
            tenant_db.tenant_id,
            &host_record.machine_id,
        )
        .await;
    }

    (
        StatusCode::OK,
        Json(TriggerDiscoveryResponse {
            plugins_queued: agents_notified,
            message: format!(
                "Discovery triggered on {} agent(s) for host '{}'",
                agents_notified, host_record.hostname
            ),
        }),
    )
        .into_response()
}

/// Bulk-discard all pending discovered software items for a host.
///
/// Optionally filter by plugin config. No autodiscovery ignore rules are created.
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{id}/discovered",
    params(
        ("id" = String, Path, description = "Host UUID"),
        ("plugin_config_id" = Option<String>, Query, description = "Filter by plugin config UUID")
    ),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Pending items discarded", body = DiscardDiscoveredResponse),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    security(("bearer_token" = []))
)]
pub async fn discard_host_discovered(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
    Query(params): Query<DiscardDiscoveredParams>,
) -> Response {
    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    // Verify host belongs to tenant.
    let exists = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !exists {
        return error_response(StatusCode::NOT_FOUND, "Host not found");
    }

    let plugin_config_id = params
        .plugin_config_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    match autodiscovery_queries::discard_pending_items(
        tenant_db.db(),
        tenant_db.tenant_id,
        Some(host_id),
        plugin_config_id,
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to discard pending items: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[derive(serde::Deserialize, Default)]
pub struct DiscardDiscoveredParams {
    pub plugin_config_id: Option<String>,
}
