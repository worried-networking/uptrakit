use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanViewSettings;
use crate::queries::enrollment_tokens as et_queries;
use uptrakit_web_api_types::enrollment_tokens::EnrollmentTokensSummary;
use uptrakit_web_api_types::settings::RegistrationSettingsResponse;
use uptrakit_web_api_types::settings_agent_certs::AgentCertificateSettingsResponse;
use uptrakit_web_api_types::settings_auth::AuthenticationSettingsResponse;
use uptrakit_web_api_types::settings_combined::CombinedSettingsResponse;

fn build_combined_settings_response(
    registration: RegistrationSettingsResponse,
    authentication: AuthenticationSettingsResponse,
    agent_certificates: AgentCertificateSettingsResponse,
    enrollment_tokens: EnrollmentTokensSummary,
) -> CombinedSettingsResponse {
    CombinedSettingsResponse {
        registration,
        authentication,
        agent_certificates,
        enrollment_tokens,
    }
}

/// Get core settings for the settings page (OIDC and MQTT remain separate).
#[utoipa::path(
    get,
    path = "/api/v1/settings",
    responses(
        (status = 200, description = "Combined core settings", body = CombinedSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
pub async fn get_combined_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let reg = state.settings.registration();
    let registration = RegistrationSettingsResponse {
        mode: reg.mode,
        require_token_for_oidc: reg.require_token_for_oidc,
    };

    let auth_settings = state.settings.authentication();
    let authentication = AuthenticationSettingsResponse {
        password_auth_enabled: auth_settings.password_auth_enabled,
    };

    let agent_certificates = AgentCertificateSettingsResponse {
        lifetime_days: state.settings.agent_cert_lifetime_days(),
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
    let enrollment_tokens = EnrollmentTokensSummary { active_count };

    let response = build_combined_settings_response(
        registration,
        authentication,
        agent_certificates,
        enrollment_tokens,
    );

    (StatusCode::OK, Json(response)).into_response()
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::enrollment_tokens::EnrollmentTokensSummary;
    use uptrakit_web_api_types::settings::RegistrationSettingsResponse;
    use uptrakit_web_api_types::settings_agent_certs::AgentCertificateSettingsResponse;

    use crate::auth::registration::RegistrationMode;

    use super::build_combined_settings_response;

    #[test]
    fn build_combined_settings_response_maps_fields() {
        let registration = RegistrationSettingsResponse {
            mode: RegistrationMode::Invite,
            require_token_for_oidc: true,
        };
        let authentication =
            uptrakit_web_api_types::settings_auth::AuthenticationSettingsResponse {
                password_auth_enabled: false,
            };
        let agent_certificates = AgentCertificateSettingsResponse {
            lifetime_days: 14,
            renewal_window_hours_override: None,
            // 14 days × 24 / 5 = 67 h, but ceiling is 14 days = 336 h → 67 h
            effective_renewal_window_hours: 67,
        };
        let enrollment_tokens = EnrollmentTokensSummary { active_count: 3 };

        let combined = build_combined_settings_response(
            registration,
            authentication,
            agent_certificates,
            enrollment_tokens,
        );

        assert_eq!(combined.registration.mode, RegistrationMode::Invite);
        assert!(combined.registration.require_token_for_oidc);
        assert!(!combined.authentication.password_auth_enabled);
        assert_eq!(combined.agent_certificates.lifetime_days, 14);
        assert_eq!(combined.agent_certificates.renewal_window_hours_override, None);
        assert_eq!(combined.agent_certificates.effective_renewal_window_hours, 67);
        assert_eq!(combined.enrollment_tokens.active_count, 3);
    }
}
