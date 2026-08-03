use crate::app_state::AuditEmitterState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::extractors::{IfMatch, SettingsVersion};
use crate::middleware::action::CanManageCommands;
use crate::middleware::require_auth::AuthenticatedApiTokenId;
use crate::queries::plugin_configs as pc_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};

use super::audit::{AuditContext, emit_plugin_config_semantic_audit};

/// Perform a batch action on multiple plugin configs.
///
/// Supported actions: `delete`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/plugin-configs/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Plugin Configs",
    security(("oauth2" = ["commands:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_plugin_configs(
    State(audit_emitter_state): State<AuditEmitterState>,
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };

    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => match pc_queries::batch_delete_plugin_configs(&tenant_db, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete failed: {e}");
                emit_plugin_config_semantic_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                    None,
                    None,
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "update_kind": "batch_delete",
                        "reason_code": "batch_delete_failed",
                        "requested_count": body.ids.len(),
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        unknown => {
            emit_plugin_config_semantic_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                None,
                None,
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "update_kind": "batch_delete",
                    "reason_code": "unknown_action",
                    "action": unknown,
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: delete"),
            );
        }
    };

    emit_plugin_config_semantic_audit(
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        None,
        None,
        None,
        if failed.is_empty() {
            uptrakit_audit_log::AuditOutcome::Success
        } else if succeeded_ids.is_empty() {
            uptrakit_audit_log::AuditOutcome::Denied
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        },
        serde_json::json!({
            "update_kind": "batch_delete",
            "requested_count": body.ids.len(),
            "deleted_count": succeeded_ids.len(),
            "failed_count": failed.len(),
        }),
    );

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
