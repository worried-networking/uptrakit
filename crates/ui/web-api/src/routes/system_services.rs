use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSystemServices, CanViewSystemServices};
use crate::queries::system_services::{self as ss_queries, SystemServiceQueryError};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_internal_wire::{
    ApprovedPayload, ControllerMessage, RejectedPayload, RequestCrlRenewalPayload,
};
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

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
        ("capability" = Option<String>, Query, description = "Filter by capability (mqtt_bridge, scheduler)"),
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
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
pub async fn update_system_service(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
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
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
pub async fn approve_system_service(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    let resp = match ss_queries::approve_system_service(state.db(), service_id).await {
        Ok(r) => r,
        Err(report) => {
            return match report.current_context() {
                SystemServiceQueryError::NotFound => {
                    error_response(StatusCode::NOT_FOUND, "System service not found")
                }
                SystemServiceQueryError::NotPending => {
                    error_response(StatusCode::BAD_REQUEST, "System service is not in pending status")
                }
                SystemServiceQueryError::Db(_) => {
                    tracing::error!("Failed to approve system service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
                _ => {
                    tracing::error!("Unexpected error approving system service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            };
        }
    };

    // Push approval via WebSocket (local + cross-controller outbox).
    let _ = state
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    (StatusCode::OK, Json(resp)).into_response()
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
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
pub async fn reject_system_service(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    let resp = match ss_queries::reject_system_service(state.db(), service_id).await {
        Ok(r) => r,
        Err(report) => {
            return match report.current_context() {
                SystemServiceQueryError::NotFound => {
                    error_response(StatusCode::NOT_FOUND, "System service not found")
                }
                SystemServiceQueryError::NotPending => {
                    error_response(StatusCode::BAD_REQUEST, "System service is not in pending status")
                }
                SystemServiceQueryError::Db(_) => {
                    tracing::error!("Failed to reject system service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
                _ => {
                    tracing::error!("Unexpected error rejecting system service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            };
        }
    };

    // Push rejection via WebSocket (local + cross-controller outbox).
    let _ = state
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    // Terminate active WebSocket connection.
    state.service_connections.unregister(&service_id).await;

    (StatusCode::OK, Json(resp)).into_response()
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
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
pub async fn deactivate_system_service(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match ss_queries::deactivate_system_service(state.db(), service_id).await {
        Ok(true) => {
            state.revocation_notify.notify_one();
            state
                .notification_service
                .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                    RequestCrlRenewalPayload::default(),
                ))
                .await;
            state.service_connections.unregister(&service_id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "System service not found"),
        Err(report) => {
            tracing::error!("Failed to deactivate system service: {}", report);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
