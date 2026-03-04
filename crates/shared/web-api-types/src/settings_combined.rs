use serde::{Deserialize, Serialize};

use crate::enrollment_tokens::EnrollmentTokensSummary;
use crate::settings::RegistrationSettingsResponse;
use crate::settings_agent_certs::AgentCertificateSettingsResponse;
use crate::settings_auth::AuthenticationSettingsResponse;
use crate::settings_mqtt::MqttLimitResponse;
use crate::settings_nats::NatsSettingsResponse;
use crate::settings_network::NetworkSettingsResponse;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CombinedSettingsResponse {
    pub registration: RegistrationSettingsResponse,
    pub authentication: AuthenticationSettingsResponse,
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokensSummary,
}

/// Combined response for all global (infrastructure-scoped) settings.
///
/// Returned by `GET /api/v1/global-settings`. This is the single call a
/// global-settings UI page needs to populate all its sections. The `nats`
/// field is omitted from serialized output when it is `None` (i.e. when the
/// controller is compiled without NATS support).
///
/// System service enrollment tokens are managed via the dedicated
/// `/api/v1/system-enrollment-tokens` endpoints.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GlobalSettingsCombinedResponse {
    pub network: NetworkSettingsResponse,
    pub mqtt_limit: MqttLimitResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nats: Option<NatsSettingsResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::RegistrationMode;

    #[test]
    fn combined_settings_response_round_trip() {
        let resp = CombinedSettingsResponse {
            registration: RegistrationSettingsResponse {
                mode: RegistrationMode::Invite,
                require_token_for_oidc: false,
            },
            authentication: AuthenticationSettingsResponse {
                password_auth_enabled: true,
            },
            agent_certificates: AgentCertificateSettingsResponse {
                lifetime_hours: 8_760,
                renewal_window_hours_override: None,
                effective_renewal_window_hours: 336,
            },
            enrollment_tokens: EnrollmentTokensSummary { active_count: 2 },
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: CombinedSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.registration.mode, RegistrationMode::Invite);
        assert!(de.authentication.password_auth_enabled);
        assert_eq!(de.agent_certificates.lifetime_hours, 8_760);
        assert_eq!(de.enrollment_tokens.active_count, 2);
    }
}
