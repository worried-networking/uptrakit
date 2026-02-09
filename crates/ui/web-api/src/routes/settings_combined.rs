use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::SettingKey;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::settings_store::load_setting;
use uptrakit_web_api_types::agents::EnrollmentTokenStatusResponse;
use uptrakit_web_api_types::settings::RegistrationSettingsResponse;
use uptrakit_web_api_types::settings_agent_certs::AgentCertificateSettingsResponse;
use uptrakit_web_api_types::settings_auth::AuthenticationSettingsResponse;
use uptrakit_web_api_types::settings_combined::{
    CombinedSettingsResponse, EnrollmentTokenStatusesResponse,
};

fn build_combined_settings_response(
    registration: RegistrationSettingsResponse,
    authentication: AuthenticationSettingsResponse,
    agent_certificates: AgentCertificateSettingsResponse,
    enrollment_tokens: EnrollmentTokenStatusesResponse,
) -> CombinedSettingsResponse {
    CombinedSettingsResponse {
        registration,
        authentication,
        agent_certificates,
        enrollment_tokens,
    }
}

fn has_settings_access(user: &AuthenticatedUser) -> bool {
    user.has_permission(Permission::ViewSettings) || user.has_permission(Permission::ManageSettings)
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
    security(("bearer_token" = []))
)]
pub async fn get_combined_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !has_settings_access(&user) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

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

    let agent_configured = matches!(
        load_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::EnrollmentTokenHash
        )
        .await,
        Ok(Some(_))
    );
    let mqtt_configured = matches!(
        load_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::MqttEnrollmentTokenHash
        )
        .await,
        Ok(Some(_))
    );
    let enrollment_tokens = EnrollmentTokenStatusesResponse {
        agent: EnrollmentTokenStatusResponse {
            configured: agent_configured,
        },
        mqtt: EnrollmentTokenStatusResponse {
            configured: mqtt_configured,
        },
    };

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
    use crate::auth::registration::RegistrationMode;
    use uptrakit_web_api_types::agents::EnrollmentTokenStatusResponse;
    use uptrakit_web_api_types::settings::RegistrationSettingsResponse;
    use uptrakit_web_api_types::settings_agent_certs::AgentCertificateSettingsResponse;
    use uptrakit_web_api_types::settings_auth::AuthenticationSettingsResponse;
    use uptrakit_web_api_types::settings_combined::EnrollmentTokenStatusesResponse;

    use super::build_combined_settings_response;

    #[test]
    fn build_combined_settings_response_maps_fields() {
        let registration = RegistrationSettingsResponse {
            mode: RegistrationMode::Invite,
            require_token_for_oidc: true,
        };
        let authentication = AuthenticationSettingsResponse {
            password_auth_enabled: false,
        };
        let agent_certificates = AgentCertificateSettingsResponse {
            lifetime_days: 14,
            renewal_window_hours: 9,
        };
        let enrollment_tokens = EnrollmentTokenStatusesResponse {
            agent: EnrollmentTokenStatusResponse { configured: true },
            mqtt: EnrollmentTokenStatusResponse { configured: false },
        };

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
        assert_eq!(combined.agent_certificates.renewal_window_hours, 9);
        assert!(combined.enrollment_tokens.agent.configured);
        assert!(!combined.enrollment_tokens.mqtt.configured);
    }
}
