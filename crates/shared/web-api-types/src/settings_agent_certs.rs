use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentCertificateSettingsResponse {
    /// Certificate lifetime in days (max 730).
    pub lifetime_days: u16,
    pub renewal_window_hours: u16,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAgentCertificateSettingsRequest {
    /// Certificate lifetime in days (max 730).
    pub lifetime_days: Option<u16>,
    pub renewal_window_hours: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_cert_settings_response_round_trip() {
        let resp = AgentCertificateSettingsResponse {
            lifetime_days: 365,
            renewal_window_hours: 168,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: AgentCertificateSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.lifetime_days, 365);
        assert_eq!(de.renewal_window_hours, 168);
    }

    #[test]
    fn update_agent_cert_settings_request_round_trip() {
        let req = UpdateAgentCertificateSettingsRequest {
            lifetime_days: Some(730),
            renewal_window_hours: Some(336),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateAgentCertificateSettingsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.lifetime_days, Some(730));
        assert_eq!(de.renewal_window_hours, Some(336));
    }
}
