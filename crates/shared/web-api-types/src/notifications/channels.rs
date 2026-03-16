use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

// ── Request types ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateNotificationChannelRequest {
    pub name: String,
    pub channel_type: String,
    pub config: serde_json::Value,
    #[serde(default = "crate::default_enabled")]
    pub enabled: bool,
}

impl Validate for CreateNotificationChannelRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "must not be empty".to_string(),
            });
        }
        if self.channel_type.trim().is_empty() {
            return Err(ValidationError {
                field: "channel_type",
                message: "must not be empty".to_string(),
            });
        }
        if !self.config.is_object() {
            return Err(ValidationError {
                field: "config",
                message: "must be a JSON object".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateNotificationChannelRequest {
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

impl Validate for UpdateNotificationChannelRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(name) = &self.name
            && name.trim().is_empty()
        {
            return Err(ValidationError {
                field: "name",
                message: "must not be empty".to_string(),
            });
        }
        if let Some(config) = &self.config
            && !config.is_object()
        {
            return Err(ValidationError {
                field: "config",
                message: "must be a JSON object".to_string(),
            });
        }
        Ok(())
    }
}

// ── Response types ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationChannelResponse {
    pub id: Uuid,
    pub name: String,
    pub channel_type: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TestNotificationResponse {
    pub success: bool,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── CreateNotificationChannelRequest ────────────────────────────────

    #[test]
    fn create_channel_request_round_trip() {
        let req = CreateNotificationChannelRequest {
            name: "My Webhook".to_string(),
            channel_type: "webhook".to_string(),
            config: serde_json::json!({"url": "https://example.com/hook"}),
            enabled: true,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateNotificationChannelRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "My Webhook");
        assert_eq!(deserialized.channel_type, "webhook");
        assert!(deserialized.config.is_object());
        assert!(deserialized.enabled);
    }

    #[test]
    fn create_channel_request_default_enabled() {
        let json = serde_json::json!({
            "name": "Test",
            "channel_type": "webhook",
            "config": {}
        });
        let req: CreateNotificationChannelRequest =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert!(req.enabled);
    }

    #[test]
    fn create_channel_request_explicit_enabled_false() {
        let json = serde_json::json!({
            "name": "Test",
            "channel_type": "webhook",
            "config": {},
            "enabled": false
        });
        let req: CreateNotificationChannelRequest =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert!(!req.enabled);
    }

    #[test]
    fn validate_create_channel_valid() {
        let req = CreateNotificationChannelRequest {
            name: "My Webhook".to_string(),
            channel_type: "webhook".to_string(),
            config: serde_json::json!({"url": "https://example.com/hook"}),
            enabled: true,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_create_channel_empty_name() {
        let req = CreateNotificationChannelRequest {
            name: "".to_string(),
            channel_type: "webhook".to_string(),
            config: serde_json::json!({}),
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_create_channel_whitespace_name() {
        let req = CreateNotificationChannelRequest {
            name: "   ".to_string(),
            channel_type: "webhook".to_string(),
            config: serde_json::json!({}),
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_create_channel_empty_channel_type() {
        let req = CreateNotificationChannelRequest {
            name: "Test".to_string(),
            channel_type: "".to_string(),
            config: serde_json::json!({}),
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "channel_type");
    }

    #[test]
    fn validate_create_channel_non_object_config() {
        let req = CreateNotificationChannelRequest {
            name: "Test".to_string(),
            channel_type: "webhook".to_string(),
            config: serde_json::json!("not an object"),
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "config");
    }

    #[test]
    fn validate_create_channel_array_config() {
        let req = CreateNotificationChannelRequest {
            name: "Test".to_string(),
            channel_type: "webhook".to_string(),
            config: serde_json::json!([1, 2, 3]),
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "config");
    }

    // ── UpdateNotificationChannelRequest ────────────────────────────────

    #[test]
    fn validate_update_channel_all_none() {
        let req = UpdateNotificationChannelRequest {
            name: None,
            config: None,
            enabled: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_update_channel_empty_name() {
        let req = UpdateNotificationChannelRequest {
            name: Some("".to_string()),
            config: None,
            enabled: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_update_channel_non_object_config() {
        let req = UpdateNotificationChannelRequest {
            name: None,
            config: Some(serde_json::json!(42)),
            enabled: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "config");
    }

    // ── NotificationChannelResponse ────────────────────────────────────

    #[test]
    fn channel_response_round_trip() {
        let resp = NotificationChannelResponse {
            id: sample_uuid(),
            name: "My Webhook".to_string(),
            channel_type: "webhook".to_string(),
            config: serde_json::json!({"url": "https://example.com/hook"}),
            enabled: true,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: NotificationChannelResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.name, "My Webhook");
        assert_eq!(deserialized.channel_type, "webhook");
        assert!(deserialized.config.is_object());
        assert!(deserialized.enabled);
    }

    #[test]
    fn channel_response_timestamps_serialize_as_rfc3339() {
        let resp = NotificationChannelResponse {
            id: sample_uuid(),
            name: "Test".to_string(),
            channel_type: "telegram".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:00:00 UTC),
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        assert_eq!(
            json_value.get("created_at").and_then(|v| v.as_str()),
            Some("2025-01-01T00:00:00Z")
        );
        assert_eq!(
            json_value.get("updated_at").and_then(|v| v.as_str()),
            Some("2025-06-01T12:00:00Z")
        );
    }

    // ── TestNotificationResponse ────────────────────────────────────────

    #[test]
    fn test_notification_response_round_trip() {
        let resp = TestNotificationResponse {
            success: true,
            message: "Notification delivered successfully".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TestNotificationResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.success);
        assert_eq!(deserialized.message, "Notification delivered successfully");
    }

    #[test]
    fn test_notification_response_failure() {
        let resp = TestNotificationResponse {
            success: false,
            message: "Connection refused".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TestNotificationResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(!deserialized.success);
        assert_eq!(deserialized.message, "Connection refused");
    }
}
