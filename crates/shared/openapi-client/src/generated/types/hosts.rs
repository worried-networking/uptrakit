pub use super::agents::MessageResponse as HostMessageResponse;
use crate::generated::types::host_tags::HostTagSummary;
use crate::generated::types::services::ServiceStatus;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
#[derive(Debug, Serialize, Deserialize)]
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
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub last_seen_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
    pub agents: Vec<HostAgentSummary>,
    /// Tags assigned to this host.
    #[serde(default)]
    pub tags: Vec<HostTagSummary>,
    /// Agent-reported host features. Empty if not reported (legacy agent).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// Aggregate software status for host-list rendering.
    #[serde(default)]
    pub software_status: HostSoftwareStatusSummary,
}
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostSoftwareStatusSummary {
    pub known: bool,
    pub update_count: u32,
    pub error_count: u32,
}
#[derive(Debug, Serialize, Deserialize)]
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
