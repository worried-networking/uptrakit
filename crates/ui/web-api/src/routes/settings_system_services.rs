//! HTTP handlers for `GET /api/v1/settings/system-services` and
//! `PUT /api/v1/settings/system-services`.
//!
//! The system services enrollment token is stored encrypted at rest with the
//! master key. Unlike SMTP passwords, the token **is** returned in plaintext in
//! the GET response so that operators can copy it into service deployment
//! configurations.
//!
//! When no token is configured, all system service enrollments are placed in
//! `Pending` status and require manual approval via the system services API.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub use uptrakit_web_api_types::settings_system_services::{
    SystemServicesSettingsResponse, UpdateSystemServicesSettingsRequest,
};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageSystemServices;
use crate::settings_store::upsert_global_setting;

fn snapshot_to_response(token: Option<String>) -> SystemServicesSettingsResponse {
    let has_token = token.is_some();
    SystemServicesSettingsResponse {
        enrollment_token: token,
        has_token,
    }
}

/// Get system services settings
///
/// Returns the current system services enrollment token configuration.
/// The plaintext token is included in the response so that operators can
/// configure their service deployments. Requires the `manage_system_services`
/// permission.
#[utoipa::path(
    get,
    path = "/api/v1/settings/system-services",
    responses(
        (status = 200, description = "System services settings", body = SystemServicesSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
pub async fn get_system_services_settings(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
) -> Response {
    let token = state.settings.system_services_enrollment_token();
    (StatusCode::OK, Json(snapshot_to_response(token))).into_response()
}

/// Update system services settings
///
/// Set or clear the system services enrollment token. When a token is
/// configured, system services that supply this token during enrollment are
/// automatically approved. When cleared (`null`), all enrollments require
/// manual approval.
///
/// The token is encrypted at rest. Send `"enrollment_token": null` to clear it.
#[utoipa::path(
    put,
    path = "/api/v1/settings/system-services",
    request_body = UpdateSystemServicesSettingsRequest,
    responses(
        (status = 200, description = "System services settings updated", body = SystemServicesSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_system_services"))),
    security(("bearer_token" = []))
)]
pub async fn update_system_services_settings(
    State(state): State<Arc<AppState>>,
    CanManageSystemServices(_user): CanManageSystemServices,
    Json(req): Json<UpdateSystemServicesSettingsRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let mut token = state.settings.system_services_enrollment_token();

    if let Some(ref val) = req.enrollment_token {
        if val.is_null() {
            if let Err(e) = upsert_global_setting(
                state.db(),
                SettingKey::SystemServicesEnrollmentToken,
                serde_json::json!(""),
            )
            .await
            {
                tracing::error!("Failed to clear system_services.enrollment_token: {e:?}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            token = None;
        } else if let Some(s) = val.as_str() {
            let stored_value = match uptrakit_crypto::encrypt_str(s, "uptrakit:settings:system_services_enrollment_token") {
                Ok(encrypted) => serde_json::json!(encrypted),
                Err(e) => {
                    tracing::error!(
                        "Failed to encrypt system_services.enrollment_token: {e:?}"
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };
            if let Err(e) = upsert_global_setting(
                state.db(),
                SettingKey::SystemServicesEnrollmentToken,
                stored_value,
            )
            .await
            {
                tracing::error!("Failed to save system_services.enrollment_token: {e:?}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            token = Some(s.to_string());
        }
    }

    state
        .settings
        .set_system_services_enrollment_token(token.clone())
        .await;

    (StatusCode::OK, Json(snapshot_to_response(token))).into_response()
}
