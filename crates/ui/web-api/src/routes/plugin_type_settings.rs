use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanManageCommands, CanViewSoftware};
use crate::queries::plugin_type_settings as pts_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uptrakit_shared_types::PluginType;
use uptrakit_web_api_types::plugin_type_settings::{
    PluginTypeSettingsResponse, UpsertPluginTypeSettingsRequest,
};

/// Convert a `plugin_type_setting::Model` into the API response type.
fn model_to_response(
    model: uptrakit_shared_db::entity::plugin_type_setting::Model,
) -> PluginTypeSettingsResponse {
    PluginTypeSettingsResponse {
        plugin_type: PluginType::from(model.plugin_type),
        config: model.config,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

/// List all plugin type settings for the current tenant.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-type-settings",
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "List of plugin type settings", body = Vec<PluginTypeSettingsResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_plugin_type_settings(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
) -> Response {
    match pts_queries::list_type_settings(tenant_db.db(), tenant_db.tenant_id).await {
        Ok(models) => {
            let responses: Vec<PluginTypeSettingsResponse> =
                models.into_iter().map(model_to_response).collect();
            (StatusCode::OK, Json(responses)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list plugin type settings: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get plugin type settings for a specific plugin type.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Plugin type settings", body = PluginTypeSettingsResponse),
        (status = 404, description = "No settings found for this plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_plugin_type_settings(
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    CanViewSoftware(_user): CanViewSoftware,
) -> Response {
    match pts_queries::get_type_settings(tenant_db.db(), tenant_db.tenant_id, &plugin_type).await {
        Ok(Some(model)) => (StatusCode::OK, Json(model_to_response(model))).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "No settings found for this plugin type",
        ),
        Err(e) => {
            tracing::error!("Failed to get plugin type settings: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Create or update plugin type settings for a specific plugin type.
///
/// If settings already exist for the given plugin type, they are updated.
/// Otherwise, new settings are created.
#[utoipa::path(
    put,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    request_body = UpsertPluginTypeSettingsRequest,
    extensions(("x-required-permission" = json!("manage_commands"))),
    responses(
        (status = 200, description = "Plugin type settings created or updated", body = PluginTypeSettingsResponse),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn upsert_plugin_type_settings(
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    CanManageCommands(user): CanManageCommands,
    Validated(req): Validated<UpsertPluginTypeSettingsRequest>,
) -> Response {
    match pts_queries::upsert_type_settings(
        tenant_db.db(),
        tenant_db.tenant_id,
        &plugin_type,
        req.config,
    )
    .await
    {
        Ok(model) => {
            tracing::info!(
                user_id = %user.user_id,
                tenant_id = %tenant_db.tenant_id,
                %plugin_type,
                "security_audit: plugin type settings upserted"
            );
            (StatusCode::OK, Json(model_to_response(model))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to upsert plugin type settings: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Delete plugin type settings, resetting to defaults.
#[utoipa::path(
    delete,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    extensions(("x-required-permission" = json!("manage_commands"))),
    responses(
        (status = 204, description = "Plugin type settings deleted (reset to defaults)"),
        (status = 404, description = "No settings found for this plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_type_settings(
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    CanManageCommands(user): CanManageCommands,
) -> Response {
    match pts_queries::delete_type_settings(tenant_db.db(), tenant_db.tenant_id, &plugin_type).await
    {
        Ok(true) => {
            tracing::warn!(
                user_id = %user.user_id,
                tenant_id = %tenant_db.tenant_id,
                %plugin_type,
                "security_audit: plugin type settings deleted"
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "No settings found for this plugin type",
        ),
        Err(e) => {
            tracing::error!("Failed to delete plugin type settings: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
