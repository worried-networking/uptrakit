use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
#[cfg(feature = "oidc")]
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_shared_db::entity::prelude::AuthMethod;
#[cfg(feature = "oidc")]
use {
    sea_orm::{ColumnTrait, QueryFilter},
    uptrakit_shared_db::entity::oidc_provider,
};

pub use uptrakit_web_api_types::settings_auth::{
    AuthenticationSettingsResponse, UpdateAuthenticationSettingsRequest,
};

/// Get authentication settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/authentication",
    responses(
        (status = 200, description = "Authentication settings", body = AuthenticationSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn get_authentication_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let auth_settings = state.settings.authentication();
    let response = AuthenticationSettingsResponse {
        password_auth_enabled: auth_settings.password_auth_enabled,
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Update authentication settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/authentication",
    request_body = UpdateAuthenticationSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = AuthenticationSettingsResponse),
        (status = 409, description = "Safety check failed")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn update_authentication_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
    #[cfg(feature = "oidc")] tenant_db: TenantDb,
    Json(req): Json<UpdateAuthenticationSettingsRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    if let Some(password_enabled) = req.password_auth_enabled {
        if !password_enabled {
            // Safety: cannot disable password auth if current session uses password
            if user.auth_method == AuthMethod::Password {
                return error_response(
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication while logged in with a password",
                );
            }

            // Safety: at least one auth method must remain
            #[cfg(feature = "oidc")]
            {
                let active_providers = tenant_db
                    .find::<oidc_provider::Entity>()
                    .filter(oidc_provider::Column::IsActive.eq(true))
                    .filter(oidc_provider::Column::DeletedAt.is_null())
                    .all(tenant_db.db())
                    .await
                    .unwrap_or_default();

                if active_providers.is_empty() {
                    return error_response(
                        StatusCode::CONFLICT,
                        "Cannot disable password authentication with no active OIDC providers",
                    );
                }
            }

            #[cfg(not(feature = "oidc"))]
            {
                return error_response(
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication: OIDC support is not enabled",
                );
            }
        }

        let mut auth_settings = state.settings.authentication();
        auth_settings.password_auth_enabled = password_enabled;
        if let Err(e) = auth_settings
            .save(state.db(), state.default_tenant_id)
            .await
        {
            tracing::error!("Failed to save authentication settings: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_authentication(auth_settings).await;
    }

    let auth_settings = state.settings.authentication();
    let response = AuthenticationSettingsResponse {
        password_auth_enabled: auth_settings.password_auth_enabled,
    };
    (StatusCode::OK, Json(response)).into_response()
}
