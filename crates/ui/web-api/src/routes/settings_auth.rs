use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uptrakit_shared_db::entity::oidc_provider;
use uptrakit_shared_db::entity::prelude::*;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct AuthenticationSettingsResponse {
    pub password_auth_enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateAuthenticationSettingsRequest {
    pub password_auth_enabled: Option<bool>,
}

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
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let auth_settings = state.settings.authentication().await;
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
    Json(req): Json<UpdateAuthenticationSettingsRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    if let Some(password_enabled) = req.password_auth_enabled {
        if !password_enabled {
            // Safety: cannot disable password auth if current session uses password
            if user.auth_method == AuthMethod::Password {
                return (
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication while logged in with a password",
                )
                    .into_response();
            }

            // Safety: at least one auth method must remain
            let active_providers = OidcProvider::find()
                .filter(oidc_provider::Column::IsActive.eq(true))
                .filter(oidc_provider::Column::DeletedAt.is_null())
                .all(&state.db)
                .await
                .unwrap_or_default();

            if active_providers.is_empty() {
                return (
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication with no active OIDC providers",
                )
                    .into_response();
            }
        }

        let mut auth_settings = state.settings.authentication_write().await;
        auth_settings.password_auth_enabled = password_enabled;
        if let Err(e) = auth_settings.save(&state.db).await {
            tracing::error!("Failed to save authentication settings: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let auth_settings = state.settings.authentication().await;
    let response = AuthenticationSettingsResponse {
        password_auth_enabled: auth_settings.password_auth_enabled,
    };
    (StatusCode::OK, Json(response)).into_response()
}
