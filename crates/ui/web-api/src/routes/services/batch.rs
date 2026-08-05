//! Batch service-action route handler.

use super::audit::{AuditContext, batch_action_to_audit_action, emit_service_batch_audit};
use super::{BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess};
use crate::AppState;
use crate::actions::services as svc_actions;
use crate::error_response::error_response;
use crate::middleware::action::{AccessAuthority, authorize_any};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_shared_types::access::actions;
use uptrakit_web_api_types::validation::Validate;

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
    security(
        ("oauth2" = ["services:approve"]),
        ("oauth2" = ["services:reject"]),
        ("oauth2" = ["services:delete"]),
        ("developer_token" = [])
    )
)]
#[tracing::instrument(skip_all)]
pub async fn batch_services(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Extension(auth_user): Extension<AuthenticatedUser>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Extension(authority): Extension<AccessAuthority>,
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

    let access_ctx = match authority {
        AccessAuthority::Ready(access_ctx) => access_ctx,
        AccessAuthority::Unavailable => {
            if let Some(action_type) = action_type {
                emit_service_batch_audit(
                    &audit_ctx,
                    action_type,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "authorization_unavailable",
                        "batch": true,
                        "requested_count": body.ids.len(),
                    }),
                );
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

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

    let required_actions = match body.action.as_str() {
        "approve" => &[actions::SERVICES_APPROVE][..],
        "reject" => &[actions::SERVICES_REJECT][..],
        "deactivate" => &[actions::SERVICES_DELETE][..],
        _ => return error_response(StatusCode::BAD_REQUEST, "Unknown batch action"),
    };
    if authorize_any(&state.access_engine, &access_ctx, required_actions).is_err() {
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
