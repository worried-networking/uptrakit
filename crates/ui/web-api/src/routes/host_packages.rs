//! HTTP route handlers for host package management.
//!
//! Endpoints:
//! - `GET    /api/v1/hosts/{host_id}/packages`             — list packages
//! - `GET    /api/v1/hosts/{host_id}/packages/{id}`         — package detail
//! - `PUT    /api/v1/hosts/{host_id}/packages/{id}`         — update (enable/disable)
//! - `DELETE /api/v1/hosts/{host_id}/packages/{id}`         — soft-delete
//! - `GET    /api/v1/hosts/{host_id}/package-ignores`       — list ignore rules
//! - `POST   /api/v1/hosts/{host_id}/package-ignores`       — create ignore rule
//! - `DELETE /api/v1/hosts/{host_id}/package-ignores/{id}`  — remove ignore rule

use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::host_packages as hp_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uptrakit_shared_db::entity::{host, prelude::*};
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::host_packages::{
    CreateHostPackageIgnoreRequest, HostPackageDetailResponse, HostPackageIgnoreResponse,
    HostPackageResponse, HostUpdateSummary, ListHostPackagesParams, PromoteHostPackageRequest,
    UpdateHostPackageRequest,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::software_items::SoftwareItemDetailResponse;

// ── Helper: verify host belongs to tenant ───────────────────────────────────

async fn verify_host(tenant_db: &TenantDb, host_id: Uuid) -> Result<(), Response> {
    match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(error_response(StatusCode::NOT_FOUND, "Host not found")),
        Err(e) => {
            tracing::error!("DB error verifying host: {e}");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ))
        }
    }
}

// ── Host packages ───────────────────────────────────────────────────────────

/// List host packages for a specific host.
///
/// Returns a paginated list of packages tracked on this host, with optional
/// filters for enabled status, update availability, category, and search text.
#[utoipa::path(
    get,
    path = "/api/v1/hosts/{host_id}/packages",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("enabled" = Option<bool>, Query, description = "Filter by enabled status"),
        ("has_update" = Option<bool>, Query, description = "Filter to packages with available updates"),
        ("category" = Option<String>, Query, description = "Filter by update category (security, bugfix, feature, unknown)"),
        ("search" = Option<String>, Query, description = "Search by package name or identifier")
    ),
    responses(
        (status = 200, description = "Paginated list of host packages", body = PaginatedResponse<HostPackageResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn list_host_packages(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(host_id): Path<Uuid>,
    Query(params): Query<ListHostPackagesParams>,
) -> Response {
    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    match hp_queries::list_host_packages(&tenant_db, host_id, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list host packages: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single host package by ID with detail including plugin config info
/// and recent update history.
#[utoipa::path(
    get,
    path = "/api/v1/hosts/{host_id}/packages/{id}",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("id" = Uuid, Path, description = "Host package UUID")
    ),
    responses(
        (status = 200, description = "Host package detail", body = HostPackageDetailResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host or package not found")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn get_host_package(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path((host_id, package_id)): Path<(Uuid, Uuid)>,
) -> Response {
    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    let pkg = match hp_queries::get_host_package(&tenant_db, host_id, package_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host package not found"),
        Err(e) => {
            tracing::error!("Failed to get host package: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Load plugin config info for the detail view.
    let (plugin_config_name, plugin_type) =
        match hp_queries::get_host_package_plugin_config(&tenant_db, pkg.plugin_config_id).await {
            Ok(Some(cfg)) => (cfg.name, cfg.plugin_type),
            Ok(None) => ("Unknown".to_string(), "unknown".to_string()),
            Err(e) => {
                tracing::error!("Failed to load plugin config: {e}");
                ("Unknown".to_string(), "unknown".to_string())
            }
        };

    // Load recent update history.
    let recent_updates =
        match hp_queries::get_host_package_update_history(&tenant_db, package_id, 10).await {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Failed to load update history: {e}");
                vec![]
            }
        };

    let detail = HostPackageDetailResponse {
        package: pkg,
        plugin_config_name,
        plugin_type,
        recent_updates,
    };

    (StatusCode::OK, Json(detail)).into_response()
}

/// Update a host package (enable or disable).
#[utoipa::path(
    put,
    path = "/api/v1/hosts/{host_id}/packages/{id}",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("id" = Uuid, Path, description = "Host package UUID")
    ),
    request_body = UpdateHostPackageRequest,
    responses(
        (status = 200, description = "Host package updated", body = HostPackageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host or package not found")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("manage_software"))),
    security(("bearer_token" = []))
)]
pub async fn update_host_package(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((host_id, package_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateHostPackageRequest>,
) -> Response {
    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    match hp_queries::update_host_package(&tenant_db, host_id, package_id, body.enabled).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Host package not found"),
        Err(e) => {
            tracing::error!("Failed to update host package: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Soft-delete a host package.
///
/// Optionally pass `?ignore=true` to also create an ignore rule preventing
/// this package from being re-discovered.
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{host_id}/packages/{id}",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("id" = Uuid, Path, description = "Host package UUID"),
        ("ignore" = Option<bool>, Query, description = "If true, also create an ignore rule to prevent re-discovery")
    ),
    responses(
        (status = 204, description = "Host package deactivated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host or package not found")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("manage_software"))),
    security(("bearer_token" = []))
)]
pub async fn delete_host_package(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((host_id, package_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<DeleteHostPackageParams>,
) -> Response {
    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    let create_ignore = params.ignore.unwrap_or(false);

    match hp_queries::deactivate_host_package(&tenant_db, host_id, package_id, create_ignore).await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Host package not found"),
        Err(e) => {
            tracing::error!("Failed to deactivate host package: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ── Host package ignore rules ───────────────────────────────────────────────

/// List host package ignore rules for a specific host.
#[utoipa::path(
    get,
    path = "/api/v1/hosts/{host_id}/package-ignores",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    responses(
        (status = 200, description = "List of ignore rules", body = Vec<HostPackageIgnoreResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("view_software"))),
    security(("bearer_token" = []))
)]
pub async fn list_host_package_ignores(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(host_id): Path<Uuid>,
) -> Response {
    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    match hp_queries::list_host_package_ignores(&tenant_db, host_id).await {
        Ok(rules) => (StatusCode::OK, Json(rules)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list host package ignores: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Create a host package ignore rule.
///
/// Idempotent — if the rule already exists, returns 200 OK.
#[utoipa::path(
    post,
    path = "/api/v1/hosts/{host_id}/package-ignores",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = CreateHostPackageIgnoreRequest,
    responses(
        (status = 201, description = "Ignore rule created"),
        (status = 200, description = "Ignore rule already exists"),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host or plugin config not found")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("manage_software"))),
    security(("bearer_token" = []))
)]
pub async fn create_host_package_ignore(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(host_id): Path<Uuid>,
    Json(req): Json<CreateHostPackageIgnoreRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    // Verify plugin config belongs to this tenant.
    use uptrakit_shared_db::entity::plugin_config;
    match PluginConfig::find_by_id(req.plugin_config_id)
        .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Plugin config not found");
        }
        Err(e) => {
            tracing::error!("DB error checking plugin config: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    match hp_queries::create_host_package_ignore(
        &tenant_db,
        host_id,
        req.plugin_config_id,
        &req.package_identifier,
    )
    .await
    {
        Ok(true) => StatusCode::CREATED.into_response(),
        Ok(false) => StatusCode::OK.into_response(),
        Err(e) => {
            tracing::error!("Failed to create host package ignore: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Delete a host package ignore rule.
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{host_id}/package-ignores/{id}",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("id" = Uuid, Path, description = "Ignore rule UUID")
    ),
    responses(
        (status = 204, description = "Ignore rule deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host or ignore rule not found")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("manage_software"))),
    security(("bearer_token" = []))
)]
pub async fn delete_host_package_ignore(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((host_id, ignore_id)): Path<(Uuid, Uuid)>,
) -> Response {
    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    match hp_queries::delete_host_package_ignore(&tenant_db, host_id, ignore_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Ignore rule not found"),
        Err(e) => {
            tracing::error!("Failed to delete host package ignore: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ── Promote host package ─────────────────────────────────────────────────────

/// Promote a host package to a tracked software item.
///
/// Creates a software item alongside the host package (additive — the package is
/// kept unchanged). If the host is already assigned to a matching software item,
/// the existing item is returned (idempotent). Returns the full software item detail
/// including host assignments and pre-populated version data.
#[utoipa::path(
    post,
    path = "/api/v1/hosts/{host_id}/packages/{id}/promote",
    params(
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("id" = Uuid, Path, description = "Host package UUID")
    ),
    request_body = PromoteHostPackageRequest,
    responses(
        (status = 200, description = "Software item created or returned (idempotent)", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host, package, or referenced software item not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Host Packages",
    extensions(("x-required-permission" = json!("manage_software"))),
    security(("bearer_token" = []))
)]
pub async fn promote_host_package(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((host_id, package_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PromoteHostPackageRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    if let Err(resp) = verify_host(&tenant_db, host_id).await {
        return resp;
    }

    match hp_queries::promote_host_package(&tenant_db, host_id, package_id, req).await {
        Ok(detail) => (StatusCode::OK, Json(detail)).into_response(),
        Err(ref e)
            if matches!(
                e.current_context(),
                hp_queries::HostPackageError::PackageNotFound
                    | hp_queries::HostPackageError::SoftwareItemNotFound(_)
            ) =>
        {
            error_response(StatusCode::NOT_FOUND, e.current_context().to_string())
        }
        Err(e) => {
            tracing::error!("Failed to promote host package: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ── Query parameter structs ─────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
pub struct DeleteHostPackageParams {
    pub ignore: Option<bool>,
}
