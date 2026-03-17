use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanApproveSystemServices, CanRejectSystemServices, CanRemoveSystemServices,
    CanUpdateSystemServices, CanViewSystemServices,
};
use crate::middleware::require_auth::AuthenticatedUser;
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
use uptrakit_web_api_types::events::AdminEvent;
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
) -> Response {
    let resp = match ss_queries::approve_system_service(state.db(), service_id).await {
        Ok(r) => r,
        Err(report) => {
            return match report.current_context() {
                SystemServiceQueryError::NotFound => {
                    error_response(StatusCode::NOT_FOUND, "System service not found")
                }
                SystemServiceQueryError::NotPending => error_response(
                    StatusCode::BAD_REQUEST,
                    "System service is not in pending status",
                ),
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

    state
        .broadcast
        .event_broadcaster
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "approved".to_string(),
        })
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
    extensions(("x-required-permission" = json!("reject_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn reject_system_service(
    State(state): State<Arc<AppState>>,
    CanRejectSystemServices(_user): CanRejectSystemServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    let resp = match ss_queries::reject_system_service(state.db(), service_id).await {
        Ok(r) => r,
        Err(report) => {
            return match report.current_context() {
                SystemServiceQueryError::NotFound => {
                    error_response(StatusCode::NOT_FOUND, "System service not found")
                }
                SystemServiceQueryError::NotPending => error_response(
                    StatusCode::BAD_REQUEST,
                    "System service is not in pending status",
                ),
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
    state
        .service_connections
        .force_disconnect(&service_id)
        .await;

    state
        .broadcast
        .event_broadcaster
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "rejected".to_string(),
        })
        .await;

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
    extensions(("x-required-permission" = json!("remove_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn deactivate_system_service(
    State(state): State<Arc<AppState>>,
    CanRemoveSystemServices(_user): CanRemoveSystemServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match ss_queries::deactivate_system_service(state.db(), service_id).await {
        Ok(true) => {
            state.cert.revocation_notify.notify_one();
            state
                .notification_service
                .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                    RequestCrlRenewalPayload::default(),
                ))
                .await;
            state
                .service_connections
                .force_disconnect(&service_id)
                .await;
            state
                .broadcast
                .event_broadcaster
                .send_global(AdminEvent::SystemServiceStatusChanged {
                    id: service_id,
                    status: "deactivated".to_string(),
                })
                .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "System service not found"),
        Err(report) => match report.current_context() {
            SystemServiceQueryError::EmbeddedService => error_response(
                StatusCode::CONFLICT,
                "Embedded services cannot be deactivated",
            ),
            _ => {
                tracing::error!("Failed to deactivate system service: {}", report);
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        },
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

    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match ss_queries::batch_approve_system_services(state.db(), &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "reject" => match ss_queries::batch_reject_system_services(state.db(), &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch reject failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "deactivate" => {
            match ss_queries::batch_deactivate_system_services(state.db(), &body.ids).await {
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

    // Dispatch side effects per succeeded item.
    for id in &succeeded_ids {
        match body.action.as_str() {
            "approve" => {
                let _ = state
                    .notification_service
                    .send(
                        id,
                        ControllerMessage::Approved(ApprovedPayload { service_id: *id }),
                    )
                    .await;
                state
                    .broadcast
                    .event_broadcaster
                    .send_global(AdminEvent::SystemServiceStatusChanged {
                        id: *id,
                        status: "approved".to_string(),
                    })
                    .await;
            }
            "reject" => {
                let _ = state
                    .notification_service
                    .send(
                        id,
                        ControllerMessage::Rejected(RejectedPayload { service_id: *id }),
                    )
                    .await;
                state.service_connections.force_disconnect(id).await;
                state
                    .broadcast
                    .event_broadcaster
                    .send_global(AdminEvent::SystemServiceStatusChanged {
                        id: *id,
                        status: "rejected".to_string(),
                    })
                    .await;
            }
            "deactivate" => {
                state.cert.revocation_notify.notify_one();
                state
                    .notification_service
                    .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                        RequestCrlRenewalPayload::default(),
                    ))
                    .await;
                state.service_connections.force_disconnect(id).await;
                state
                    .broadcast
                    .event_broadcaster
                    .send_global(AdminEvent::SystemServiceStatusChanged {
                        id: *id,
                        status: "deactivated".to_string(),
                    })
                    .await;
            }
            _ => {}
        }
    }

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
