pub use crate::generated::shared_types::{ParseServiceStatusError, ServiceStatus};
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
/// Unified response for a tenant-agnostic system service (MQTT bridge, scheduler).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemServiceResponse {
    pub id: Uuid,
    pub capabilities: Vec<String>,
    pub hostname: String,
    pub friendly_name: String,
    pub is_embedded: bool,
    pub ip_address: Option<String>,
    pub status: ServiceStatus,
    pub client_version: Option<String>,
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
    /// Custom ping interval override in seconds. `None` means the global
    /// default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_interval_seconds: Option<u32>,
    /// Per-service certificate lifetime override in hours. `None` means the
    /// global default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_lifetime_hours: Option<u32>,
    /// External service IDs currently causing this embedded service to yield.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yielded_to: Option<Vec<Uuid>>,
}
/// Query parameters for listing system services.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListSystemServicesQuery {
    /// Filter by capability.
    pub capability: Option<String>,
    /// Filter by status: `pending`, `approved`, `rejected`, `deactivated`.
    pub status: Option<ServiceStatus>,
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}
impl ListSystemServicesQuery {
    pub fn pagination(&self) -> crate::generated::types::pagination::PaginationParams {
        crate::generated::types::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}
/// Request to update a system service's configurable settings.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateSystemServiceRequest {
    /// Custom ping interval in seconds.
    /// Omit to keep current value. Set to `0` to clear the override and
    /// revert to the global default. Set to a positive value to override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_interval_seconds: Option<u32>,
    /// Per-service certificate lifetime in hours.
    /// Omit to keep current value. Set to `0` to clear the override and revert
    /// to the global default. Set to a positive value (1–17520) to override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_lifetime_hours: Option<u32>,
}
impl Validate for UpdateSystemServiceRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(interval) = self.ping_interval_seconds {
            if interval != 0 && interval < 5 {
                return Err(ValidationError {
                    field: "ping_interval_seconds",
                    message: "ping_interval_seconds must be 0 (to clear) or at least 5".to_string(),
                });
            }
        }
        if let Some(hours) = self.cert_lifetime_hours
            && hours != 0
            && !(1..=17_520u32).contains(&hours)
        {
            return Err(ValidationError {
                field: "cert_lifetime_hours",
                message: "cert_lifetime_hours must be 0 (to clear) or between 1 and 17520"
                    .to_string(),
            });
        }
        Ok(())
    }
}
