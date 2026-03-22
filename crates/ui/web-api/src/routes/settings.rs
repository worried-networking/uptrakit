use crate::AppState;
use crate::auth::registration::RegistrationMode;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use axum::{
    Json,
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
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_registration_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let reg = state.settings.registration();
    let response = RegistrationSettingsResponse {
        mode: reg.mode,
        require_token_for_oidc: reg.require_token_for_oidc,
    };

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
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_registration_settings(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(_user): CanManageAuthSettings,
    Json(req): Json<UpdateRegistrationSettingsRequest>,
) -> Response {
    // Validate: invite mode requires a token
    if req.mode == RegistrationMode::Invite && req.token.is_none() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Token is required when mode is invite",
        );
    }

    let mut reg = state.settings.registration();
    if let Err(e) = reg
        .update(
            state.db(),
            state.default_tenant_id,
            req.mode,
            req.token.map(|t| t.expose_secret().to_string()),
            req.require_token_for_oidc,
        )
        .await
    {
        tracing::error!(error = ?e, "Failed to update registration settings");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    state.settings.set_registration(reg).await;

    let reg = state.settings.registration();
    let response = RegistrationSettingsResponse {
        mode: reg.mode,
        require_token_for_oidc: reg.require_token_for_oidc,
    };

    (StatusCode::OK, Json(response)).into_response()
}
