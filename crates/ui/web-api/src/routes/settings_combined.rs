use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::action::CanReadSettings;
use crate::queries::enrollment_tokens as et_queries;
use uptrakit_web_api_types::enrollment_tokens::EnrollmentTokensSummary;
use uptrakit_web_api_types::settings_agent_certs::AgentCertificateSettingsResponse;
use uptrakit_web_api_types::settings_combined::CombinedSettingsResponse;

/// Get core settings for the settings page (excludes access settings, which self-load).
#[utoipa::path(
    get,
    path = "/api/v1/settings",
    responses(
        (status = 200, description = "Combined core settings", body = CombinedSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("oauth2" = ["settings:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_combined_settings(
    State(state): State<Arc<AppState>>,
    CanReadSettings(_user): CanReadSettings,
) -> Response {
    let agent_certificates = AgentCertificateSettingsResponse {
        lifetime_hours: state.settings.agent_cert_lifetime_hours(),
        renewal_window_hours_override: state.settings.renewal_window_hours_override(),
        effective_renewal_window_hours: state.settings.renewal_window_hours(),
    };

    let active_count =
        match et_queries::count_active_tokens(state.db(), state.default_tenant_id).await {
            Ok(count) => count,
            Err(e) => {
                tracing::error!("Failed to count active enrollment tokens: {}", e);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(state.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {}", e);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let response = CombinedSettingsResponse::new(
        agent_certificates,
        EnrollmentTokensSummary { active_count },
        multi_tenancy_enabled,
    );

    (StatusCode::OK, Json(response)).into_response()
}
