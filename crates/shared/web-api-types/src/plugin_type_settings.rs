use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::PluginType;

use crate::validation::{Validate, ValidationError};

/// Response returned by `GET /api/v1/plugin-type-settings/:plugin_type`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginTypeSettingsResponse {
    pub plugin_type: PluginType,
    /// Plugin-type-level settings blob (always a JSON object).
    pub config: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

/// Request body for `PUT /api/v1/plugin-type-settings/:plugin_type` (upsert).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpsertPluginTypeSettingsRequest {
    /// Plugin-type-level settings. Must be a JSON object.
    pub config: serde_json::Value,
}

impl Validate for UpsertPluginTypeSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if !self.config.is_object() {
            return Err(ValidationError {
                field: "config",
                message: "config must be a JSON object".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── UpsertPluginTypeSettingsRequest ────────────────────────────

    #[test]
    fn upsert_request_round_trip() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::json!({"poll_interval_secs": 300}),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpsertPluginTypeSettingsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.config["poll_interval_secs"], 300);
    }

    #[test]
    fn upsert_request_validate_rejects_null() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::Value::Null,
        };
        let err = req.validate().expect_err("should reject null config");
        assert_eq!(err.field, "config");
    }

    #[test]
    fn upsert_request_validate_rejects_array() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::json!([1, 2, 3]),
        };
        let err = req.validate().expect_err("should reject array config");
        assert_eq!(err.field, "config");
    }

    #[test]
    fn upsert_request_validate_rejects_string() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::json!("not an object"),
        };
        let err = req.validate().expect_err("should reject string config");
        assert_eq!(err.field, "config");
    }

    #[test]
    fn upsert_request_validate_accepts_empty_object() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::json!({}),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn upsert_request_validate_accepts_populated_object() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::json!({"key": "value", "nested": {"a": 1}}),
        };
        assert!(req.validate().is_ok());
    }

    // ── PluginTypeSettingsResponse ────────────────────────────────

    #[test]
    fn response_round_trip() {
        use time::macros::datetime;
        let resp = PluginTypeSettingsResponse {
            plugin_type: PluginType::ReleasesGithub,
            config: serde_json::json!({"poll_interval_secs": 300}),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: PluginTypeSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.plugin_type, PluginType::ReleasesGithub);
        assert_eq!(de.config["poll_interval_secs"], 300);
        assert_eq!(de.created_at, datetime!(2025-01-01 00:00:00 UTC));
        assert_eq!(de.updated_at, datetime!(2025-06-01 00:00:00 UTC));
    }

    #[test]
    fn response_timestamps_serialize_as_rfc3339() {
        use time::macros::datetime;
        let resp = PluginTypeSettingsResponse {
            plugin_type: PluginType::ReleasesDocker,
            config: serde_json::json!({}),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:30:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        assert!(
            json.contains("2025-01-01T00:00:00Z"),
            "created_at should be RFC 3339"
        );
        assert!(
            json.contains("2025-06-01T12:30:00Z"),
            "updated_at should be RFC 3339"
        );
    }
}
