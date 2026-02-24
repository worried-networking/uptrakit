use crate::AppState;
use crate::auth::token::generate_uuid;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::provider_configs::find_raw_active_config;
use crate::queries::software_items::{self as item_queries, SoftwareItemQueryError};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, RelationTrait, Set};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::SoftwareDiscoveryState;
use uptrakit_shared_db::entity::{
    host, host_software_item, prelude::*, provider_config, service, service_host, software_item,
    update_history,
};
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, ListSoftwareItemsParams,
    SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse,
    TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus, TriggerVersionCheckResponse,
    UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
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
            "A software item with this name already exists",
        ),
        SoftwareItemQueryError::DuplicateHostAssignment => error_response(
            StatusCode::CONFLICT,
            "This host already has an assignment for the given provider config and package identifier",
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
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("discovery_state" = Option<String>, Query, description = "Filter by discovery state: \"pending\" or \"approved\". Omit to return all items.")
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
    Query(params): Query<ListSoftwareItemsParams>,
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
    params(
        ("id" = String, Path, description = "Software item UUID"),
    ),
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

/// Approve a pending discovered software item.
///
/// Sets `discovery_state = 'approved'` and enables the item for version
/// tracking and update management.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/approve",
    params(("id" = String, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Software item approved", body = SoftwareItemResponse),
        (status = 404, description = "Software item not found"),
        (status = 409, description = "Item is not in pending discovery state")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn approve_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let item = match software_item::Entity::find_by_id(item_id)
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(i)) => i,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
        Err(e) => {
            tracing::error!("Failed to fetch software item: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if item.discovery_state != Some(SoftwareDiscoveryState::Pending) {
        return error_response(
            StatusCode::CONFLICT,
            "Software item is not in pending discovery state",
        );
    }

    let now = OffsetDateTime::now_utc();
    let mut active: software_item::ActiveModel = item.into();
    active.discovery_state = Set(Some(SoftwareDiscoveryState::Approved));
    active.enabled = Set(true);
    active.updated_at = Set(now);

    let updated = match active.update(tenant_db.db()).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to approve software item: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match item_queries::get_software_item(&tenant_db, updated.id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Item not found after update",
        ),
        Err(e) => {
            tracing::error!("Failed to fetch approved software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Assign a software item to additional hosts.
///
/// Each host in `host_assignments` carries its own `provider_config_id`,
/// `package_identifier`, and optional `config_override`.
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

    if req.host_assignments.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "host_assignments must not be empty",
        );
    }

    match item_queries::assign_hosts(&tenant_db, item_id, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => query_error_to_response(e),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct DeleteHostAssignmentParams {
    pub ignore: Option<bool>,
}

/// Unassign a software item from a host.
///
/// The optional `ignore=true` query parameter also creates an autodiscovery
/// ignore rule based on the host assignment's provider config and package
/// identifier, so this combination is not re-discovered in future runs.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}/hosts/{host_id}",
    params(
        ("id" = String, Path, description = "Software item UUID"),
        ("host_id" = String, Path, description = "Host UUID"),
        ("ignore" = Option<bool>, Query, description = "If true, permanently suppress this package/provider combination from future autodiscovery runs")
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
    Query(params): Query<DeleteHostAssignmentParams>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid software item UUID"),
    };

    let host_id = match uuid::Uuid::parse_str(&host_id_str) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host UUID"),
    };

    // If ignore=true, load the assignment before deleting to capture provider info.
    let ignore_info: Option<(uuid::Uuid, String)> = if params.ignore.unwrap_or(false) {
        match host_software_item::Entity::find_by_id((host_id, item_id))
            .one(tenant_db.db())
            .await
        {
            Ok(Some(link)) => Some((link.provider_config_id, link.package_identifier)),
            Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host assignment not found"),
            Err(e) => {
                tracing::error!("Failed to look up host assignment for ignore: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    } else {
        None
    };

    match item_queries::unassign_host(&tenant_db, item_id, host_id).await {
        Ok(true) => {
            if let Some((provider_config_id, package_identifier)) = ignore_info
                && let Err(e) = autodiscovery_queries::create_or_ignore_ignore_rule(
                    tenant_db.db(),
                    tenant_db.tenant_id,
                    provider_config_id,
                    &package_identifier,
                )
                .await
            {
                tracing::warn!("Failed to create autodiscovery ignore rule: {e}");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "Software item or host assignment not found",
        ),
        Err(e) => {
            tracing::error!("Failed to unassign host from software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update the provider assignment for a specific host–software-item link.
#[utoipa::path(
    put,
    path = "/api/v1/software-items/{id}/hosts/{host_id}",
    params(
        ("id" = String, Path, description = "Software item UUID"),
        ("host_id" = String, Path, description = "Host UUID")
    ),
    request_body = UpdateHostAssignmentRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Host assignment updated", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item or host assignment not found"),
        (status = 409, description = "Duplicate host assignment")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn update_host_assignment(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((id, host_id_str)): Path<(String, String)>,
    Json(req): Json<UpdateHostAssignmentRequest>,
) -> Response {
    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid software item UUID"),
    };

    let host_id = match uuid::Uuid::parse_str(&host_id_str) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host UUID"),
    };

    match item_queries::update_host_assignment(&tenant_db, item_id, host_id, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => query_error_to_response(e),
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
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
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

    // 3. Verify host is assigned to software item and load per-host provider info
    let link = match item_queries::load_host_assignment(tenant_db.db(), host_id, item_id).await {
        Some(l) => l,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            );
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

    // 6. Load provider config from the host-specific assignment
    let provider_config = match find_raw_active_config(&tenant_db, link.provider_config_id).await {
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

    // 8. Resolve hooks from provider config + per-host config_override
    let resolved_hooks =
        crate::update_hooks::resolve_hooks(&provider_config.config, link.config_override.as_ref());

    // 9. Merge config
    let merged_config =
        crate::update_hooks::merge_config(&provider_config.config, link.config_override.as_ref());

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
        package_identifier: link.package_identifier.clone(),
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
///
/// Each host receives a version-check message using its own per-host provider config
/// and package identifier.
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
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
            Some(i) => i,
            None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
        };

    // Find all hosts assigned to this software item
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

    // Collect IDs for batch loads.
    let host_ids: Vec<uuid::Uuid> = links.iter().map(|l| l.host_id).collect();
    let config_ids: Vec<uuid::Uuid> = links.iter().map(|l| l.provider_config_id).collect();

    // Batch query 1: Hosts (tenant-scoped).
    let hosts: std::collections::HashMap<uuid::Uuid, host::Model> = match tenant_db
        .find::<host::Entity>()
        .filter(host::Column::Id.is_in(host_ids.clone()))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(hs) => hs.into_iter().map(|h| (h.id, h)).collect(),
        Err(e) => {
            tracing::error!("Failed to load hosts: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Batch query 2: service_host → service JOIN (tenant-scoped, approved services only).
    // Uses find_via_tenant_join to enforce tenant isolation without a separate service query.
    let service_hosts: std::collections::HashMap<uuid::Uuid, uuid::Uuid> = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.is_in(host_ids))
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(service::Column::Status.eq(service::ServiceStatus::Approved))
        .all(tenant_db.db())
        .await
    {
        Ok(shs) => shs
            .into_iter()
            .map(|sh| (sh.host_id, sh.service_id))
            .collect(),
        Err(e) => {
            tracing::error!("Failed to load service-host links: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Batch query 3: Provider configs (tenant-scoped).
    let configs: std::collections::HashMap<uuid::Uuid, provider_config::Model> = match tenant_db
        .find::<provider_config::Entity>()
        .filter(provider_config::Column::Id.is_in(config_ids))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(cs) => cs.into_iter().map(|c| (c.id, c)).collect(),
        Err(e) => {
            tracing::error!("Failed to load provider configs: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut agents_notified: u32 = 0;
    // Deduplicate by (service_id, host_id) — each pair gets exactly one CheckVersions message.
    let mut seen = std::collections::HashSet::new();

    for link in &links {
        let Some(host_record) = hosts.get(&link.host_id) else {
            continue;
        };
        let Some(&service_id) = service_hosts.get(&link.host_id) else {
            continue;
        };
        if !seen.insert((service_id, link.host_id)) {
            continue;
        }
        let Some(prov_config) = configs.get(&link.provider_config_id) else {
            continue;
        };

        let provider_type: uptrakit_internal_wire::ProviderType = match serde_json::from_value(
            serde_json::Value::String(prov_config.provider_type.clone()),
        ) {
            Ok(pt) => pt,
            Err(_) => continue,
        };

        let config =
            crate::update_hooks::merge_config(&prov_config.config, link.config_override.as_ref());

        let assignment = uptrakit_internal_wire::VersionCheckAssignment {
            software_item_id: item_id,
            name: item.name.clone(),
            provider_type,
            package_identifier: link.package_identifier.clone(),
            config,
        };

        let msg = uptrakit_internal_wire::ControllerMessage::CheckVersions(
            uptrakit_internal_wire::CheckVersionsPayload {
                host_machine_id: host_record.machine_id.clone(),
                assignments: vec![assignment],
            },
        );
        state.notification_service.send(&service_id, msg).await;
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
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
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

    // Verify host is assigned and load per-host provider info
    let link = match item_queries::load_host_assignment(tenant_db.db(), host_id, item_id).await {
        Some(l) => l,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            );
        }
    };

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

    // Load the per-host provider config
    let provider_config = match find_raw_active_config(&tenant_db, link.provider_config_id).await {
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

    let config =
        crate::update_hooks::merge_config(&provider_config.config, link.config_override.as_ref());

    let assignment = uptrakit_internal_wire::VersionCheckAssignment {
        software_item_id: item_id,
        name: item.name.clone(),
        provider_type,
        package_identifier: link.package_identifier.clone(),
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
