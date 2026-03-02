use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentCertificateSettingsResponse {
    /// Certificate lifetime in days (max 730).
    pub lifetime_days: u16,
    /// Admin-configured renewal window override in hours.
    ///
    /// `null` means automatic mode: the window is `min(14 days, lifetime / 5)`.
    pub renewal_window_hours_override: Option<u16>,
    /// Effective renewal window in hours.
    ///
    /// In automatic mode this equals `min(14 days, lifetime_days × 24 / 5)`.
    /// When an override is set this equals `renewal_window_hours_override`.
    pub effective_renewal_window_hours: u16,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAgentCertificateSettingsRequest {
    /// Certificate lifetime in days (max 730).
    pub lifetime_days: Option<u16>,
    /// Renewal window override in hours.
    ///
    /// Set to `0` to reset to automatic mode (`min(14 days, lifetime / 5)`).
    /// Omit to leave the current value unchanged.
    pub renewal_window_hours: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_cert_settings_response_round_trip_auto_mode() {
        let resp = AgentCertificateSettingsResponse {
            lifetime_days: 365,
            renewal_window_hours_override: None,
            effective_renewal_window_hours: 336,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: AgentCertificateSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.lifetime_days, 365);
        assert_eq!(de.renewal_window_hours_override, None);
        assert_eq!(de.effective_renewal_window_hours, 336);
    }

    #[test]
    fn agent_cert_settings_response_round_trip_override() {
        let resp = AgentCertificateSettingsResponse {
            lifetime_days: 365,
            renewal_window_hours_override: Some(72),
            effective_renewal_window_hours: 72,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: AgentCertificateSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.lifetime_days, 365);
        assert_eq!(de.renewal_window_hours_override, Some(72));
        assert_eq!(de.effective_renewal_window_hours, 72);
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

    #[test]
    fn update_agent_cert_settings_request_reset_sentinel() {
        // renewal_window_hours = Some(0) means reset to automatic mode
        let req = UpdateAgentCertificateSettingsRequest {
            lifetime_days: None,
            renewal_window_hours: Some(0),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateAgentCertificateSettingsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.renewal_window_hours, Some(0));
    }
}
