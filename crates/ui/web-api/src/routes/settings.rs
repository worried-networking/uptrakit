use crate::AppState;
use crate::auth::permissions::Permission;
use crate::auth::registration::RegistrationMode;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub use uptrakit_web_api_types::settings::{
    RegistrationSettingsResponse, UpdateRegistrationSettingsRequest,
};

/// Get current registration settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/registration",
    responses(
        (status = 200, description = "Current registration settings", body = RegistrationSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn get_registration_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let reg = state.settings.registration().await;
    let response = RegistrationSettingsResponse { mode: reg.mode };

    (StatusCode::OK, Json(response)).into_response()
}

/// Update registration settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/registration",
    request_body = UpdateRegistrationSettingsRequest,
    responses(
        (status = 200, description = "Registration settings updated", body = RegistrationSettingsResponse),
        (status = 400, description = "Invalid request (e.g., invite mode without token)"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn update_registration_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(req): Json<UpdateRegistrationSettingsRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    // Validate: invite mode requires a token
    if req.mode == RegistrationMode::Invite && req.token.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "Token is required when mode is invite",
        )
            .into_response();
    }

    if let Err(e) = state
        .settings
        .registration_write()
        .await
        .update(&state.db, state.default_tenant_id, req.mode, req.token)
        .await
    {
        tracing::error!("Failed to update registration settings: {:?}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let reg = state.settings.registration().await;
    let response = RegistrationSettingsResponse { mode: reg.mode };

    (StatusCode::OK, Json(response)).into_response()
}
