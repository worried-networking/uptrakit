//! HTTP route handlers for autodiscovery ignore-list management.
//!
//! Endpoints:
//! - `GET  /api/v1/autodiscovery/ignores`    — list rules
//! - `POST /api/v1/autodiscovery/ignores`    — create rule
//! - `DELETE /api/v1/autodiscovery/ignores/{id}` — remove rule

use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub use uptrakit_web_api_types::autodiscovery::{
    AutodiscoveryIgnoreResponse, CreateAutodiscoveryIgnoreRequest,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

/// List autodiscovery ignore rules.
///
/// Optionally filter by `plugin_config_id`.
#[utoipa::path(
    get,
    path = "/api/v1/autodiscovery/ignores",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("plugin_config_id" = Option<String>, Query, description = "Filter by plugin config UUID")
    ),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of ignore rules", body = PaginatedResponse<AutodiscoveryIgnoreResponse>),
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn list_autodiscovery_ignores(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<ListIgnoresParams>,
) -> Response {
    let plugin_config_id = params
        .plugin_config_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    };

    match autodiscovery_queries::list_ignore_rules(
        tenant_db.db(),
        tenant_db.tenant_id,
        plugin_config_id,
        &pagination,
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list autodiscovery ignore rules: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Create an autodiscovery ignore rule.
///
/// Idempotent — if the rule already exists, returns the existing rule.
#[utoipa::path(
    post,
    path = "/api/v1/autodiscovery/ignores",
    request_body = CreateAutodiscoveryIgnoreRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 201, description = "Ignore rule created", body = AutodiscoveryIgnoreResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn create_autodiscovery_ignore(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Json(req): Json<CreateAutodiscoveryIgnoreRequest>,
) -> Response {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use uptrakit_shared_db::entity::{plugin_config, prelude::*};

    if req.package_identifier.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "package_identifier must not be empty",
        );
    }

    // Verify plugin config belongs to this tenant.
    let cfg = match PluginConfig::find_by_id(req.plugin_config_id)
        .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Plugin config not found");
        }
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Create the rule (idempotent).
    if let Err(e) = autodiscovery_queries::create_or_ignore_ignore_rule(
        tenant_db.db(),
        tenant_db.tenant_id,
        req.plugin_config_id,
        &req.package_identifier,
    )
    .await
    {
        tracing::error!("Failed to create autodiscovery ignore rule: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Return the current rule (may have been pre-existing).
    let resp = AutodiscoveryIgnoreResponse {
        id: uuid::Uuid::nil(), // will be replaced below
        plugin_config_id: cfg.id,
        plugin_config_name: cfg.name.clone(),
        plugin_type: cfg.plugin_type.clone(),
        package_identifier: req.package_identifier.clone(),
        created_at: time::OffsetDateTime::now_utc(),
    };

    // Fetch actual row to get the correct ID and created_at.
    use uptrakit_shared_db::entity::autodiscovery_ignore;
    let rule = match AutodiscoveryIgnore::find()
        .filter(autodiscovery_ignore::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(autodiscovery_ignore::Column::PluginConfigId.eq(req.plugin_config_id))
        .filter(autodiscovery_ignore::Column::PackageIdentifier.eq(req.package_identifier.clone()))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(r)) => r,
        _ => {
            return (StatusCode::CREATED, Json(resp)).into_response();
        }
    };

    (
        StatusCode::CREATED,
        Json(AutodiscoveryIgnoreResponse {
            id: rule.id,
            plugin_config_id: cfg.id,
            plugin_config_name: cfg.name,
            plugin_type: cfg.plugin_type,
            package_identifier: rule.package_identifier,
            created_at: rule.created_at,
        }),
    )
        .into_response()
}

/// Delete an autodiscovery ignore rule.
#[utoipa::path(
    delete,
    path = "/api/v1/autodiscovery/ignores/{id}",
    params(("id" = String, Path, description = "Ignore rule UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Ignore rule deleted"),
        (status = 404, description = "Ignore rule not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn delete_autodiscovery_ignore(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
) -> Response {
    let rule_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    match autodiscovery_queries::delete_ignore_rule(tenant_db.db(), tenant_db.tenant_id, rule_id)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Ignore rule not found"),
        Err(e) => {
            tracing::error!("Failed to delete autodiscovery ignore rule: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[derive(serde::Deserialize, Default)]
pub struct ListIgnoresParams {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub plugin_config_id: Option<String>,
}
