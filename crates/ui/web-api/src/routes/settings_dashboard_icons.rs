//! HTTP handlers for `GET /api/v1/settings/dashboard-icons` and
//! `PUT /api/v1/settings/dashboard-icons`.
//!
//! Per-tenant toggle for the Dashboard Icons enhancement plugin. When enabled,
//! newly created software items are automatically enriched with icon URLs from
//! the community-curated Dashboard Icons collection.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub use uptrakit_web_api_types::settings_dashboard_icons::{
    DashboardIconsSettingsResponse, UpdateDashboardIconsSettingsRequest,
};

use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanManageGlobalSettings, CanViewSettings};
use crate::settings_store::{load_setting, upsert_setting};
use crate::tenant_db::TenantDb;

/// Get Dashboard Icons settings
///
/// Returns whether the Dashboard Icons enhancement is enabled for this tenant.
#[utoipa::path(
    get,
    path = "/api/v1/settings/dashboard-icons",
    responses(
        (status = 200, description = "Dashboard Icons settings", body = DashboardIconsSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_dashboard_icons_settings(
    tenant_db: TenantDb,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let enabled = match load_setting(
        tenant_db.db(),
        tenant_db.tenant_id,
        SettingKey::DashboardIconsEnabled,
    )
    .await
    {
        Ok(Some(serde_json::Value::Bool(val))) => val,
        _ => false,
    };

    let resp = DashboardIconsSettingsResponse { enabled };
    (StatusCode::OK, Json(resp)).into_response()
}

/// Update Dashboard Icons settings
///
/// Enable or disable the Dashboard Icons enhancement for this tenant. When
/// enabled, newly created and auto-discovered software items are automatically
/// enriched with icon URLs from the Dashboard Icons project.
#[utoipa::path(
    put,
    path = "/api/v1/settings/dashboard-icons",
    request_body = UpdateDashboardIconsSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = DashboardIconsSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_dashboard_icons_settings(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Validated(req): Validated<UpdateDashboardIconsSettingsRequest>,
) -> Response {
    if let Err(e) = upsert_setting(
        state.db(),
        tenant_db.tenant_id,
        SettingKey::DashboardIconsEnabled,
        serde_json::json!(req.enabled),
    )
    .await
    {
        tracing::error!("Failed to save dashboard_icons.enabled: {e:?}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    let resp = DashboardIconsSettingsResponse {
        enabled: req.enabled,
    };
    (StatusCode::OK, Json(resp)).into_response()
}
