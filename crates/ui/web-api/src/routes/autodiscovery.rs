//! HTTP route handlers for autodiscovery ignore-list management.
//!
//! Endpoints:
//! - `GET  /api/v1/autodiscovery/ignores`    — list rules
//! - `POST /api/v1/autodiscovery/ignores`    — create rule
//! - `DELETE /api/v1/autodiscovery/ignores/{id}` — remove rule

use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanManageIgnores, CanViewSoftware};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

pub use uptrakit_web_api_types::autodiscovery::{
    CreateSoftwareIgnoreRequest, SoftwareIgnoreResponse,
};
pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

/// List autodiscovery ignore rules.
#[utoipa::path(
    get,
    path = "/api/v1/autodiscovery/ignores",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
    ),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of ignore rules", body = PaginatedResponse<SoftwareIgnoreResponse>),
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_autodiscovery_ignores(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<ListIgnoresParams>,
) -> Response {
    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    };

    match autodiscovery_queries::list_ignore_rules(tenant_db.db(), tenant_db.tenant_id, &pagination)
        .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list autodiscovery ignore rules");
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
    request_body = CreateSoftwareIgnoreRequest,
    extensions(("x-required-permission" = json!("manage_ignores"))),
    responses(
        (status = 201, description = "Ignore rule created", body = SoftwareIgnoreResponse),
        (status = 200, description = "Ignore rule already exists", body = SoftwareIgnoreResponse),
        (status = 400, description = "Invalid input"),
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_autodiscovery_ignore(
    tenant_db: TenantDb,
    CanManageIgnores(_user): CanManageIgnores,
    Validated(req): Validated<CreateSoftwareIgnoreRequest>,
) -> Response {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use uptrakit_shared_db::entity::{prelude::*, software_ignore};

    let name = req.name.trim().to_string();

    // Create the rule (idempotent). Returns true if newly inserted, false if already existed.
    let was_created = match autodiscovery_queries::create_or_ignore_ignore_rule(
        tenant_db.db(),
        tenant_db.tenant_id,
        &name,
        req.host_id,
    )
    .await
    {
        Ok(created) => created,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create autodiscovery ignore rule");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Fetch the current rule to get the correct ID and created_at.
    let mut query = SoftwareIgnore::find()
        .filter(software_ignore::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_ignore::Column::Name.eq(&name));
    if let Some(host_id) = req.host_id {
        query = query.filter(software_ignore::Column::HostId.eq(host_id));
    } else {
        query = query.filter(software_ignore::Column::HostId.is_null());
    }
    let rule = match query.one(tenant_db.db()).await {
        Ok(Some(r)) => r,
        Ok(None) | Err(_) => {
            return (
                StatusCode::CREATED,
                Json(SoftwareIgnoreResponse {
                    id: uuid::Uuid::nil(),
                    name,
                    host_id: req.host_id,
                    created_at: time::OffsetDateTime::now_utc(),
                }),
            )
                .into_response();
        }
    };

    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    (
        status,
        Json(SoftwareIgnoreResponse {
            id: rule.id,
            name: rule.name,
            host_id: rule.host_id,
            created_at: rule.created_at,
        }),
    )
        .into_response()
}

/// Delete an autodiscovery ignore rule.
#[utoipa::path(
    delete,
    path = "/api/v1/autodiscovery/ignores/{id}",
    params(("id" = Uuid, Path, description = "Ignore rule UUID")),
    extensions(("x-required-permission" = json!("manage_ignores"))),
    responses(
        (status = 204, description = "Ignore rule deleted"),
        (status = 404, description = "Ignore rule not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_autodiscovery_ignore(
    tenant_db: TenantDb,
    CanManageIgnores(_user): CanManageIgnores,
    Path(rule_id): Path<Uuid>,
) -> Response {
    match autodiscovery_queries::delete_ignore_rule(tenant_db.db(), tenant_db.tenant_id, rule_id)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Ignore rule not found"),
        Err(e) => {
            tracing::error!(error = %e, "Failed to delete autodiscovery ignore rule");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Perform a batch action on multiple autodiscovery ignore rules.
///
/// Supported actions: `delete`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/autodiscovery/ignores/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Autodiscovery",
    extensions(("x-required-permission" = json!("manage_ignores"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_autodiscovery_ignores(
    tenant_db: TenantDb,
    CanManageIgnores(_user): CanManageIgnores,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => {
            match autodiscovery_queries::batch_delete_ignore_rules(
                tenant_db.db(),
                tenant_db.tenant_id,
                &body.ids,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "batch delete failed");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        unknown => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: delete"),
            );
        }
    };

    let response = BatchActionResponse {
        succeeded: succeeded_ids
            .into_iter()
            .map(|id| BatchActionSuccess { id })
            .collect(),
        failed: failed
            .into_iter()
            .map(|(id, error)| BatchActionFailure { id, error })
            .collect(),
    };

    (StatusCode::OK, Json(response)).into_response()
}

#[derive(serde::Deserialize, Default)]
pub struct ListIgnoresParams {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}
