mod audit;
mod crud;
mod lifecycle;

pub use crud::{
    __path_get_service, __path_list_services, __path_update_service, get_service, list_services,
    update_service,
};
pub use lifecycle::{
    __path_approve_service, __path_deactivate_service, __path_reject_service,
    __path_set_update_freeze, approve_service, deactivate_service, reject_service,
    set_update_freeze,
};

use crate::AppState;
use crate::actions::services as svc_actions;
use crate::api_error::ApiError;
use crate::auth::permissions::Permission;
use crate::error_response::{error_response, error_response_with_code};
use crate::middleware::permission::CanUpdateServices;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
use crate::queries::services as svc_queries;
use crate::tenant_db::TenantDb;
use audit::{
    AuditContext, batch_action_to_audit_action, emit_service_batch_audit,
    emit_service_lifecycle_audit,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
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
pub use uptrakit_web_api_types::services::{
    ListServicesQuery, MergeAgentRequest, MessageResponse, ServiceResponse, ServiceStatus,
    SetUpdateFreezeRequest, UpdateServiceRequest,
};

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

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
        tenant_id: tenant_db.tenant_id(),
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

    // Defence-in-depth: short-circuit with a clear 400 + audit before any
    // session/state interaction. The query layer also rejects this (see
    // ServiceQueryError::{Target,Source}Embedded) but the route layer owns
    // the audit emission for ValidationFailed outcomes.
    let (target_embedded, source_embedded) =
        match svc_queries::is_embedded_pair(&tenant_db, target_uuid, source_uuid).await {
            Ok(pair) => pair,
            Err(err) => return Err(ApiError::from(err)),
        };
    if target_embedded {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.embedded_target",
            }),
        );
        return Ok(error_response_with_code(
            StatusCode::BAD_REQUEST,
            "Cannot merge into an embedded service.",
            "service.embedded_target",
        ));
    }
    if source_embedded {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.embedded_source",
            }),
        );
        return Ok(error_response_with_code(
            StatusCode::BAD_REQUEST,
            "Cannot merge from an embedded service.",
            "service.embedded_source",
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
        tenant_id: tenant_db.tenant_id(),
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
mod tests;
