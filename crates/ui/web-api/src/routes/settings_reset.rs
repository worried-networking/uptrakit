use crate::AppState;
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
use uptrakit_internal_wire::{Capability, ControllerMessage};
use uptrakit_web_api_types::events::AdminEvent;
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
    let tenant_id = tenant_db.tenant_id;

    let counts =
        match uptrakit_web_api_queries::queries::reset_data::reset_tenant_data(&tenant_db).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("failed to reset tenant data: {:?}", e);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    // Notify connected services with ResetData capability to clear local stores
    state
        .service_connections
        .broadcast_by_capability(&Capability::ResetData, ControllerMessage::ResetData)
        .await;

    // Broadcast SSE event to admin subscribers
    state
        .event_broadcaster
        .send(tenant_id, AdminEvent::DataReset)
        .await;

    tracing::info!(
        hosts = counts.hosts,
        software_items = counts.software_items,
        plugin_configs = counts.plugin_configs,
        host_tags = counts.host_tags,
        update_history = counts.update_history,
        update_batches = counts.update_batches,
        "tenant data reset completed"
    );

    (StatusCode::OK, Json(ResetDataResponse { deleted: counts })).into_response()
}
