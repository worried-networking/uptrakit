use crate::AppState;
use crate::actions::system_services as ss_actions;
use crate::api_error::ApiError;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanApproveSystemServices, CanRejectSystemServices, CanRemoveSystemServices,
    CanUpdateSystemServices, CanViewSystemServices,
};
use crate::middleware::require_auth::AuthenticatedUser;
use crate::queries::system_services as ss_queries;
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
pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::system_services::{
    ListSystemServicesQuery, SystemServiceResponse, UpdateSystemServiceRequest,
};

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all system services
#[utoipa::path(
    get,
    path = "/api/v1/system-services",
    params(
        ("capability" = Option<String>, Query, description = "Filter by capability (update_tracking, scheduler)"),
        ("status" = Option<String>, Query, description = "Filter by status (pending, approved, rejected, deactivated)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of system services", body = PaginatedResponse<SystemServiceResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("view_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_system_services(
    State(state): State<Arc<AppState>>,
    CanViewSystemServices(_user): CanViewSystemServices,
    Query(query): Query<ListSystemServicesQuery>,
) -> Response {
    match ss_queries::list_system_services(state.db(), &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list system services: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single system service by ID
#[utoipa::path(
    get,
    path = "/api/v1/system-services/{id}",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 200, description = "System service details", body = SystemServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("view_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_system_service(
    State(state): State<Arc<AppState>>,
    CanViewSystemServices(_user): CanViewSystemServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match ss_queries::get_active_system_service(state.db(), service_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "System service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a system service's configurable settings (e.g. ping interval)
#[utoipa::path(
    put,
    path = "/api/v1/system-services/{id}",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    request_body = UpdateSystemServiceRequest,
    responses(
        (status = 200, description = "System service updated", body = SystemServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("update_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_system_service(
    State(state): State<Arc<AppState>>,
    CanUpdateSystemServices(_user): CanUpdateSystemServices,
    Path(service_id): Path<Uuid>,
    Json(body): Json<UpdateSystemServiceRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match ss_queries::update_system_service_settings(
        state.db(),
        service_id,
        body.ping_interval_seconds,
        body.cert_lifetime_hours,
    )
    .await
    {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "System service not found"),
        Err(e) => {
            tracing::error!("Failed to update system service: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Approve a pending system service
#[utoipa::path(
    post,
    path = "/api/v1/system-services/{id}/approve",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 200, description = "System service approved", body = SystemServiceResponse),
        (status = 400, description = "Service is not in pending status"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("approve_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn approve_system_service(
    State(state): State<Arc<AppState>>,
    CanApproveSystemServices(_user): CanApproveSystemServices,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = state.mutation_context();
    let resp = ss_actions::approve(state.db(), &ctx, service_id).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Reject a pending system service
#[utoipa::path(
    post,
    path = "/api/v1/system-services/{id}/reject",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 200, description = "System service rejected", body = SystemServiceResponse),
        (status = 400, description = "Service is not in pending status"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("reject_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn reject_system_service(
    State(state): State<Arc<AppState>>,
    CanRejectSystemServices(_user): CanRejectSystemServices,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = state.mutation_context();
    let resp = ss_actions::reject(state.db(), &ctx, service_id, &state.service_connections).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Deactivate a system service (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/system-services/{id}",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 204, description = "System service deactivated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("remove_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn deactivate_system_service(
    State(state): State<Arc<AppState>>,
    CanRemoveSystemServices(_user): CanRemoveSystemServices,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = state.mutation_context();
    let found = ss_actions::deactivate(
        state.db(),
        &ctx,
        service_id,
        &state.cert,
        &state.service_connections,
    )
    .await?;
    if found {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(error_response(
            StatusCode::NOT_FOUND,
            "System service not found",
        ))
    }
}

/// Perform a batch action on multiple system services.
///
/// Supported actions: `approve`, `reject`, `deactivate`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/system-services/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("approve_system_services, reject_system_services, or remove_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_system_services(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let required = match body.action.as_str() {
        "approve" => Permission::ApproveSystemServices,
        "reject" => Permission::RejectSystemServices,
        "deactivate" => Permission::RemoveSystemServices,
        _ => return error_response(StatusCode::BAD_REQUEST, "Unknown batch action"),
    };
    if !auth_user.has_permission(required) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let ctx = state.mutation_context();
    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match ss_actions::batch_approve(state.db(), &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "reject" => {
            match ss_actions::batch_reject(state.db(), &ctx, &body.ids, &state.service_connections)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("batch reject failed: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        "deactivate" => {
            match ss_actions::batch_deactivate(
                state.db(),
                &ctx,
                &body.ids,
                &state.cert,
                &state.service_connections,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("batch deactivate failed: {e}");
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
                format!("unknown action: {unknown}. Supported: approve, reject, deactivate"),
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
