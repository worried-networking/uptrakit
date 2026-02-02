use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentCertificateSettingsResponse {
    pub lifetime_days: u16,
    pub renewal_window_hours: u16,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAgentCertificateSettingsRequest {
    pub lifetime_days: Option<u16>,
    pub renewal_window_hours: Option<u16>,
}
