use crate::AppState;
use crate::actions::services as svc_actions;
use crate::api_error::ApiError;
use crate::app_state::AuditEmitterState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanApproveServices, CanRejectServices, CanRemoveServices, CanUpdateServices, CanViewServices,
};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::services as svc_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_web_api_types::validation::Validate;
use uptrakit_wire::{ControllerMessage, SetUpdateFreezePayload};
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::services::{
    ListServicesQuery, MergeAgentRequest, MessageResponse, ServiceResponse, ServiceStatus,
    SetUpdateFreezeRequest, UpdateServiceRequest,
};

struct AuditContext<'a> {
    audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_service_lifecycle_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    service_id: Uuid,
    service_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .target("service", service_id.to_string(), service_display)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_best_effort(entry);
    }
}

fn emit_service_batch_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_best_effort(entry);
    }
}

fn batch_action_to_audit_action(action: &str) -> Option<uptrakit_audit_log::RegisteredAuditAction> {
    match action {
        "approve" => Some(uptrakit_audit_log::AuditActionType::SERVICE_APPROVE),
        "reject" => Some(uptrakit_audit_log::AuditActionType::SERVICE_REJECT),
        "deactivate" => Some(uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all services (agents and/or MQTT)
#[utoipa::path(
    get,
    path = "/api/v1/services",
    params(
        ("capability" = Option<String>, Query, description = "Filter by capability (software_discovery, update_tracking, ssh_remote, scheduler)"),
        ("status" = Option<String>, Query, description = "Filter by status (pending, approved, rejected, deactivated)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of services", body = PaginatedResponse<ServiceResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_services(
    tenant_db: TenantDb,
    CanViewServices(_user): CanViewServices,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    match svc_queries::list_services(&tenant_db, &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list services: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single service by ID
#[utoipa::path(
    get,
    path = "/api/v1/services/{id}",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service details", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_service(
    tenant_db: TenantDb,
    CanViewServices(_user): CanViewServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a service's configurable settings (e.g. ping interval)
#[utoipa::path(
    put,
    path = "/api/v1/services/{id}",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    request_body = UpdateServiceRequest,
    responses(
        (status = 200, description = "Service updated", body = ServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("update_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_service(
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanUpdateServices(user): CanUpdateServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
    Json(body): Json<UpdateServiceRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    if let Err(e) = body.validate() {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match svc_queries::update_service_settings(
        &tenant_db,
        service_id,
        body.ping_interval_seconds,
        body.cert_lifetime_hours,
    )
    .await
    {
        Ok(Some(resp)) => {
            emit_service_lifecycle_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
                resp.id,
                Some(resp.friendly_name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "ping_interval_seconds": body.ping_interval_seconds,
                    "cert_lifetime_hours": body.cert_lifetime_hours,
                }),
            );
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) => {
            emit_service_lifecycle_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
                service_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "service.not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Service not found")
        }
        Err(e) => {
            tracing::error!("Failed to update service: {}", e);
            emit_service_lifecycle_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
                service_id,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "service.database_error",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Approve a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/approve",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service approved", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("approve_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn approve_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanApproveServices(user): CanApproveServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let ctx = state.mutation_context();
    let resp = match svc_actions::approve(&tenant_db, &ctx, service_id).await {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            emit_service_lifecycle_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
                service_id,
                None,
                outcome,
                serde_json::json!({
                    "reason_code": reason_code,
                }),
            );
            return Err(err.into());
        }
    };
    emit_service_lifecycle_audit(
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
        resp.id,
        Some(resp.friendly_name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "status": resp.status,
        }),
    );
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Reject a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/reject",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service rejected", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("reject_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn reject_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanRejectServices(user): CanRejectServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let ctx = state.mutation_context();
    let resp =
        match svc_actions::reject(&tenant_db, &ctx, service_id, &state.service_connections).await {
            Ok(resp) => resp,
            Err(err) => {
                let (outcome, reason_code) = err.current_context().audit_classification();
                emit_service_lifecycle_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
                    service_id,
                    None,
                    outcome,
                    serde_json::json!({
                        "reason_code": reason_code,
                    }),
                );
                return Err(err.into());
            }
        };
    emit_service_lifecycle_audit(
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
        resp.id,
        Some(resp.friendly_name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "status": resp.status,
        }),
    );
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Deactivate a service (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/services/{id}",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 204, description = "Service deactivated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("remove_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn deactivate_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanRemoveServices(user): CanRemoveServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let ctx = state.mutation_context();
    let found = svc_actions::deactivate(
        &tenant_db,
        &ctx,
        service_id,
        state.default_tenant_id,
        &state.cert,
        &state.service_connections,
    )
    .await
    .map_err(|err| {
        let (outcome, reason_code) = err.current_context().audit_classification();
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            service_id,
            None,
            outcome,
            serde_json::json!({
                "reason_code": reason_code,
            }),
        );
        ApiError::from(err)
    })?;
    if found {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::Success,
            serde_json::json!({}),
        );
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "service.not_found",
            }),
        );
        Ok(error_response(StatusCode::NOT_FOUND, "Service not found"))
    }
}

/// Enable or disable the update freeze on a connected service.
///
/// Sends a `SetUpdateFreeze` wire message to the connected agent. The agent
/// creates or removes the `update-freeze` file in its state directory,
/// immediately blocking or unblocking `ExecuteUpdate` and
/// `ExecuteBatchHostPackageUpdate` processing.
///
/// Returns 404 if the service is not found, 409 if the service is not
/// currently connected, and 200 with a confirmation message on success.
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/update-freeze",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    request_body = SetUpdateFreezeRequest,
    responses(
        (status = 200, description = "Freeze state sent to service", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found"),
        (status = 409, description = "Service not connected")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("update_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn set_update_freeze(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateServices(user): CanUpdateServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
    Json(body): Json<SetUpdateFreezeRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let action_type = if body.enabled {
        uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_ENABLE
    } else {
        uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_DISABLE
    };
    if let Err(e) = body.validate() {
        emit_service_lifecycle_audit(
            &audit_ctx,
            action_type,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "enabled": body.enabled,
                "reason_present": body.reason.is_some(),
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Verify the service exists in this tenant.
    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            emit_service_lifecycle_audit(
                &audit_ctx,
                action_type,
                service_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "enabled": body.enabled,
                    "reason_present": body.reason.is_some(),
                    "reason_code": "service.not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Service not found");
        }
        Err(report) => {
            tracing::error!("Failed to look up service: {report}");
            emit_service_lifecycle_audit(
                &audit_ctx,
                action_type,
                service_id,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "enabled": body.enabled,
                    "reason_present": body.reason.is_some(),
                    "reason_code": "service.database_error",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Check that the service is currently connected.
    if !state.service_connections.is_connected(&service_id).await {
        emit_service_lifecycle_audit(
            &audit_ctx,
            action_type,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "enabled": body.enabled,
                "reason_present": body.reason.is_some(),
                "reason_code": "service.not_connected",
            }),
        );
        return error_response(StatusCode::CONFLICT, "Service is not currently connected");
    }

    let msg = ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
        enabled: body.enabled,
        reason: body.reason.clone(),
    });

    let sent = state.service_connections.send(&service_id, msg).await;
    if !sent {
        emit_service_lifecycle_audit(
            &audit_ctx,
            action_type,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "enabled": body.enabled,
                "reason_present": body.reason.is_some(),
                "reason_code": "service.not_connected",
            }),
        );
        return error_response(StatusCode::CONFLICT, "Service is not currently connected");
    }

    let action = if body.enabled { "enabled" } else { "disabled" };
    emit_service_lifecycle_audit(
        &audit_ctx,
        action_type,
        service_id,
        None,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "enabled": body.enabled,
            "reason_present": body.reason.is_some(),
        }),
    );

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: format!("Update freeze {action} for service {service_id}."),
        }),
    )
        .into_response()
}

/// Merge a pending (source) agent into an existing approved (target) agent.
///
/// This operation is only valid for agent services. MQTT services cannot be merged.
#[utoipa::path(
    post,
    path = "/api/v1/services/{target_id}/merge",
    params(
        ("target_id" = Uuid, Path, description = "Target service UUID (approved agent)")
    ),
    request_body = MergeAgentRequest,
    responses(
        (status = 200, description = "Services merged", body = ServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("update_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn merge_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateServices(user): CanUpdateServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(target_uuid): Path<Uuid>,
    Json(body): Json<MergeAgentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let source_uuid = body.source_id;

    if target_uuid == source_uuid {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.self_merge",
            }),
        );
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Cannot merge service into itself",
        ));
    }

    let target_connected = state.service_connections.is_connected(&target_uuid).await;
    let ctx = state.mutation_context();
    let resp = svc_actions::merge(
        &tenant_db,
        &ctx,
        svc_actions::MergeParams {
            target_id: target_uuid,
            source_id: source_uuid,
            target_connected,
            default_tenant_id: state.default_tenant_id,
        },
        &state.cert,
        &state.service_connections,
    )
    .await
    .map_err(|err| {
        let (outcome, reason_code) = err.current_context().audit_classification();
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            outcome,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": reason_code,
            }),
        );
        ApiError::from(err)
    })?;

    tracing::info!(
        target_id = %target_uuid,
        source_id = %source_uuid,
        "services merged: source deactivated, target updated"
    );

    emit_service_lifecycle_audit(
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
        target_uuid,
        Some(resp.friendly_name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "source_service_id": source_uuid,
        }),
    );

    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Perform a batch action on multiple services.
///
/// Supported actions: `approve`, `reject`, `deactivate`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/services/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("approve_services, reject_services, or remove_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_services(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Extension(auth_user): Extension<AuthenticatedUser>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &auth_user,
        api_token_id,
    };
    let action_type = batch_action_to_audit_action(&body.action);

    if let Err(e) = body.validate() {
        if let Some(action_type) = action_type {
            emit_service_batch_audit(
                &audit_ctx,
                action_type,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "reason_code": "invalid_request",
                    "batch": true,
                }),
            );
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let required = match body.action.as_str() {
        "approve" => Permission::ApproveServices,
        "reject" => Permission::RejectServices,
        "deactivate" => Permission::RemoveServices,
        _ => return error_response(StatusCode::BAD_REQUEST, "Unknown batch action"),
    };
    if !auth_user.has_permission(required) {
        if let Some(action_type) = action_type {
            emit_service_batch_audit(
                &audit_ctx,
                action_type,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "insufficient_permissions",
                    "batch": true,
                    "requested_count": body.ids.len(),
                }),
            );
        }
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let ctx = state.mutation_context();
    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match svc_actions::batch_approve(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                emit_service_batch_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "batch_approve_failed",
                        "batch": true,
                        "requested_count": body.ids.len(),
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "reject" => {
            match svc_actions::batch_reject(&tenant_db, &ctx, &body.ids, &state.service_connections)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("batch reject failed: {e}");
                    emit_service_batch_audit(
                        &audit_ctx,
                        uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({
                            "reason_code": "batch_reject_failed",
                            "batch": true,
                            "requested_count": body.ids.len(),
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        "deactivate" => {
            match svc_actions::batch_deactivate(
                &tenant_db,
                &ctx,
                &body.ids,
                state.default_tenant_id,
                &state.cert,
                &state.service_connections,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("batch deactivate failed: {e}");
                    emit_service_batch_audit(
                        &audit_ctx,
                        uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({
                            "reason_code": "batch_deactivate_failed",
                            "batch": true,
                            "requested_count": body.ids.len(),
                        }),
                    );
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

    if let Some(action_type) = action_type {
        let outcome = if succeeded_ids.is_empty() {
            uptrakit_audit_log::AuditOutcome::Failed
        } else if failed.is_empty() {
            uptrakit_audit_log::AuditOutcome::Success
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        };

        emit_service_batch_audit(
            &audit_ctx,
            action_type,
            outcome,
            serde_json::json!({
                "batch": true,
                "requested_count": body.ids.len(),
                "succeeded_count": succeeded_ids.len(),
                "failed_count": failed.len(),
            }),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    use crate::middleware::permission::{
        CanApproveServices, CanRejectServices, CanRemoveServices, CanUpdateServices,
    };
    use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
    use crate::settings::Settings;
    use crate::tenant_db::TenantDb;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::{Extension, Json};
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, QueryOrder,
        Set,
    };
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{audit_log, prelude::Service, service, tenant};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn insert_tenant(db: &DatabaseConnection, id: uuid::Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set("Test Tenant".to_string()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
    }

    async fn test_state(db: DatabaseConnection, tenant_id: uuid::Uuid) -> Arc<AppState> {
        struct NoopCertSigner;
        #[async_trait::async_trait]
        impl AgentCertSigner for NoopCertSigner {
            async fn sign_agent_csr(
                &self,
                _: &str,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>>
            {
                Err(rootcause::report!(CertSignerError::Signing(
                    "noop signer".to_string(),
                )))
            }
            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let notification_service = crate::notification_service::NotificationService::new(
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            )
            .expect("default catalog should build"),
        );

        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState {
                jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-for-service-merge-tests",
                )),
                device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            },
            notification: crate::app_state::NotificationState {
                notification_service,
                notification_dispatcher,
                event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
            },
            broadcast: crate::app_state::BroadcastState {
                device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster::new(
                ),
                update_output_broadcaster:
                    crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
                batch_progress_broadcaster:
                    crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
            },
            #[cfg(feature = "oidc")]
            oidc: crate::app_state::OidcState {
                oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
                account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
                oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                    db.clone(),
                ),
                oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                    db.clone(),
                ),
            },
            default_tenant_id: tenant_id,
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            controller_id: uuid::Uuid::nil(),
            plugin_ops,
            global_providers: Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::DatabaseBackend::new(db.clone()),
            )),
            audit_emitter: uptrakit_audit_log::AuditEmitter::new(
                uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                    uptrakit_audit_log::DatabaseBackend::new(db.clone()),
                )),
            ),
            surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
                Arc::new(crate::surface_registry::SurfaceRegistry::new(
                    crate::surface_registry::SurfaceRegistryConfig::default(),
                )),
                Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            ),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            reject_dangerous_commands: false,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
        })
    }

    async fn latest_tenant_audit_row(db: &DatabaseConnection) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row");
    }

    fn agent_caps_json() -> String {
        use std::collections::BTreeSet;
        use uptrakit_wire::Capability;
        uptrakit_wire::service_profile::serialize_capabilities(&BTreeSet::from([
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
        ]))
    }

    /// Helper: insert a pair of test services (approved target + pending source).
    async fn insert_target_and_source(
        db: &DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) -> (service::Model, service::Model) {
        let now = OffsetDateTime::now_utc();
        let target = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            capabilities: Set(agent_caps_json()),
            hostname: Set("target-host".to_string()),
            friendly_name: Set("Target".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set("target-hash".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        };
        let target = target.insert(db).await.unwrap();

        let source = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            capabilities: Set(agent_caps_json()),
            hostname: Set("source-host".to_string()),
            friendly_name: Set("Source".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Pending),
            enrollment_secret_hash: Set("source-hash".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        };
        let source = source.insert(db).await.unwrap();

        (target, source)
    }

    /// When the target agent is currently connected the merge must be rejected
    /// with 409 CONFLICT and leave the source service completely unmodified.
    #[tokio::test]
    async fn merge_service_target_connected_returns_conflict() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, source) = insert_target_and_source(&db, tenant_id).await;

        // Register the target as connected — merge must be rejected before any DB changes.
        let caps = {
            use std::collections::BTreeSet;
            use uptrakit_wire::Capability;
            BTreeSet::from([
                Capability::GracefulShutdown,
                Capability::SoftwareDiscovery,
                Capability::UpdateHooks,
            ])
        };
        let (_rx, _token) = state
            .service_connections
            .register(target.id, caps, None, None, None)
            .await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = merge_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            None,
            Path(target.id),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        let status = match response {
            Err(e) => e.into_response().status(),
            Ok(_) => panic!("expected Err(ApiError) but got Ok"),
        };
        assert_eq!(status, StatusCode::CONFLICT);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["source_service_id"], serde_json::json!(source.id));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service.target_connected")
        );

        // Source must not have been touched.
        let source_after = Service::find_by_id(source.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(source_after.deactivated_at.is_none());
        assert_eq!(source_after.enrollment_secret_hash, "source-hash");
    }

    /// A merge of a valid pending source into an approved target must succeed:
    /// the source is deactivated (with its hash invalidated) and the target
    /// adopts the source's identity fields.
    #[tokio::test]
    async fn merge_service_succeeds_and_deactivates_source() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = merge_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            None,
            Path(target.id),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        let status = match response {
            Ok(r) => r.into_response().status(),
            Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
        };
        assert_eq!(status, StatusCode::OK);

        // Source must be deactivated and its original hash must be invalidated.
        let source_after = Service::find_by_id(source.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(
            source_after.deactivated_at.is_some(),
            "source must be deactivated after merge"
        );
        assert_ne!(
            source_after.enrollment_secret_hash, "source-hash",
            "source hash must be invalidated after merge"
        );

        // Target must have adopted the source's identity.
        let target_after = Service::find_by_id(target.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(target_after.hostname, "source-host");
        assert_eq!(target_after.enrollment_secret_hash, "source-hash");
    }

    #[tokio::test]
    async fn merge_service_writes_service_merge_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = merge_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            None,
            Path(target.id),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        let status = match response {
            Ok(r) => r.into_response().status(),
            Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
        };
        assert_eq!(status, StatusCode::OK);

        let row = latest_tenant_audit_row(&db).await;
        let expected_target_id = target.id.to_string();
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["source_service_id"], serde_json::json!(source.id));
    }

    #[tokio::test]
    async fn merge_service_api_token_actor_writes_api_token_actor_type() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::ApiToken,
            permissions: vec![Permission::UpdateServices],
            jti: None,
        };
        let token_id = uuid::Uuid::now_v7();
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = merge_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            Some(Extension(AuthenticatedApiTokenId(token_id))),
            Path(target.id),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        let status = match response {
            Ok(r) => r.into_response().status(),
            Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
        };
        assert_eq!(status, StatusCode::OK);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, Some(token_id));
    }

    #[tokio::test]
    async fn approve_service_writes_service_approve_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (_target, source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ApproveServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = approve_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanApproveServices::new(auth_user),
            None,
            Path(source.id),
        )
        .await;

        let status = match response {
            Ok(r) => r.into_response().status(),
            Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
        };
        assert_eq!(status, StatusCode::OK);

        let row = latest_tenant_audit_row(&db).await;
        let expected_target_id = source.id.to_string();
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
    }

    #[tokio::test]
    async fn update_service_missing_service_writes_denied_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let missing_service_id = uuid::Uuid::now_v7();

        let response = update_service(
            State(AuditEmitterState(state.audit_emitter.clone())),
            tenant_db,
            CanUpdateServices::new(auth_user),
            None,
            Path(missing_service_id),
            Json(UpdateServiceRequest {
                ping_interval_seconds: Some(15),
                cert_lifetime_hours: Some(72),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_service_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service.not_found")
        );
    }

    #[tokio::test]
    async fn approve_service_missing_service_writes_denied_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ApproveServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let missing_service_id = uuid::Uuid::now_v7();

        let response = approve_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanApproveServices::new(auth_user),
            None,
            Path(missing_service_id),
        )
        .await;

        let status = match response {
            Err(e) => e.into_response().status(),
            Ok(_) => panic!("expected Err(ApiError) but got Ok"),
        };
        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_service_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service.not_found")
        );
    }

    #[tokio::test]
    async fn reject_service_writes_service_reject_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (_target, source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::RejectServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = reject_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanRejectServices::new(auth_user),
            None,
            Path(source.id),
        )
        .await;

        let status = match response {
            Ok(r) => r.into_response().status(),
            Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
        };
        assert_eq!(status, StatusCode::OK);

        let row = latest_tenant_audit_row(&db).await;
        let expected_target_id = source.id.to_string();
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
    }

    #[tokio::test]
    async fn deactivate_service_writes_service_deactivate_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, _source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::RemoveServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = deactivate_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanRemoveServices::new(auth_user),
            None,
            Path(target.id),
        )
        .await;

        let status = match response {
            Ok(r) => r.into_response().status(),
            Err(e) => panic!("expected Ok but got Err: {}", e.into_response().status()),
        };
        assert_eq!(status, StatusCode::NO_CONTENT);

        let row = latest_tenant_audit_row(&db).await;
        let expected_target_id = target.id.to_string();
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
    }

    #[tokio::test]
    async fn deactivate_service_missing_service_writes_denied_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::RemoveServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let missing_service_id = uuid::Uuid::now_v7();

        let response = deactivate_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanRemoveServices::new(auth_user),
            None,
            Path(missing_service_id),
        )
        .await;

        let status = match response {
            Ok(r) => r.into_response().status(),
            Err(e) => panic!(
                "expected Ok(response) but got Err: {}",
                e.into_response().status()
            ),
        };
        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_service_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service.not_found")
        );
    }

    #[tokio::test]
    async fn set_update_freeze_writes_service_freeze_enable_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, _source) = insert_target_and_source(&db, tenant_id).await;

        let caps = {
            use std::collections::BTreeSet;
            use uptrakit_wire::Capability;
            BTreeSet::from([
                Capability::GracefulShutdown,
                Capability::SoftwareDiscovery,
                Capability::UpdateHooks,
            ])
        };
        let (_rx, _token) = state
            .service_connections
            .register(target.id, caps, None, None, None)
            .await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = set_update_freeze(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            None,
            Path(target.id),
            Json(SetUpdateFreezeRequest {
                enabled: true,
                reason: Some("maintenance".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let row = latest_tenant_audit_row(&db).await;
        let expected_target_id = target.id.to_string();
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_ENABLE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
        let details = row.details_json.expect("details");
        assert_eq!(details["enabled"], serde_json::json!(true));
        assert_eq!(details["reason_present"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn set_update_freeze_not_connected_writes_denied_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, _source) = insert_target_and_source(&db, tenant_id).await;
        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = set_update_freeze(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            None,
            Path(target.id),
            Json(SetUpdateFreezeRequest {
                enabled: false,
                reason: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_DISABLE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(target.id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["enabled"], serde_json::json!(false));
        assert_eq!(details["reason_present"], serde_json::json!(false));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service.not_connected")
        );
    }

    #[tokio::test]
    async fn batch_services_invalid_request_writes_validation_failed_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ApproveServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = batch_services(
            State(Arc::clone(&state)),
            tenant_db,
            Extension(auth_user),
            None,
            Json(BatchActionRequest {
                action: "approve".to_string(),
                ids: vec![],
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
        assert_eq!(details["batch"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn batch_services_permission_denied_writes_denied_audit_event() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, _source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewServices],
            jti: None,
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = batch_services(
            State(Arc::clone(&state)),
            tenant_db,
            Extension(auth_user),
            None,
            Json(BatchActionRequest {
                action: "approve".to_string(),
                ids: vec![target.id],
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("insufficient_permissions")
        );
        assert_eq!(details["batch"], serde_json::json!(true));
    }
}
