//! Batch handler for `/api/v1/software-items/batch`.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::actions::software_items as item_actions;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanDeleteSoftware;
use crate::middleware::require_auth::AuthenticatedApiTokenId;
use crate::tenant_db::TenantDb;

use super::audit::{
    AuditContext, SOFTWARE_ITEM_BATCH_AUDIT_ACTION, emit_software_item_mutation_audit,
};
use super::{BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess};

/// Perform a batch action on multiple software items.
///
/// Supported actions: `approve`, `delete`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/software-items/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Software Items",
    extensions(("x-required-permission" = json!("delete_software"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_software_items(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanDeleteSoftware(user): CanDeleteSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let ctx = state.mutation_context();
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };
    let requested_count = body.ids.len();

    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match item_actions::batch_feature(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                emit_software_item_mutation_audit(
                    &audit_ctx,
                    SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
                    "batch".to_string(),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "action": body.action,
                        "requested_count": requested_count,
                        "reason_code": "software_item.batch_approve_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "delete" => match item_actions::batch_delete(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete failed: {e}");
                emit_software_item_mutation_audit(
                    &audit_ctx,
                    SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
                    "batch".to_string(),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "action": body.action,
                        "requested_count": requested_count,
                        "reason_code": "software_item.batch_delete_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        unknown => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
                "batch".to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "action": unknown,
                    "requested_count": requested_count,
                    "reason_code": "software_item.batch_unknown_action",
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: approve, delete"),
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

    let outcome = if response.failed.is_empty() {
        uptrakit_audit_log::AuditOutcome::Success
    } else {
        uptrakit_audit_log::AuditOutcome::Partial
    };
    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
        "batch".to_string(),
        None,
        outcome,
        serde_json::json!({
            "action": body.action,
            "requested_count": requested_count,
            "succeeded_count": response.succeeded.len(),
            "failed_count": response.failed.len(),
        }),
    );

    (StatusCode::OK, Json(response)).into_response()
}
