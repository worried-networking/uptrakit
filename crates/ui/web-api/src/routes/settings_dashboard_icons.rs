//! HTTP handlers for `GET /api/v1/settings/dashboard-icons` and
//! `PUT /api/v1/settings/dashboard-icons`.
//!
//! Per-tenant toggle for the Dashboard Icons enhancement plugin. When enabled,
//! newly created software items are automatically enriched with icon URLs from
//! the community-curated Dashboard Icons collection. Unset defaults to enabled.

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
use sea_orm::DatabaseConnection;
use uuid::Uuid;

/// Return the effective Dashboard Icons state for a tenant.
///
/// Explicit `false` disables the feature. Explicit `true` and an unset value
/// both enable it.
pub(crate) async fn is_dashboard_icons_enabled(db: &DatabaseConnection, tenant_id: Uuid) -> bool {
    !matches!(
        load_setting(db, tenant_id, SettingKey::DashboardIconsEnabled).await,
        Ok(Some(serde_json::Value::Bool(false)))
    )
}

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
    let enabled = is_dashboard_icons_enabled(tenant_db.db(), tenant_db.tenant_id).await;

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

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::test_harness::{insert_default_tenant, setup_migrated_db};

    #[tokio::test]
    async fn setting_defaults_to_enabled_when_unset() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;

        assert!(is_dashboard_icons_enabled(&db, tenant_id).await);
    }

    #[tokio::test]
    async fn setting_respects_explicit_false() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        upsert_setting(
            &db,
            tenant_id,
            SettingKey::DashboardIconsEnabled,
            serde_json::json!(false),
        )
        .await
        .expect("save setting");

        assert!(!is_dashboard_icons_enabled(&db, tenant_id).await);
    }

    #[tokio::test]
    async fn setting_respects_explicit_true() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        upsert_setting(
            &db,
            tenant_id,
            SettingKey::DashboardIconsEnabled,
            serde_json::json!(true),
        )
        .await
        .expect("save setting");

        assert!(is_dashboard_icons_enabled(&db, tenant_id).await);
    }
}
