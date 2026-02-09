use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use crate::routes::services::ServiceStatus;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_db::entity::{
    host,
    prelude::{Host, Service as Agent, ServiceHost as AgentHost},
    service as agent, service_host as agent_host,
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
    security(("bearer_token" = []))
)]
pub async fn list_hosts(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Query(params): Query<PaginationParams>,
) -> Response {
    if !user.has_permission(Permission::ViewAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let pagination = params.resolve();

    let base_query = Host::find()
        .filter(host::Column::TenantId.eq(tenant.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .order_by_desc(host::Column::CreatedAt);

    let total = match base_query.clone().count(&state.db).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to count hosts: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hosts = match base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(&state.db)
        .await
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to list hosts: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut items = Vec::with_capacity(hosts.len());
    for h in hosts {
        let agents = load_host_agents(&state.db, h.id, tenant.tenant_id).await;
        items.push(host_to_response(h, agents));
    }

    (
        StatusCode::OK,
        Json(PaginatedResponse::new(items, total, pagination)),
    )
        .into_response()
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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ViewAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    let host_model = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let agents = load_host_agents(&state.db, host_id, tenant.tenant_id).await;
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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateHostRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    let host_model = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let agents = load_host_agents(&state.db, host_id, tenant.tenant_id).await;
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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let host_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host ID"),
    };

    let host_model = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active: host::ActiveModel = host_model.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    if let Err(e) = active.update(&state.db).await {
        tracing::error!("Failed to deactivate host: {}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
    tenant_id: uuid::Uuid,
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

    let service_ids: Vec<uuid::Uuid> = links.into_iter().map(|link| link.service_id).collect();
    if service_ids.is_empty() {
        return Vec::new();
    }

    let agents = match Agent::find()
        .filter(agent::Column::Id.is_in(service_ids))
        .filter(agent::Column::TenantId.eq(tenant_id))
        .filter(agent::Column::DeactivatedAt.is_null())
        .all(db)
        .await
    {
        Ok(agents) => agents,
        Err(e) => {
            tracing::warn!("Failed to load host agents: {}", e);
            return Vec::new();
        }
    };

    agents
        .into_iter()
        .map(|agent| HostAgentSummary {
            id: agent.id.to_string(),
            friendly_name: agent.friendly_name,
            status: match agent.status {
                agent::ServiceStatus::Pending => ServiceStatus::Pending,
                agent::ServiceStatus::Approved => ServiceStatus::Approved,
                agent::ServiceStatus::Rejected => ServiceStatus::Rejected,
                agent::ServiceStatus::Deactivated => ServiceStatus::Deactivated,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use uptrakit_shared_db::entity::{service, service_host};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn setup_test_db() -> DatabaseConnection {
        let db = test_db().await;

        db.execute_unprepared(
            "CREATE TABLE services (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                service_type TEXT NOT NULL,
                hostname TEXT NOT NULL,
                friendly_name TEXT NOT NULL,
                ip_address TEXT,
                status TEXT NOT NULL,
                enrollment_secret_hash TEXT NOT NULL,
                client_version TEXT,
                last_seen_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deactivated_at INTEGER
            )",
        )
        .await
        .unwrap();

        db.execute_unprepared(
            "CREATE TABLE service_hosts (
                service_id TEXT NOT NULL,
                host_id TEXT NOT NULL,
                linked_at INTEGER NOT NULL,
                PRIMARY KEY (service_id, host_id)
            )",
        )
        .await
        .unwrap();

        db
    }

    #[tokio::test]
    async fn load_host_agents_filters_by_tenant() {
        let db = setup_test_db().await;
        let now = OffsetDateTime::now_utc();
        let host_id = uuid::Uuid::now_v7();
        let tenant_a = uuid::Uuid::now_v7();
        let tenant_b = uuid::Uuid::now_v7();

        let service_a = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_a),
            service_type: Set(service::ServiceType::Agent),
            hostname: Set("host-a".to_string()),
            friendly_name: Set("Agent A".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set("hash-a".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        let service_a = service_a.insert(&db).await.unwrap();

        let service_b = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_b),
            service_type: Set(service::ServiceType::Agent),
            hostname: Set("host-b".to_string()),
            friendly_name: Set("Agent B".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set("hash-b".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        let service_b = service_b.insert(&db).await.unwrap();

        let link_a = service_host::ActiveModel {
            service_id: Set(service_a.id),
            host_id: Set(host_id),
            linked_at: Set(now),
        };
        link_a.insert(&db).await.unwrap();

        let link_b = service_host::ActiveModel {
            service_id: Set(service_b.id),
            host_id: Set(host_id),
            linked_at: Set(now),
        };
        link_b.insert(&db).await.unwrap();

        let agents = load_host_agents(&db, host_id, tenant_a).await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].friendly_name, "Agent A");
    }
}
