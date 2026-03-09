use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Response for trigger-discovery endpoints.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerDiscoveryResponse {
    /// Number of plugin assignments queued for discovery.
    pub plugins_queued: u32,
    /// Human-readable summary message.
    pub message: String,
}

/// Response for bulk-discard (delete all pending discovered items) endpoints.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DiscardDiscoveredResponse {
    /// Number of pending items soft-deleted.
    pub discarded_count: u32,
}

/// A single entry in the autodiscovery ignore list.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AutodiscoveryIgnoreResponse {
    /// Ignore rule UUID.
    pub id: Uuid,
    /// Software item display name to suppress.
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// Request body for creating an autodiscovery ignore rule.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateAutodiscoveryIgnoreRequest {
    /// Software item display name to permanently suppress from future discoveries.
    pub name: String,
}

impl Validate for CreateAutodiscoveryIgnoreRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "name must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(name: &str) -> CreateAutodiscoveryIgnoreRequest {
        CreateAutodiscoveryIgnoreRequest {
            name: name.to_string(),
        }
    }

    #[test]
    fn validate_accepts_non_empty_name() {
        assert!(make_request("FreshRSS").validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let err = make_request("").validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_rejects_whitespace_only_name() {
        assert!(make_request("   ").validate().is_err());
    }
}
