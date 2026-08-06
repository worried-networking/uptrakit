//! Merge handlers for `/api/v1/software-items/merge`.

use axum::{Extension, Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::api_error::ApiError;
use crate::app_state::AuditEmitterState;
use crate::extract::Unvalidated;
use crate::middleware::action::{CanDeleteSoftware, CanUpdateSoftware};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
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
    responses(
        (status = 200, description = "Merge preview calculated", body = MergeSoftwareItemsPreviewResponse),
        (status = 400, description = "Invalid merge request")
    ),
    tag = "Software Items",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn preview_software_item_merge(
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    body: Unvalidated<MergeSoftwareItemsPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                e.to_string(),
                "validation_error",
                None,
            ));
        }
    };
    let resp = item_queries::preview_merge_software_items(&tenant_db, &req).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Execute a manual merge of software items.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/merge/execute",
    request_body = MergeSoftwareItemsExecuteRequest,
    responses(
        (status = 200, description = "Software items merged", body = MergeSoftwareItemsExecuteResponse),
        (status = 400, description = "Invalid merge request")
    ),
    tag = "Software Items",
    security(("oauth2" = ["software:delete", "software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn execute_software_item_merge(
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanUpdateSoftware(update_user): CanUpdateSoftware,
    CanDeleteSoftware(_delete_user): CanDeleteSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    body: Unvalidated<MergeSoftwareItemsExecuteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            let (actor_type, actor_id) = authenticated_user_audit_actor(&update_user, api_token_id);
            if let Ok(entry) =
                uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    SOFTWARE_ITEM_MERGE_AUDIT_ACTION,
                )
                .tenant_scope(tenant_db.tenant_id())
                .actor(actor_type, actor_id)
                .outcome(uptrakit_audit_log::AuditOutcome::ValidationFailed)
                .details(serde_json::json!({
                    "reason_code": "invalid_request",
                }))
                .build()
            {
                audit_emitter_state.0.emit_event(entry);
            }
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                e.to_string(),
                "validation_error",
                None,
            ));
        }
    };
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
