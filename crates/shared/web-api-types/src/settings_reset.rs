//! Types for the `POST /api/v1/settings/reset-data` endpoint.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_reset() {
        let req = ResetDataRequest {
            confirm: "RESET".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_rejects_wrong_confirm() {
        let req = ResetDataRequest {
            confirm: "reset".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "confirm");
    }

    #[test]
    fn validate_rejects_empty() {
        let req = ResetDataRequest {
            confirm: String::new(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn response_serde_round_trip() {
        let resp = ResetDataResponse {
            deleted: ResetDeletedCounts {
                hosts: 5,
                software_items: 10,
                plugin_configs: 3,
                host_tags: 2,
                update_history: 100,
                update_batches: 4,
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: ResetDataResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.deleted.hosts, 5);
        assert_eq!(deserialized.deleted.update_history, 100);
    }
}
