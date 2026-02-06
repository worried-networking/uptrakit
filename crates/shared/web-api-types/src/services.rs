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
}

#[derive(Debug)]
pub struct ParseServiceStatusError;

impl std::fmt::Display for ParseServiceStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid service status value")
    }
}

impl std::error::Error for ParseServiceStatusError {}

impl std::str::FromStr for ServiceStatus {
    type Err = ParseServiceStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "deactivated" => Ok(Self::Deactivated),
            _ => Err(ParseServiceStatusError),
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
