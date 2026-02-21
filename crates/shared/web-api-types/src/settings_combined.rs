use serde::{Deserialize, Serialize};

use crate::agents::EnrollmentTokenStatusResponse;
use crate::settings::RegistrationSettingsResponse;
use crate::settings_agent_certs::AgentCertificateSettingsResponse;
use crate::settings_auth::AuthenticationSettingsResponse;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollmentTokenStatusesResponse {
    pub agent: EnrollmentTokenStatusResponse,
    pub mqtt: EnrollmentTokenStatusResponse,
    pub ssh_agent: EnrollmentTokenStatusResponse,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CombinedSettingsResponse {
    pub registration: RegistrationSettingsResponse,
    pub authentication: AuthenticationSettingsResponse,
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokenStatusesResponse,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::RegistrationMode;

    #[test]
    fn enrollment_token_statuses_response_round_trip() {
        let resp = EnrollmentTokenStatusesResponse {
            agent: EnrollmentTokenStatusResponse { configured: true },
            mqtt: EnrollmentTokenStatusResponse { configured: false },
            ssh_agent: EnrollmentTokenStatusResponse { configured: true },
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: EnrollmentTokenStatusesResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(de.agent.configured);
        assert!(!de.mqtt.configured);
        assert!(de.ssh_agent.configured);
    }

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
                renewal_window_hours: 168,
            },
            enrollment_tokens: EnrollmentTokenStatusesResponse {
                agent: EnrollmentTokenStatusResponse { configured: true },
                mqtt: EnrollmentTokenStatusResponse { configured: false },
                ssh_agent: EnrollmentTokenStatusResponse { configured: false },
            },
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: CombinedSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.registration.mode, RegistrationMode::Invite);
        assert!(de.authentication.password_auth_enabled);
        assert_eq!(de.agent_certificates.lifetime_days, 365);
        assert!(de.enrollment_tokens.agent.configured);
    }
}
