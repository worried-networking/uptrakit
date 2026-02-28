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
    /// Plugin config UUID this rule applies to.
    pub plugin_config_id: Uuid,
    /// Display name of the referenced plugin config.
    pub plugin_config_name: String,
    /// Plugin type string (e.g. `"homebrew"`).
    pub plugin_type: String,
    /// Package identifier to suppress.
    pub package_identifier: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// Request body for creating an autodiscovery ignore rule.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateAutodiscoveryIgnoreRequest {
    /// Plugin config UUID the rule applies to.
    pub plugin_config_id: Uuid,
    /// Package identifier to permanently suppress from future discoveries.
    pub package_identifier: String,
}

impl Validate for CreateAutodiscoveryIgnoreRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.package_identifier.trim().is_empty() {
            return Err(ValidationError {
                field: "package_identifier",
                message: "package_identifier must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(package_identifier: &str) -> CreateAutodiscoveryIgnoreRequest {
        CreateAutodiscoveryIgnoreRequest {
            plugin_config_id: Uuid::nil(),
            package_identifier: package_identifier.to_string(),
        }
    }

    #[test]
    fn validate_accepts_non_empty_identifier() {
        assert!(make_request("com.example.package").validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_identifier() {
        let err = make_request("").validate().unwrap_err();
        assert_eq!(err.field, "package_identifier");
    }

    #[test]
    fn validate_rejects_whitespace_only_identifier() {
        assert!(make_request("   ").validate().is_err());
    }
}
