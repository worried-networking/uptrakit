use crate::AppState;
use crate::actions::settings as settings_actions;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_web_api_types::settings_reset::{ResetDataRequest, ResetDataResponse};

/// Reset all tenant-scoped data (hosts, software items, configs, history, etc.)
#[utoipa::path(
    post,
    path = "/api/v1/settings/reset-data",
    request_body = ResetDataRequest,
    responses(
        (status = 200, description = "Data reset successfully", body = ResetDataResponse),
        (status = 400, description = "Invalid request (confirm != RESET)"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn reset_data(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    tenant_db: TenantDb,
    Validated(_req): Validated<ResetDataRequest>,
) -> Response {
    let ctx = state.mutation_context();
    match settings_actions::reset_data(&tenant_db, &ctx, &state.service_connections).await {
        Ok(counts) => (StatusCode::OK, Json(ResetDataResponse { deleted: counts })).into_response(),
        Err(e) => {
            tracing::error!("failed to reset tenant data: {:?}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
