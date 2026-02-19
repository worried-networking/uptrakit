use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Canonical types from shared-types with feature-gated OpenAPI derives.
pub use uptrakit_shared_types::{
    ParseServiceStatusError, ParseServiceTypeError, ServiceStatus, ServiceType,
};

/// Unified response for any service (agent or MQTT).
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServiceResponse {
    pub id: Uuid,
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
    /// Filter by service type: `agent`, `mqtt`, or `ssh_agent`.
    pub r#type: Option<ServiceType>,
    /// Filter by status: `pending`, `approved`, `rejected`, `deactivated`.
    pub status: Option<ServiceStatus>,
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}

impl ListServicesQuery {
    pub fn pagination(&self) -> crate::pagination::PaginationParams {
        crate::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}

// Re-export generic types that are shared across service operations.
pub use super::agents::{
    EnrollmentTokenResponse, EnrollmentTokenStatusResponse, MergeAgentRequest, MessageResponse,
};
