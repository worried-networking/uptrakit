use serde::{Deserialize, Serialize};

/// The type of service (agent or MQTT).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    Agent,
    Mqtt,
}

/// Unified status for all service types.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Pending,
    Approved,
    Rejected,
    Deactivated,
}

impl ServiceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Deactivated => "deactivated",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            "deactivated" => Some(Self::Deactivated),
            _ => None,
        }
    }
}

/// Unified response for any service (agent or MQTT).
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServiceResponse {
    pub id: String,
    pub service_type: ServiceType,
    pub hostname: String,
    pub friendly_name: String,
    pub ip_address: Option<String>,
    pub status: ServiceStatus,
    pub client_version: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Query parameters for listing services.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListServicesQuery {
    /// Filter by service type: `agent` or `mqtt`.
    pub r#type: Option<String>,
    /// Filter by status: `pending`, `approved`, `rejected`, `deactivated`.
    pub status: Option<String>,
}

// Re-export generic types that are shared across service operations.
pub use super::agents::{
    EnrollmentTokenResponse, EnrollmentTokenStatusResponse, MergeAgentRequest, MessageResponse,
};
