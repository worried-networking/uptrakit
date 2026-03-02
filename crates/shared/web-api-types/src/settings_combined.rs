use serde::{Deserialize, Serialize};

use crate::enrollment_tokens::EnrollmentTokensSummary;
use crate::settings::RegistrationSettingsResponse;
use crate::settings_agent_certs::AgentCertificateSettingsResponse;
use crate::settings_auth::AuthenticationSettingsResponse;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CombinedSettingsResponse {
    pub registration: RegistrationSettingsResponse,
    pub authentication: AuthenticationSettingsResponse,
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokensSummary,
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
                lifetime_days: 365,
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
        assert_eq!(de.agent_certificates.lifetime_days, 365);
        assert_eq!(de.enrollment_tokens.active_count, 2);
    }
}
