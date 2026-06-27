//! Merge handlers for `/api/v1/software-items/merge`.

use axum::{Extension, Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::api_error::ApiError;
use crate::app_state::AuditEmitterState;
use crate::middleware::permission::{CanDeleteSoftware, CanUpdateSoftware};
use crate::middleware::require_auth::AuthenticatedApiTokenId;
use crate::queries::software_items as item_queries;
use crate::tenant_db::TenantDb;

use super::audit::{
    AuditContext, SOFTWARE_ITEM_MERGE_AUDIT_ACTION, emit_software_item_mutation_audit,
};
use super::{
    MergeSoftwareItemsExecuteRequest, MergeSoftwareItemsExecuteResponse,
    MergeSoftwareItemsPreviewRequest, MergeSoftwareItemsPreviewResponse,
};

/// Preview a manual merge of software items.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/merge/preview",
    request_body = MergeSoftwareItemsPreviewRequest,
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 200, description = "Merge preview calculated", body = MergeSoftwareItemsPreviewResponse),
        (status = 400, description = "Invalid merge request")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn preview_software_item_merge(
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Json(req): Json<MergeSoftwareItemsPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = item_queries::preview_merge_software_items(&tenant_db, &req).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Execute a manual merge of software items.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/merge/execute",
    request_body = MergeSoftwareItemsExecuteRequest,
    extensions(("x-required-permission" = json!("update_software and delete_software"))),
    responses(
        (status = 200, description = "Software items merged", body = MergeSoftwareItemsExecuteResponse),
        (status = 400, description = "Invalid merge request")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn execute_software_item_merge(
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanUpdateSoftware(update_user): CanUpdateSoftware,
    CanDeleteSoftware(_delete_user): CanDeleteSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<MergeSoftwareItemsExecuteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id(),
        user: &update_user,
        api_token_id,
    };
    let requested_count = req.candidate_ids.len();
    let resp = match item_queries::execute_merge_software_items(&tenant_db, &req).await {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_MERGE_AUDIT_ACTION,
                req.survivor_id.to_string(),
                None,
                outcome,
                serde_json::json!({
                    "reason_code": reason_code,
                    "candidate_count": requested_count,
                }),
            );
            return Err(err.into());
        }
    };

    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_MERGE_AUDIT_ACTION,
        resp.survivor_id.to_string(),
        None,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "candidate_count": requested_count,
            "deleted_count": resp.deleted_ids.len(),
            "moved_link_count": resp.moved_link_ids.len(),
            "skipped_duplicate_link_count": resp.skipped_duplicate_link_ids.len(),
        }),
    );

    Ok((StatusCode::OK, Json(resp)).into_response())
}
