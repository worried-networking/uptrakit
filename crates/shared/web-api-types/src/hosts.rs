use crate::services::ServiceStatus;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use super::agents::MessageResponse as HostMessageResponse;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostResponse {
    pub id: Uuid,
    pub machine_id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub os_type: Option<String>,
    pub os_version: Option<String>,
    pub architecture: Option<String>,
    pub ip_address: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub agents: Vec<HostAgentSummary>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostAgentSummary {
    pub id: Uuid,
    pub friendly_name: String,
    pub status: ServiceStatus,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateHostRequest {
    pub friendly_name: Option<String>,
}
