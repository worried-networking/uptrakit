//! Types for the `POST /api/v1/settings/reset-data` endpoint.
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
/// Request body for the reset-data endpoint.
///
/// The caller must send `confirm: "RESET"` to acknowledge the destructive
/// operation. Any other value is rejected with a validation error.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResetDataRequest {
    /// Confirmation string — must be exactly `"RESET"`.
    pub confirm: String,
}
impl Validate for ResetDataRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.confirm != "RESET" {
            return Err(ValidationError {
                field: "confirm",
                message: "confirm must be exactly \"RESET\"".to_string(),
            });
        }
        Ok(())
    }
}
/// Response body from a successful data reset, reporting how many rows
/// were deleted from each table category.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResetDataResponse {
    /// Per-category deletion counts.
    pub deleted: ResetDeletedCounts,
}
/// Per-category counts of rows deleted during data reset.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResetDeletedCounts {
    /// Number of host rows deleted.
    pub hosts: u64,
    /// Number of software item rows deleted.
    pub software_items: u64,
    /// Number of plugin config rows deleted.
    pub plugin_configs: u64,
    /// Number of host tag rows deleted.
    pub host_tags: u64,
    /// Number of update history rows deleted.
    pub update_history: u64,
    /// Number of update batch rows deleted.
    pub update_batches: u64,
}
