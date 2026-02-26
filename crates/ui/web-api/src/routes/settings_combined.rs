use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::SettingKey;
use crate::middleware::permission::CanViewSettings;
use crate::settings_store::load_setting;
use uptrakit_web_api_types::agents::EnrollmentTokenStatusResponse;
use uptrakit_web_api_types::settings::RegistrationSettingsResponse;
use uptrakit_web_api_types::settings_agent_certs::AgentCertificateSettingsResponse;
use uptrakit_web_api_types::settings_auth::AuthenticationSettingsResponse;
use uptrakit_web_api_types::settings_combined::CombinedSettingsResponse;

fn build_combined_settings_response(
    registration: RegistrationSettingsResponse,
    authentication: AuthenticationSettingsResponse,
    agent_certificates: AgentCertificateSettingsResponse,
    enrollment_token: EnrollmentTokenStatusResponse,
) -> CombinedSettingsResponse {
    CombinedSettingsResponse {
        registration,
        authentication,
        agent_certificates,
        enrollment_token,
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
        renewal_window_hours: state.settings.renewal_window_hours(),
    };

    let configured = matches!(
        load_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::EnrollmentTokenHash
        )
        .await,
        Ok(Some(_))
    );
    let enrollment_token = EnrollmentTokenStatusResponse { configured };

    let response = build_combined_settings_response(
        registration,
        authentication,
        agent_certificates,
        enrollment_token,
    );

    (StatusCode::OK, Json(response)).into_response()
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::agents::EnrollmentTokenStatusResponse;
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
        let authentication = uptrakit_web_api_types::settings_auth::AuthenticationSettingsResponse {
            password_auth_enabled: false,
        };
        let agent_certificates = AgentCertificateSettingsResponse {
            lifetime_days: 14,
            renewal_window_hours: 9,
        };
        let enrollment_token = EnrollmentTokenStatusResponse { configured: true };

        let combined = build_combined_settings_response(
            registration,
            authentication,
            agent_certificates,
            enrollment_token,
        );

        assert_eq!(combined.registration.mode, RegistrationMode::Invite);
        assert!(combined.registration.require_token_for_oidc);
        assert!(!combined.authentication.password_auth_enabled);
        assert_eq!(combined.agent_certificates.lifetime_days, 14);
        assert_eq!(combined.agent_certificates.renewal_window_hours, 9);
        assert!(combined.enrollment_token.configured);
    }
}
