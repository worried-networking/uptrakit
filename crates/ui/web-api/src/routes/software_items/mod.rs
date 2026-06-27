//! HTTP route handlers for `/api/v1/software-items`.
//!
//! Controller-side fetch orchestration lives in [`controller_fetch`].
//! Version-check context loading and agent dispatch live in [`version_check_dispatch`].

mod audit;
mod controller_fetch;
mod crud;
mod host_assignments;
mod version_check;
mod version_check_dispatch;

pub use crud::{
    approve_software_item, create_software_item, delete_software_item, get_software_item,
    list_software_items, update_software_item,
};
// Re-export utoipa `__path_*` types so `routes!(crate::routes::software_items::<handler>)`
// in router.rs resolves them at the facade's public path.
pub use crud::{
    __path_approve_software_item, __path_create_software_item, __path_delete_software_item,
    __path_get_software_item, __path_list_software_items, __path_update_software_item,
};
pub use host_assignments::{
    __path_assign_hosts, __path_delete_plugin_assignment, __path_unassign_host,
    __path_update_host_assignment,
};
pub use host_assignments::{
    DeleteHostAssignmentParams, assign_hosts, delete_plugin_assignment, unassign_host,
    update_host_assignment,
};
pub use version_check::{__path_check_versions, __path_check_versions_host};
pub use version_check::{check_versions, check_versions_host};

use crate::AppState;
use crate::actions::software_items as item_actions;
use crate::api_error::ApiError;
use crate::app_state::AuditEmitterState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanDeleteSoftware, CanTriggerUpdates, CanUpdateSoftware};
use crate::middleware::require_auth::AuthenticatedApiTokenId;
use crate::queries::software_items as item_queries;
use crate::queries::update_types::ActorType;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, ListSoftwareItemsParams,
    MergeSoftwareItemsExecuteRequest, MergeSoftwareItemsExecuteResponse,
    MergeSoftwareItemsPreviewRequest, MergeSoftwareItemsPreviewResponse,
    SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse,
    TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus, TriggerVersionCheckResponse,
    UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
};

use audit::{
    AuditContext, SOFTWARE_ITEM_BATCH_AUDIT_ACTION, SOFTWARE_ITEM_MERGE_AUDIT_ACTION,
    emit_software_item_mutation_audit,
};

// --- Endpoints ---

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

/// Trigger a software update for a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/update",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = TriggerUpdateRequest,
    extensions(("x-required-permission" = json!("trigger_updates"))),
    responses(
        (status = 200, description = "Update triggered", body = TriggerUpdateResponse),
        (status = 400, description = "Invalid input or validation failed"),
        (status = 404, description = "Software item, host, or agent not found"),
        (status = 409, description = "Update already in progress")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn trigger_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerUpdates(user): CanTriggerUpdates,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<TriggerUpdateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (update_actor_type, update_actor_id) = match api_token_id {
        Some(token_id) => (ActorType::ApiToken, token_id.0.to_string()),
        None => (ActorType::User, user.user_id.to_string()),
    };
    let to_version = req.to_version.clone();
    let interactive = req.interactive;
    // Convert the API release_info type to the wire type then serialise for
    // UpdateDispatchParams, which carries an opaque JSON value.
    let release_info = req.release_info.map(|ri| {
        let wire = uptrakit_wire::ReleaseInfo {
            tag: ri.tag,
            release_url: ri.release_url,
            assets: ri
                .assets
                .into_iter()
                .map(|a| uptrakit_wire::ReleaseAsset {
                    name: a.name,
                    download_url: a.download_url,
                    size: a.size,
                    content_type: None,
                    sha256_digest: None,
                })
                .collect(),
            // Attestation fields are enriched server-side at dispatch time from
            // latest_release_metadata and the fetch_releases plugin config.
            attestation_status: None,
            require_attestation: false,
        };
        serde_json::to_value(wire).unwrap_or_else(|e| {
            tracing::warn!("failed to serialize release info: {e}");
            serde_json::Value::Null
        })
    });

    let result = state
        .update_dispatcher
        .dispatch(uptrakit_controller_core::update::UpdateDispatchParams::new(
            tenant_db.tenant_id(),
            host_id,
            item_id,
            to_version.clone(),
            uptrakit_controller_core::update::ActorInfo::new(update_actor_type, update_actor_id),
            release_info,
            interactive,
        ))
        .await?;

    let status = match result.outcome {
        uptrakit_controller_core::update::DispatchOutcome::Sent => TriggerUpdateStatus::Pending,
        uptrakit_controller_core::update::DispatchOutcome::Queued => TriggerUpdateStatus::Queued,
        uptrakit_controller_core::update::DispatchOutcome::Failed => TriggerUpdateStatus::Failed,
        _ => {
            tracing::warn!("unhandled DispatchOutcome in HTTP trigger_update response mapping");
            TriggerUpdateStatus::Failed
        }
    };

    let resp = TriggerUpdateResponse {
        update_history_id: result.update_history_id,
        status,
    };
    Ok((StatusCode::OK, Json(resp)).into_response())
}

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

#[cfg(all(test, feature = "db-sqlite"))]
mod audit_tests;
#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
