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

/// A single entry in the software ignore list.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareIgnoreResponse {
    /// Ignore rule UUID.
    pub id: Uuid,
    /// Software item display name to suppress.
    pub name: String,
    /// When set, this ignore rule applies only to the given host.
    /// `None` means the rule is tenant-wide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// Request body for creating a software ignore rule.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSoftwareIgnoreRequest {
    /// Software item display name to permanently suppress from future discoveries.
    pub name: String,
    /// Optionally scope the ignore rule to a specific host.
    /// `None` means the rule applies tenant-wide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<Uuid>,
}

impl Validate for CreateSoftwareIgnoreRequest {
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

    fn make_request(name: &str) -> CreateSoftwareIgnoreRequest {
        CreateSoftwareIgnoreRequest {
            name: name.to_string(),
            host_id: None,
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

    #[test]
    fn software_ignore_response_round_trip_tenant_wide() {
        use time::macros::datetime;
        let resp = SoftwareIgnoreResponse {
            id: Uuid::nil(),
            name: "FreshRSS".to_string(),
            host_id: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareIgnoreResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "FreshRSS");
        assert!(deserialized.host_id.is_none());
        // host_id should be omitted when None
        assert!(!json.contains("host_id"));
    }

    #[test]
    fn software_ignore_response_round_trip_per_host() {
        use time::macros::datetime;
        let host = Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").expect("valid uuid");
        let resp = SoftwareIgnoreResponse {
            id: Uuid::nil(),
            name: "FreshRSS".to_string(),
            host_id: Some(host),
            created_at: datetime!(2025-01-01 0:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareIgnoreResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.host_id, Some(host));
    }

    #[test]
    fn create_request_with_host_id() {
        let host = Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").expect("valid uuid");
        let req = CreateSoftwareIgnoreRequest {
            name: "FreshRSS".to_string(),
            host_id: Some(host),
        };
        assert!(req.validate().is_ok());
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateSoftwareIgnoreRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.host_id, Some(host));
    }
}
