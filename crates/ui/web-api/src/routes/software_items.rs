use crate::AppState;
use crate::auth::permissions::Permission;
use crate::auth::token::generate_uuid;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_db::entity::{
    host, host_software_item, prelude::*, provider_config, service, service_host, software_item,
    update_history,
};

use uptrakit_provider_registry::ProviderRegistry;

pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, SoftwareItemDetailResponse,
    SoftwareItemHostSummary, SoftwareItemResponse, TriggerUpdateRequest, TriggerUpdateResponse,
    TriggerUpdateStatus, UpdateSoftwareItemRequest,
};

// --- Helpers ---

fn format_rfc3339(dt: OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn build_list_response(
    item: software_item::Model,
    config: &provider_config::Model,
    host_count: u64,
) -> SoftwareItemResponse {
    SoftwareItemResponse {
        id: item.id.to_string(),
        name: item.name,
        provider_config_id: item.provider_config_id.to_string(),
        provider_config_name: config.name.clone(),
        provider_type: config.provider_type.clone(),
        package_identifier: item.package_identifier,
        config_override: item.config_override,
        enabled: item.enabled,
        last_checked_at: item.last_checked_at.map(format_rfc3339),
        host_count,
        created_at: format_rfc3339(item.created_at),
        updated_at: format_rfc3339(item.updated_at),
    }
}

fn build_detail_response(
    item: software_item::Model,
    config: &provider_config::Model,
    host_count: u64,
    hosts: Vec<SoftwareItemHostSummary>,
) -> SoftwareItemDetailResponse {
    SoftwareItemDetailResponse {
        id: item.id.to_string(),
        name: item.name,
        provider_config_id: item.provider_config_id.to_string(),
        provider_config_name: config.name.clone(),
        provider_type: config.provider_type.clone(),
        package_identifier: item.package_identifier,
        config_override: item.config_override,
        enabled: item.enabled,
        last_checked_at: item.last_checked_at.map(format_rfc3339),
        host_count,
        created_at: format_rfc3339(item.created_at),
        updated_at: format_rfc3339(item.updated_at),
        hosts,
    }
}

/// Find a non-deactivated software item by ID, scoped to a tenant.
async fn find_active_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
) -> Option<software_item::Model> {
    SoftwareItem::find_by_id(id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Find a non-deactivated provider config by ID, scoped to a tenant.
async fn find_active_provider_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
) -> Option<provider_config::Model> {
    ProviderConfig::find_by_id(id)
        .filter(provider_config::Column::TenantId.eq(tenant_id))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Count the number of hosts linked to a software item.
async fn count_linked_hosts(db: &sea_orm::DatabaseConnection, item_id: uuid::Uuid) -> u64 {
    HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .count(db)
        .await
        .unwrap_or(0)
}

/// Load host summaries for a software item.
async fn load_item_hosts(
    db: &sea_orm::DatabaseConnection,
    item_id: uuid::Uuid,
) -> Vec<SoftwareItemHostSummary> {
    let links = match HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(db)
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::warn!("Failed to load software item hosts: {e}");
            return Vec::new();
        }
    };

    let mut summaries = Vec::with_capacity(links.len());
    for link in links {
        if let Ok(Some(h)) = Host::find_by_id(link.host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(db)
            .await
        {
            summaries.push(SoftwareItemHostSummary {
                host_id: h.id.to_string(),
                hostname: h.hostname,
                friendly_name: h.friendly_name,
                installed_version: link.installed_version,
                installed_version_detected_at: link
                    .installed_version_detected_at
                    .map(format_rfc3339),
                last_updated_at: link.last_updated_at.map(format_rfc3339),
                linked_at: format_rfc3339(link.linked_at),
            });
        }
    }

    summaries
}

/// Error returned when config override validation fails.
#[derive(Debug, thiserror::Error)]
enum ConfigOverrideError {
    #[error("config_override must be a JSON object")]
    NotAnObject,
    #[error("provider validation failed: {0}")]
    ProviderValidation(String),
}

/// Validate `config_override` by merging it with the base provider config and running
/// provider-specific validation.
fn validate_config_override(
    provider_type: &str,
    base_config: &serde_json::Value,
    override_config: &serde_json::Value,
) -> std::result::Result<(), ConfigOverrideError> {
    // Merge: base first, then overlay the override values
    let mut merged = base_config.clone();
    if let (Some(base_obj), Some(over_obj)) = (merged.as_object_mut(), override_config.as_object())
    {
        for (k, v) in over_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    } else {
        return Err(ConfigOverrideError::NotAnObject);
    }

    ProviderRegistry::validate_config_str(provider_type, &merged)
        .map_err(|e| ConfigOverrideError::ProviderValidation(e.to_string()))
}

// --- Endpoints ---

/// Create a new software item.
#[utoipa::path(
    post,
    path = "/api/v1/software-items",
    request_body = CreateSoftwareItemRequest,
    responses(
        (status = 201, description = "Software item created", body = SoftwareItemResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn create_software_item(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Json(req): Json<CreateSoftwareItemRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    if req.name.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "name must not be empty");
    }

    let provider_config_id = match uuid::Uuid::parse_str(&req.provider_config_id) {
        Ok(id) => id,
        Err(_) => {
            return error_response(StatusCode::BAD_REQUEST, "Invalid provider_config_id UUID");
        }
    };

    let config =
        match find_active_provider_config(&state.db, tenant.tenant_id, provider_config_id).await {
            Some(c) => c,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "provider_config_id does not reference an active provider config",
                );
            }
        };

    let package_identifier = req.package_identifier.unwrap_or_default();

    // Validate config_override if provided
    if let Some(ref override_val) = req.config_override
        && let Err(e) =
            validate_config_override(&config.provider_type, &config.config, override_val)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("config_override validation failed: {e}"),
        );
    }

    // Check uniqueness: (provider_config_id, package_identifier) among active items
    let duplicate = SoftwareItem::find()
        .filter(software_item::Column::ProviderConfigId.eq(provider_config_id))
        .filter(software_item::Column::PackageIdentifier.eq(&package_identifier))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await;

    match duplicate {
        Ok(Some(_)) => {
            return error_response(
                StatusCode::CONFLICT,
                "A software item with this provider_config_id and package_identifier already exists",
            );
        }
        Err(e) => {
            tracing::error!("Failed to check for duplicate software item: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        Ok(None) => {}
    }

    let now = OffsetDateTime::now_utc();
    let model = software_item::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant.tenant_id),
        name: Set(req.name),
        provider_config_id: Set(provider_config_id),
        package_identifier: Set(package_identifier),
        config_override: Set(req.config_override),
        enabled: Set(req.enabled),
        last_checked_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    match model.insert(&state.db).await {
        Ok(inserted) => {
            let resp = build_list_response(inserted, &config, 0);
            (StatusCode::CREATED, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
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
    responses(
        (status = 200, description = "Paginated list of software items", body = PaginatedResponse<SoftwareItemResponse>),
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn list_software_items(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Query(params): Query<PaginationParams>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let pagination = params.resolve();

    let base_query = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .order_by_asc(software_item::Column::Name);

    let total = match base_query.clone().count(&state.db).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to count software items: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let items = match base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(&state.db)
        .await
    {
        Ok(items) => items,
        Err(e) => {
            tracing::error!("Failed to list software items: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut response = Vec::with_capacity(items.len());
    for item in items {
        let config =
            match find_active_provider_config(&state.db, tenant.tenant_id, item.provider_config_id)
                .await
            {
                Some(c) => c,
                None => {
                    tracing::warn!(
                        "Software item {} references missing provider config {}",
                        item.id,
                        item.provider_config_id
                    );
                    continue;
                }
            };
        let host_count = count_linked_hosts(&state.db, item.id).await;
        response.push(build_list_response(item, &config, host_count));
    }

    (
        StatusCode::OK,
        Json(PaginatedResponse::new(response, total, pagination)),
    )
        .into_response()
}

/// Get a software item with assigned hosts and installed versions.
#[utoipa::path(
    get,
    path = "/api/v1/software-items/{id}",
    params(("id" = String, Path, description = "Software item UUID")),
    responses(
        (status = 200, description = "Software item details", body = SoftwareItemDetailResponse),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn get_software_item(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let item = match find_active_item(&state.db, tenant.tenant_id, item_id).await {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    let config =
        match find_active_provider_config(&state.db, tenant.tenant_id, item.provider_config_id)
            .await
        {
            Some(c) => c,
            None => {
                tracing::error!(
                    "Software item {} references missing provider config {}",
                    item.id,
                    item.provider_config_id
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let hosts = load_item_hosts(&state.db, item_id).await;
    let host_count = hosts.len() as u64;
    let resp = build_detail_response(item, &config, host_count, hosts);
    (StatusCode::OK, Json(resp)).into_response()
}

/// Update a software item (partial update).
#[utoipa::path(
    put,
    path = "/api/v1/software-items/{id}",
    params(("id" = String, Path, description = "Software item UUID")),
    request_body = UpdateSoftwareItemRequest,
    responses(
        (status = 200, description = "Software item updated", body = SoftwareItemResponse),
        (status = 404, description = "Software item not found"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn update_software_item(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSoftwareItemRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let existing = match find_active_item(&state.db, tenant.tenant_id, item_id).await {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    let config =
        match find_active_provider_config(&state.db, tenant.tenant_id, existing.provider_config_id)
            .await
        {
            Some(c) => c,
            None => {
                tracing::error!(
                    "Software item {} references missing provider config {}",
                    existing.id,
                    existing.provider_config_id
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    // Determine the effective package_identifier for uniqueness check
    let new_package_id = req
        .package_identifier
        .as_deref()
        .unwrap_or(&existing.package_identifier);

    // If package_identifier is changing, check uniqueness
    if new_package_id != existing.package_identifier {
        let duplicate = SoftwareItem::find()
            .filter(software_item::Column::ProviderConfigId.eq(existing.provider_config_id))
            .filter(software_item::Column::PackageIdentifier.eq(new_package_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(software_item::Column::Id.ne(item_id))
            .one(&state.db)
            .await;

        match duplicate {
            Ok(Some(_)) => {
                return error_response(
                    StatusCode::CONFLICT,
                    "A software item with this provider_config_id and package_identifier already exists",
                );
            }
            Err(e) => {
                tracing::error!("Failed to check for duplicate software item: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            Ok(None) => {}
        }
    }

    // Validate config_override if provided (non-null value means replace, null means clear)
    if let Some(ref override_val) = req.config_override
        && !override_val.is_null()
        && let Err(e) =
            validate_config_override(&config.provider_type, &config.config, override_val)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("config_override validation failed: {e}"),
        );
    }

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = existing.into();

    if let Some(name) = req.name {
        if name.is_empty() {
            return error_response(StatusCode::BAD_REQUEST, "name must not be empty");
        }
        model.name = Set(name);
    }
    if let Some(package_identifier) = req.package_identifier {
        model.package_identifier = Set(package_identifier);
    }
    if let Some(config_override) = req.config_override {
        if config_override.is_null() {
            model.config_override = Set(None);
        } else {
            model.config_override = Set(Some(config_override));
        }
    }
    if let Some(enabled) = req.enabled {
        model.enabled = Set(enabled);
    }
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(updated) => {
            let host_count = count_linked_hosts(&state.db, item_id).await;
            let resp = build_list_response(updated, &config, host_count);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to update software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Soft-delete a software item.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}",
    params(("id" = String, Path, description = "Software item UUID")),
    responses(
        (status = 204, description = "Software item deleted"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn delete_software_item(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let item = match find_active_item(&state.db, tenant.tenant_id, item_id).await {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = item.into();
    model.deactivated_at = Set(Some(now));
    model.enabled = Set(false);
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("Failed to soft-delete software item: {e}");
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
    responses(
        (status = 200, description = "Hosts assigned", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn assign_hosts(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(req): Json<AssignHostsRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let item = match find_active_item(&state.db, tenant.tenant_id, item_id).await {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    if req.host_ids.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "host_ids must not be empty");
    }

    let now = OffsetDateTime::now_utc();

    for host_id_str in &req.host_ids {
        let host_id = match uuid::Uuid::parse_str(host_id_str) {
            Ok(id) => id,
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    format!("Invalid host UUID: {host_id_str}"),
                );
            }
        };

        // Verify host exists and is active
        let host_exists = Host::find_by_id(host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(&state.db)
            .await;

        match host_exists {
            Ok(Some(_)) => {}
            Ok(None) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    format!("Host {host_id_str} not found or deactivated"),
                );
            }
            Err(e) => {
                tracing::error!("Failed to check host {host_id_str}: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }

        // Check if link already exists
        let existing_link = HostSoftwareItem::find_by_id((host_id, item_id))
            .one(&state.db)
            .await;

        match existing_link {
            Ok(Some(_)) => {
                // Already linked, skip
                continue;
            }
            Ok(None) => {
                let link = host_software_item::ActiveModel {
                    host_id: Set(host_id),
                    software_item_id: Set(item_id),
                    installed_version: Set(None),
                    installed_version_detected_at: Set(None),
                    last_updated_at: Set(None),
                    linked_at: Set(now),
                };
                if let Err(e) = link.insert(&state.db).await {
                    tracing::error!("Failed to link host {host_id_str} to software item: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to check existing link for host {host_id_str}: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    }

    // Reload the item to reflect the current state
    let config =
        match find_active_provider_config(&state.db, tenant.tenant_id, item.provider_config_id)
            .await
        {
            Some(c) => c,
            None => {
                tracing::error!(
                    "Software item {} references missing provider config {}",
                    item.id,
                    item.provider_config_id
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let hosts = load_item_hosts(&state.db, item_id).await;
    let host_count = hosts.len() as u64;
    let resp = build_detail_response(item, &config, host_count, hosts);
    (StatusCode::OK, Json(resp)).into_response()
}

/// Unassign a software item from a host.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}/hosts/{host_id}",
    params(
        ("id" = String, Path, description = "Software item UUID"),
        ("host_id" = String, Path, description = "Host UUID")
    ),
    responses(
        (status = 204, description = "Host unassigned"),
        (status = 404, description = "Software item or link not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn unassign_host(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path((id, host_id_str)): Path<(String, String)>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid software item UUID"),
    };

    let host_id = match uuid::Uuid::parse_str(&host_id_str) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host UUID"),
    };

    // Verify the software item exists
    if find_active_item(&state.db, tenant.tenant_id, item_id)
        .await
        .is_none()
    {
        return error_response(StatusCode::NOT_FOUND, "Software item not found");
    }

    // Find and delete the link
    let link = match HostSoftwareItem::find_by_id((host_id, item_id))
        .one(&state.db)
        .await
    {
        Ok(Some(l)) => l,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "Host is not assigned to this software item",
            );
        }
        Err(e) => {
            tracing::error!("Failed to find host-software-item link: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match link.delete(&state.db).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("Failed to delete host-software-item link: {e}");
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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path((id, host_id_str)): Path<(String, String)>,
    Json(req): Json<TriggerUpdateRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let item_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid software item UUID"),
    };

    let host_id = match uuid::Uuid::parse_str(&host_id_str) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid host UUID"),
    };

    // 1. Verify software item exists and is active
    let item = match find_active_item(&state.db, tenant.tenant_id, item_id).await {
        Some(i) => i,
        None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
    };

    // 2. Verify host exists, is active, and belongs to tenant
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(&state.db)
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
        .one(&state.db)
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
        .one(&state.db)
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
        .one(&state.db)
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
        .one(&state.db)
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
    let provider_config =
        match find_active_provider_config(&state.db, tenant.tenant_id, item.provider_config_id)
            .await
        {
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
    let update_history_id = crate::auth::token::generate_uuid();
    let update_record = update_history::ActiveModel {
        id: Set(update_history_id),
        host_id: Set(host_id),
        software_item_id: Set(item_id),
        from_version: Set(None), // Will be updated when agent reports
        to_version: Set(req.to_version.clone()),
        status: Set(update_history::UpdateStatus::Pending),
        output: Set(String::new()),
        initiated_by: Set(user.user_id.to_string()),
        started_at: Set(now),
        completed_at: Set(None),
        created_at: Set(now),
    };

    if let Err(e) = update_record.insert(&state.db).await {
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

    // Determine shell type - use pre_update shell if available, otherwise post_update shell
    let shell = if !resolved_hooks.pre_update_commands.is_empty() {
        Some(super::agent_ws::wire_hook_shell(
            resolved_hooks.pre_update_shell,
        ))
    } else if !resolved_hooks.post_update_commands.is_empty() {
        Some(super::agent_ws::wire_hook_shell(
            resolved_hooks.post_update_shell,
        ))
    } else {
        None
    };

    // 11. Build ExecuteUpdatePayload
    let execute_payload = uptrakit_internal_wire::ExecuteUpdatePayload {
        update_history_id,
        software_item_id: item_id,
        software_item_name: item.name.clone(),
        package_identifier: item.package_identifier.clone(),
        to_version: req.to_version,
        provider_type,
        provider_config: merged_config,
        pre_update_commands: resolved_hooks.pre_update_commands,
        post_update_commands: resolved_hooks.post_update_commands,
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
        timeout_seconds: 300, // Default timeout
        shell,
    };

    // 12. Check if agent is connected and send
    let agent_connected = state.service_connections.is_connected(&agent.id).await;
    let status = if agent_connected {
        let msg =
            uptrakit_internal_wire::ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
        if state.service_connections.send(&agent.id, msg).await {
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
        tracing::info!(
            update_id = %update_history_id,
            agent_id = %agent.id,
            host = %host_record.friendly_name,
            software = %item.name,
            "agent offline, update queued for reconnect"
        );
        TriggerUpdateStatus::Queued
    };

    let resp = TriggerUpdateResponse {
        update_history_id: update_history_id.to_string(),
        status,
    };

    (StatusCode::OK, Json(resp)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_list_response_formats_timestamps() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Node.js".to_string(),
            provider_config_id: uuid::Uuid::now_v7(),
            package_identifier: String::new(),
            config_override: None,
            enabled: true,
            last_checked_at: Some(now),
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let config = provider_config::Model {
            id: item.provider_config_id,
            tenant_id: uuid::Uuid::nil(),
            name: "My GitHub Config".to_string(),
            provider_type: "github_releases".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(item, &config, 3);

        assert_eq!(resp.name, "Node.js");
        assert_eq!(resp.provider_config_name, "My GitHub Config");
        assert_eq!(resp.provider_type, "github_releases");
        assert_eq!(resp.host_count, 3);
        assert!(resp.last_checked_at.is_some());
        assert!(resp.config_override.is_none());
    }

    #[test]
    fn build_detail_response_includes_hosts() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Redis".to_string(),
            provider_config_id: uuid::Uuid::now_v7(),
            package_identifier: "redis-server".to_string(),
            config_override: Some(serde_json::json!({"asset_patterns": ["redis.*linux"]})),
            enabled: true,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let config = provider_config::Model {
            id: item.provider_config_id,
            tenant_id: uuid::Uuid::nil(),
            name: "Redis GitHub".to_string(),
            provider_type: "github_releases".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let hosts = vec![SoftwareItemHostSummary {
            host_id: uuid::Uuid::now_v7().to_string(),
            hostname: "web-01".to_string(),
            friendly_name: "Web Server 1".to_string(),
            installed_version: Some("7.2.4".to_string()),
            installed_version_detected_at: Some(format_rfc3339(now)),
            last_updated_at: None,
            linked_at: format_rfc3339(now),
        }];

        let resp = build_detail_response(item, &config, 1, hosts);

        assert_eq!(resp.name, "Redis");
        assert_eq!(resp.package_identifier, "redis-server");
        assert!(resp.config_override.is_some());
        assert_eq!(resp.hosts.len(), 1);
        assert_eq!(resp.hosts[0].hostname, "web-01");
        assert_eq!(resp.hosts[0].installed_version, Some("7.2.4".to_string()));
    }

    #[test]
    fn build_list_response_null_last_checked_at() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            provider_config_id: uuid::Uuid::now_v7(),
            package_identifier: String::new(),
            config_override: None,
            enabled: false,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let config = provider_config::Model {
            id: item.provider_config_id,
            tenant_id: uuid::Uuid::nil(),
            name: "Config".to_string(),
            provider_type: "github_releases".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(item, &config, 0);

        assert!(!resp.enabled);
        assert!(resp.last_checked_at.is_none());
        assert_eq!(resp.host_count, 0);
    }

    #[test]
    fn validate_config_override_valid_merge() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let override_val = serde_json::json!({
            "tag_strip_prefix": "release-"
        });

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_config_override_invalid_merge() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        // Override that clears a required field
        let override_val = serde_json::json!({
            "owner": ""
        });

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_override_non_object_rejected() {
        let base = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let override_val = serde_json::json!("not an object");

        let result = validate_config_override("github_releases", &base, &override_val);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigOverrideError::NotAnObject
        ));
    }
}
