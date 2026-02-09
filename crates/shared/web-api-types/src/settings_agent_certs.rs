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
