use crate::AppState;
use crate::auth::token::generate_uuid;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::provider_configs::find_raw_active_config;
use crate::queries::software_items::{self as item_queries, SoftwareItemQueryError};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item, prelude::*, service, service_host, update_history,
};
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, SoftwareItemDetailResponse,
    SoftwareItemHostSummary, SoftwareItemResponse, TriggerUpdateRequest, TriggerUpdateResponse,
    TriggerUpdateStatus, TriggerVersionCheckResponse, UpdateSoftwareItemRequest,
};

// --- Error mapping helper ---

fn query_error_to_response(e: SoftwareItemQueryError) -> Response {
    match e {
        SoftwareItemQueryError::NotFound => {
            error_response(StatusCode::NOT_FOUND, "Software item not found")
        }
        SoftwareItemQueryError::EmptyName => {
            error_response(StatusCode::BAD_REQUEST, "name must not be empty")
        }
        SoftwareItemQueryError::ProviderConfigNotFound => error_response(
            StatusCode::BAD_REQUEST,
            "provider_config_id does not reference an active provider config",
        ),
        SoftwareItemQueryError::DuplicateItem => error_response(
            StatusCode::CONFLICT,
            "A software item with this provider_config_id and package_identifier already exists",
        ),
        SoftwareItemQueryError::HostNotFound(id) => error_response(
            StatusCode::BAD_REQUEST,
            format!("Host {id} not found or deactivated"),
        ),
        SoftwareItemQueryError::InvalidPackageIdentifier(msg)
        | SoftwareItemQueryError::InvalidConfigOverride(msg)
        | SoftwareItemQueryError::InvalidInlineProviderConfig(msg) => {
            error_response(StatusCode::BAD_REQUEST, msg)
        }
        SoftwareItemQueryError::Db(e) => {
            tracing::error!("Database error in software items: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// --- Endpoints ---

/// Create a new software item.
#[utoipa::path(
    post,
    path = "/api/v1/software-items",
    request_body = CreateSoftwareItemRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 201, description = "Software item created", body = SoftwareItemResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn create_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Json(req): Json<CreateSoftwareItemRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }
    match item_queries::create_software_item(&tenant_db, req).await {
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(e) => query_error_to_response(e),
    }
}

/// List all active software items (with host count).
#[utoipa::path(
    get,
    path = "/api/v1/software-items",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of software items", body = PaginatedResponse<SoftwareItemResponse>),
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn list_software_items(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<PaginationParams>,
) -> Response {
    match item_queries::list_software_items(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list software items: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a software item with assigned hosts and installed versions.
#[utoipa::path(
    get,
    path = "/api/v1/software-items/{id}",
    params(("id" = String, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Software item details", body = SoftwareItemDetailResponse),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn get_software_item(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(id): Path<String>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };
    match item_queries::get_software_item(&tenant_db, item_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Software item not found"),
        Err(e) => {
            tracing::error!("Failed to get software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a software item (partial update).
#[utoipa::path(
    put,
    path = "/api/v1/software-items/{id}",
    params(("id" = String, Path, description = "Software item UUID")),
    request_body = UpdateSoftwareItemRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Software item updated", body = SoftwareItemResponse),
        (status = 404, description = "Software item not found"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn update_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
    Json(req): Json<UpdateSoftwareItemRequest>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };
    match item_queries::update_software_item(&tenant_db, item_id, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => query_error_to_response(e),
    }
}

/// Soft-delete a software item.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}",
    params(("id" = String, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Software item deleted"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn delete_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };
    match item_queries::delete_software_item(&tenant_db, item_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Software item not found"),
        Err(e) => {
            tracing::error!("Failed to delete software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Assign a software item to additional hosts.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts",
    params(("id" = String, Path, description = "Software item UUID")),
    request_body = AssignHostsRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Hosts assigned", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn assign_hosts(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
    Json(req): Json<AssignHostsRequest>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    if req.host_ids.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "host_ids must not be empty");
    }

    match item_queries::assign_hosts(&tenant_db, item_id, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => query_error_to_response(e),
    }
}

/// Unassign a software item from a host.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}/hosts/{host_id}",
    params(
        ("id" = String, Path, description = "Software item UUID"),
        ("host_id" = String, Path, description = "Host UUID")
    ),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Host unassigned"),
        (status = 404, description = "Software item or host assignment not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn unassign_host(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((id, host_id_str)): Path<(String, String)>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid software item UUID"),
    };

    let host_id = match uuid::Uuid::parse_str(&host_id_str) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host UUID"),
    };

    match item_queries::unassign_host(&tenant_db, item_id, host_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Software item or host assignment not found"),
        Err(e) => {
            tracing::error!("Failed to unassign host from software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Trigger a software update for a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/update",
    params(
        ("id" = String, Path, description = "Software item UUID"),
        ("host_id" = String, Path, description = "Host UUID")
    ),
    request_body = TriggerUpdateRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Update triggered", body = TriggerUpdateResponse),
        (status = 400, description = "Invalid input or validation failed"),
        (status = 404, description = "Software item, host, or agent not found"),
        (status = 409, description = "Update already in progress")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn trigger_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(user): CanManageSoftware,
    Path((id, host_id_str)): Path<(String, String)>,
    Json(req): Json<TriggerUpdateRequest>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid software item UUID"),
    };

    let host_id = match uuid::Uuid::parse_str(&host_id_str) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host UUID"),
    };

    // 1. Verify software item exists and is active
    let item = match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id)
        .await
    {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    // 2. Verify host exists, is active, and belongs to tenant
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("Failed to lookup host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // 3. Verify host is assigned to software item
    let _link = match HostSoftwareItem::find_by_id((host_id, item_id))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(l)) => l,
        Ok(None) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            );
        }
        Err(e) => {
            tracing::error!("Failed to check host-software-item link: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // 4. Find agent linked to host
    let agent_link = match ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(l)) => l,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "No agent linked to this host");
        }
        Err(e) => {
            tracing::error!("Failed to find agent for host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify agent exists and is approved
    let agent = match Service::find_by_id(agent_link.service_id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(a)) => {
            if a.status != service::ServiceStatus::Approved {
                return error_response(StatusCode::BAD_REQUEST, "Agent is not approved");
            }
            a
        }
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Agent not found or deactivated");
        }
        Err(e) => {
            tracing::error!("Failed to lookup agent: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // 5. Check for pending/in_progress updates for this (host_id, software_item_id)
    let existing_update = UpdateHistory::find()
        .filter(update_history::Column::HostId.eq(host_id))
        .filter(update_history::Column::SoftwareItemId.eq(item_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Pending,
            update_history::UpdateStatus::InProgress,
        ]))
        .one(tenant_db.db())
        .await;

    match existing_update {
        Ok(Some(_)) => {
            return error_response(
                StatusCode::CONFLICT,
                "An update is already pending or in progress",
            );
        }
        Err(e) => {
            tracing::error!("Failed to check existing updates: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        Ok(None) => {}
    }

    // 6. Load provider config
    let provider_config = match find_raw_active_config(&tenant_db, item.provider_config_id).await {
        Some(c) => c,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Provider config not found",
            );
        }
    };

    // 7. Create update_history record with status = pending
    let now = OffsetDateTime::now_utc();
    let update_history_id = generate_uuid();
    let update_record = update_history::ActiveModel {
        id: Set(update_history_id),
        host_id: Set(host_id),
        software_item_id: Set(item_id),
        from_version: Set(None),
        to_version: Set(req.to_version.clone()),
        status: Set(update_history::UpdateStatus::Pending),
        output: Set(String::new()),
        output_bytes: Set(0),
        initiated_by: Set(user.user_id.to_string()),
        started_at: Set(now),
        completed_at: Set(None),
        created_at: Set(now),
    };

    if let Err(e) = update_record.insert(tenant_db.db()).await {
        tracing::error!("Failed to create update history record: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // 8. Resolve hooks from provider config + config_override
    let resolved_hooks =
        crate::update_hooks::resolve_hooks(&provider_config.config, item.config_override.as_ref());

    // 9. Merge config
    let merged_config =
        crate::update_hooks::merge_config(&provider_config.config, item.config_override.as_ref());

    // 10. Convert provider type
    let provider_type: uptrakit_internal_wire::ProviderType = match serde_json::from_value(
        serde_json::Value::String(provider_config.provider_type.clone()),
    ) {
        Ok(pt) => pt,
        Err(_) => {
            tracing::error!("Unknown provider type: {}", provider_config.provider_type);
            return error_response(StatusCode::BAD_REQUEST, "Unknown provider type");
        }
    };

    // 11. Build ExecuteUpdatePayload
    let execute_payload = uptrakit_internal_wire::ExecuteUpdatePayload {
        host_machine_id: host_record.machine_id.clone(),
        update_history_id,
        software_item_id: item_id,
        software_item_name: item.name.clone(),
        package_identifier: item.package_identifier.clone(),
        to_version: req.to_version,
        provider_type,
        provider_config: merged_config,
        pre_update_hooks: resolved_hooks.pre_update_hooks,
        post_update_hooks: resolved_hooks.post_update_hooks,
        release_info: req
            .release_info
            .map(|ri| uptrakit_internal_wire::ReleaseInfo {
                tag: ri.tag,
                release_url: ri.release_url,
                assets: ri
                    .assets
                    .into_iter()
                    .map(|a| uptrakit_internal_wire::ReleaseAsset {
                        name: a.name,
                        download_url: a.download_url,
                        size: a.size,
                        content_type: None,
                    })
                    .collect(),
            }),
        timeout_seconds: 300,
    };

    // 12. Check if agent is connected locally and send (also writes outbox for cross-controller delivery)
    let agent_connected = state.service_connections.is_connected(&agent.id).await;
    let msg = uptrakit_internal_wire::ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
    let status = if agent_connected {
        if state.notification_service.send(&agent.id, msg).await {
            tracing::info!(
                update_id = %update_history_id,
                agent_id = %agent.id,
                host = %host_record.friendly_name,
                software = %item.name,
                "update sent to connected agent"
            );
            TriggerUpdateStatus::Pending
        } else {
            tracing::info!(
                update_id = %update_history_id,
                agent_id = %agent.id,
                "agent disconnected during send, update queued"
            );
            TriggerUpdateStatus::Queued
        }
    } else {
        // Agent not connected locally — attempt cross-controller delivery via outbox
        state.notification_service.send(&agent.id, msg).await;
        tracing::info!(
            update_id = %update_history_id,
            agent_id = %agent.id,
            host = %host_record.friendly_name,
            software = %item.name,
            "agent not connected locally, update queued (outbox written for cross-controller delivery)"
        );
        TriggerUpdateStatus::Queued
    };

    let resp = TriggerUpdateResponse {
        update_history_id,
        status,
    };

    (StatusCode::OK, Json(resp)).into_response()
}

/// Trigger a version check for a specific software item across all assigned hosts.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/check-versions",
    params(("id" = String, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found or no agents")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn check_versions(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    // Verify software item exists and is active
    let item = match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id)
        .await
    {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    // Load provider config
    let provider_config = match find_raw_active_config(&tenant_db, item.provider_config_id).await {
        Some(c) => c,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Provider config not found",
            );
        }
    };

    let provider_type: uptrakit_internal_wire::ProviderType = match serde_json::from_value(
        serde_json::Value::String(provider_config.provider_type.clone()),
    ) {
        Ok(pt) => pt,
        Err(_) => {
            tracing::error!("Unknown provider type: {}", provider_config.provider_type);
            return error_response(StatusCode::BAD_REQUEST, "Unknown provider type");
        }
    };

    let config = match item.config_override.as_ref() {
        Some(ovr) => crate::update_hooks::merge_config(&provider_config.config, Some(ovr)),
        None => provider_config.config.clone(),
    };

    // Find all hosts assigned to this software item that have agents
    let links = match HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(tenant_db.db())
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::error!("Failed to load software item hosts: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if links.is_empty() {
        return error_response(
            StatusCode::NOT_FOUND,
            "No hosts assigned to this software item",
        );
    }

    let assignment = uptrakit_internal_wire::VersionCheckAssignment {
        software_item_id: item_id,
        name: item.name.clone(),
        provider_type,
        package_identifier: item.package_identifier.clone(),
        config,
    };

    let mut agents_notified: u32 = 0;
    // Deduplicate by (agent, host) — each pair gets exactly one CheckVersions message.
    let mut seen = std::collections::HashSet::new();

    for link in &links {
        // Load the host to obtain its machine_id for routing.
        let host_record = match Host::find_by_id(link.host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(tenant_db.db())
            .await
        {
            Ok(Some(h)) => h,
            _ => continue,
        };

        // Find agent linked to this host
        let agent_link = match ServiceHost::find()
            .filter(service_host::Column::HostId.eq(link.host_id))
            .one(tenant_db.db())
            .await
        {
            Ok(Some(l)) => l,
            _ => continue,
        };

        if !seen.insert((agent_link.service_id, link.host_id)) {
            continue;
        }

        // Verify agent exists and is approved
        let agent = match Service::find_by_id(agent_link.service_id)
            .filter(service::Column::DeactivatedAt.is_null())
            .one(tenant_db.db())
            .await
        {
            Ok(Some(a)) if a.status == service::ServiceStatus::Approved => a,
            _ => continue,
        };

        let msg = uptrakit_internal_wire::ControllerMessage::CheckVersions(
            uptrakit_internal_wire::CheckVersionsPayload {
                host_machine_id: host_record.machine_id.clone(),
                assignments: vec![assignment.clone()],
            },
        );
        state.notification_service.send(&agent.id, msg).await;
        agents_notified += 1;
    }

    if agents_notified == 0 {
        return error_response(
            StatusCode::NOT_FOUND,
            "No approved agents found for assigned hosts",
        );
    }

    let resp = TriggerVersionCheckResponse {
        agents_notified,
        message: format!(
            "Version check triggered for '{}' on {agents_notified} agent(s)",
            item.name
        ),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

/// Trigger a version check for a specific software item on a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/check-versions",
    params(
        ("id" = String, Path, description = "Software item UUID"),
        ("host_id" = String, Path, description = "Host UUID")
    ),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item, host, or agent not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn check_versions_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((id, host_id_str)): Path<(String, String)>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid software item UUID"),
    };

    let host_id = match uuid::Uuid::parse_str(&host_id_str) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host UUID"),
    };

    // Verify software item exists and is active
    let item = match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id)
        .await
    {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    // Verify host exists and belongs to tenant; keep the record for machine_id.
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("Failed to lookup host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify host is assigned to software item
    match HostSoftwareItem::find_by_id((host_id, item_id))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            );
        }
        Err(e) => {
            tracing::error!("Failed to check host-software-item link: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Find agent linked to host
    let agent_link = match ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(l)) => l,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "No agent linked to this host");
        }
        Err(e) => {
            tracing::error!("Failed to find agent for host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify agent exists and is approved
    let agent = match Service::find_by_id(agent_link.service_id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(a)) => {
            if a.status != service::ServiceStatus::Approved {
                return error_response(StatusCode::BAD_REQUEST, "Agent is not approved");
            }
            a
        }
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Agent not found or deactivated");
        }
        Err(e) => {
            tracing::error!("Failed to lookup agent: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Load provider config
    let provider_config = match find_raw_active_config(&tenant_db, item.provider_config_id).await {
        Some(c) => c,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Provider config not found",
            );
        }
    };

    let provider_type: uptrakit_internal_wire::ProviderType = match serde_json::from_value(
        serde_json::Value::String(provider_config.provider_type.clone()),
    ) {
        Ok(pt) => pt,
        Err(_) => {
            tracing::error!("Unknown provider type: {}", provider_config.provider_type);
            return error_response(StatusCode::BAD_REQUEST, "Unknown provider type");
        }
    };

    let config = match item.config_override.as_ref() {
        Some(ovr) => crate::update_hooks::merge_config(&provider_config.config, Some(ovr)),
        None => provider_config.config.clone(),
    };

    let assignment = uptrakit_internal_wire::VersionCheckAssignment {
        software_item_id: item_id,
        name: item.name.clone(),
        provider_type,
        package_identifier: item.package_identifier.clone(),
        config,
    };

    let msg = uptrakit_internal_wire::ControllerMessage::CheckVersions(
        uptrakit_internal_wire::CheckVersionsPayload {
            host_machine_id: host_record.machine_id.clone(),
            assignments: vec![assignment],
        },
    );
    state.notification_service.send(&agent.id, msg).await;

    let resp = TriggerVersionCheckResponse {
        agents_notified: 1,
        message: format!("Version check triggered for '{}' on 1 agent", item.name),
    };
    (StatusCode::OK, Json(resp)).into_response()
}
