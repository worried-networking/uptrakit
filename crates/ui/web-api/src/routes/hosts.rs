use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_db::entity::{agent, agent_host, host, prelude::*};
use utoipa::ToSchema;

use crate::routes::agents::AgentStatus;

// --- Response/Request types ---

#[derive(Serialize, ToSchema)]
pub struct HostResponse {
    pub id: String,
    pub machine_id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub os_type: Option<String>,
    pub os_version: Option<String>,
    pub architecture: Option<String>,
    pub ip_address: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub agents: Vec<HostAgentSummary>,
}

#[derive(Serialize, ToSchema)]
pub struct HostAgentSummary {
    pub id: String,
    pub friendly_name: String,
    pub status: AgentStatus,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateHostRequest {
    pub friendly_name: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct HostMessageResponse {
    pub message: String,
}

// --- Endpoints ---

/// List all non-deactivated hosts
#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    responses(
        (status = 200, description = "List of hosts", body = Vec<HostResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Hosts",
    security(("bearer_token" = []))
)]
pub async fn list_hosts(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let hosts = match Host::find()
        .filter(host::Column::DeactivatedAt.is_null())
        .order_by_desc(host::Column::CreatedAt)
        .all(&state.db)
        .await
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to list hosts: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut response = Vec::with_capacity(hosts.len());
    for h in hosts {
        let agents = load_host_agents(&state.db, h.id).await;
        response.push(host_to_response(h, agents));
    }

    (StatusCode::OK, Json(response)).into_response()
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
    security(("bearer_token" = []))
)]
pub async fn get_host(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ViewAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid host ID").into_response(),
    };

    let host_model = match Host::find_by_id(host_id)
        .filter(host::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return (StatusCode::NOT_FOUND, "Host not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let agents = load_host_agents(&state.db, host_id).await;
    (StatusCode::OK, Json(host_to_response(host_model, agents))).into_response()
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
    security(("bearer_token" = []))
)]
pub async fn update_host(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateHostRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid host ID").into_response(),
    };

    let host_model = match Host::find_by_id(host_id)
        .filter(host::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return (StatusCode::NOT_FOUND, "Host not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut active: host::ActiveModel = host_model.into();
    if let Some(ref name) = body.friendly_name {
        active.friendly_name = Set(name.clone());
    }
    active.updated_at = Set(OffsetDateTime::now_utc());

    let updated = match active.update(&state.db).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to update host: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let agents = load_host_agents(&state.db, host_id).await;
    (StatusCode::OK, Json(host_to_response(updated, agents))).into_response()
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
    security(("bearer_token" = []))
)]
pub async fn deactivate_host(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid host ID").into_response(),
    };

    let host_model = match Host::find_by_id(host_id)
        .filter(host::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return (StatusCode::NOT_FOUND, "Host not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active: host::ActiveModel = host_model.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    if let Err(e) = active.update(&state.db).await {
        tracing::error!("Failed to deactivate host: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (
        StatusCode::OK,
        Json(HostMessageResponse {
            message: "Host deactivated".to_string(),
        }),
    )
        .into_response()
}

// --- Helpers ---

fn format_rfc3339(dt: OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn host_to_response(h: host::Model, agents: Vec<HostAgentSummary>) -> HostResponse {
    HostResponse {
        id: h.id.to_string(),
        machine_id: h.machine_id,
        hostname: h.hostname,
        friendly_name: h.friendly_name,
        os_type: h.os_type,
        os_version: h.os_version,
        architecture: h.architecture,
        ip_address: h.ip_address,
        last_seen_at: h.last_seen_at.map(format_rfc3339),
        created_at: format_rfc3339(h.created_at),
        updated_at: format_rfc3339(h.updated_at),
        agents,
    }
}

async fn load_host_agents(
    db: &sea_orm::DatabaseConnection,
    host_id: uuid::Uuid,
) -> Vec<HostAgentSummary> {
    let links = match AgentHost::find()
        .filter(agent_host::Column::HostId.eq(host_id))
        .all(db)
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::warn!("Failed to load host agents: {}", e);
            return Vec::new();
        }
    };

    let mut summaries = Vec::with_capacity(links.len());
    for link in links {
        if let Ok(Some(a)) = Agent::find_by_id(link.agent_id)
            .filter(agent::Column::DeactivatedAt.is_null())
            .one(db)
            .await
        {
            summaries.push(HostAgentSummary {
                id: a.id.to_string(),
                friendly_name: a.friendly_name,
                status: AgentStatus::from_str(&a.status).unwrap_or(AgentStatus::Pending),
            });
        }
    }

    summaries
}
