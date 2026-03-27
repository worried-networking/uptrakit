use crate::AppState;
use crate::actions::host_tags as tag_actions;
use crate::error_response::error_response;
use crate::middleware::permission::{CanUpdateHosts, CanViewHosts};
use crate::queries::host_tags as tag_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::host_tags::{
    CreateHostTagRequest, HostTagResponse, HostTagSummary, ListHostTagsQuery, SetHostTagsRequest,
    UpdateHostTagRequest,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;

// --- Endpoints ---

/// List all active host tags
#[utoipa::path(
    get,
    path = "/api/v1/host-tags",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("search" = Option<String>, Query, description = "Filter by name (contains)")
    ),
    responses(
        (status = 200, description = "Paginated list of host tags", body = PaginatedResponse<HostTagResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_host_tags(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Query(params): Query<ListHostTagsQuery>,
) -> Response {
    match tag_queries::list_host_tags(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list host tags: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single host tag by ID
#[utoipa::path(
    get,
    path = "/api/v1/host-tags/{id}",
    params(
        ("id" = Uuid, Path, description = "Host tag UUID")
    ),
    responses(
        (status = 200, description = "Host tag details", body = HostTagResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host tag not found")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_host_tag(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Path(tag_id): Path<Uuid>,
) -> Response {
    match tag_queries::get_host_tag(&tenant_db, tag_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Host tag not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Create a new host tag
#[utoipa::path(
    post,
    path = "/api/v1/host-tags",
    request_body = CreateHostTagRequest,
    responses(
        (status = 201, description = "Host tag created", body = HostTagResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 409, description = "Tag with this name already exists")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_host_tag(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(_user): CanUpdateHosts,
    Json(body): Json<CreateHostTagRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let ctx = state.mutation_context();
    match tag_actions::create(&tenant_db, &ctx, &body).await {
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
                error_response(StatusCode::CONFLICT, "A tag with this name already exists")
            } else {
                tracing::error!("Failed to create host tag: {e}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

/// Update an existing host tag
#[utoipa::path(
    put,
    path = "/api/v1/host-tags/{id}",
    params(
        ("id" = Uuid, Path, description = "Host tag UUID")
    ),
    request_body = UpdateHostTagRequest,
    responses(
        (status = 200, description = "Host tag updated", body = HostTagResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host tag not found"),
        (status = 409, description = "Tag with this name already exists")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_host_tag(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(_user): CanUpdateHosts,
    Path(tag_id): Path<Uuid>,
    Json(body): Json<UpdateHostTagRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let ctx = state.mutation_context();
    match tag_actions::update(&tenant_db, &ctx, tag_id, &body).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Host tag not found"),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
                error_response(StatusCode::CONFLICT, "A tag with this name already exists")
            } else {
                tracing::error!("Failed to update host tag: {e}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

/// Delete a host tag (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/host-tags/{id}",
    params(
        ("id" = Uuid, Path, description = "Host tag UUID")
    ),
    responses(
        (status = 204, description = "Host tag deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host tag not found")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_host_tag(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(_user): CanUpdateHosts,
    Path(tag_id): Path<Uuid>,
) -> Response {
    let ctx = state.mutation_context();
    match tag_actions::delete(&tenant_db, &ctx, tag_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Host tag not found"),
        Err(e) => {
            tracing::error!("Failed to delete host tag: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Perform a batch action on multiple host tags.
///
/// Supported actions: `delete`.
#[utoipa::path(
    post,
    path = "/api/v1/host-tags/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_host_tags(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(_user): CanUpdateHosts,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let ctx = state.mutation_context();
    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => match tag_actions::batch_delete(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete host tags failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
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

/// Set (replace-all) tags on a host
#[utoipa::path(
    put,
    path = "/api/v1/hosts/{id}/tags",
    params(
        ("id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = SetHostTagsRequest,
    responses(
        (status = 200, description = "Tags assigned", body = Vec<HostTagSummary>),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn set_host_tags(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(_user): CanUpdateHosts,
    Path(host_id): Path<Uuid>,
    Json(body): Json<SetHostTagsRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Verify host exists and belongs to tenant.
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use uptrakit_shared_db::entity::host;

    let host_exists = match host::Entity::find_by_id(host_id)
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

    if !host_exists {
        return error_response(StatusCode::NOT_FOUND, "Host not found");
    }

    let ctx = state.mutation_context();
    // Refresh MQTT `{prefix}/hosts/{h}/tags` for all connected MQTT services.
    match tag_actions::set(&tenant_db, &ctx, host_id, &body.tag_ids).await {
        Ok(tags) => (StatusCode::OK, Json(tags)).into_response(),
        Err(e) => {
            tracing::error!("Failed to set host tags: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
