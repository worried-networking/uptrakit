use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

// ── Enums ────────────────────────────────────────────────────────────────

/// The type of event that triggers a notification.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NotificationEventType {
    UpdateAvailable,
    UpdateCompleted,
    UpdateFailed,
    NewSoftwareDiscovered,
    NewServiceEnrolled,
    CaRotated,
}

impl NotificationEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UpdateAvailable => "update_available",
            Self::UpdateCompleted => "update_completed",
            Self::UpdateFailed => "update_failed",
            Self::NewSoftwareDiscovered => "new_software_discovered",
            Self::NewServiceEnrolled => "new_service_enrolled",
            Self::CaRotated => "ca_rotated",
        }
    }
}

impl std::fmt::Display for NotificationEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid [`NotificationEventType`] string.
#[derive(Debug, Error)]
#[error("invalid notification event type")]
pub struct ParseNotificationEventTypeError;

impl FromStr for NotificationEventType {
    type Err = ParseNotificationEventTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "update_available" => Ok(Self::UpdateAvailable),
            "update_completed" => Ok(Self::UpdateCompleted),
            "update_failed" => Ok(Self::UpdateFailed),
            "new_software_discovered" => Ok(Self::NewSoftwareDiscovered),
            "new_service_enrolled" => Ok(Self::NewServiceEnrolled),
            "ca_rotated" => Ok(Self::CaRotated),
            _ => Err(ParseNotificationEventTypeError),
        }
    }
}

/// The type of delivery channel for notifications.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannelType {
    Webhook,
    Telegram,
}

impl NotificationChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Telegram => "telegram",
        }
    }
}

impl std::fmt::Display for NotificationChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid [`NotificationChannelType`] string.
#[derive(Debug, Error)]
#[error("invalid notification channel type")]
pub struct ParseNotificationChannelTypeError;

impl FromStr for NotificationChannelType {
    type Err = ParseNotificationChannelTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "webhook" => Ok(Self::Webhook),
            "telegram" => Ok(Self::Telegram),
            _ => Err(ParseNotificationChannelTypeError),
        }
    }
}

/// The delivery status of a notification.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NotificationDeliveryStatus {
    Pending,
    Delivered,
    Failed,
}

impl NotificationDeliveryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for NotificationDeliveryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid [`NotificationDeliveryStatus`] string.
#[derive(Debug, Error)]
#[error("invalid notification delivery status")]
pub struct ParseNotificationDeliveryStatusError;

impl FromStr for NotificationDeliveryStatus {
    type Err = ParseNotificationDeliveryStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "delivered" => Ok(Self::Delivered),
            "failed" => Ok(Self::Failed),
            _ => Err(ParseNotificationDeliveryStatusError),
        }
    }
}

// ── Request types ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateNotificationChannelRequest {
    pub name: String,
    pub channel_type: NotificationChannelType,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateNotificationRuleRequest {
    pub channel_id: Uuid,
    pub event_type: NotificationEventType,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub plugin_type: Option<String>,
    #[serde(default = "crate::default_enabled")]
    pub enabled: bool,
}

impl Validate for CreateNotificationRuleRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        // channel_id and event_type are validated by their types
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateNotificationRuleRequest {
    pub event_type: Option<NotificationEventType>,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub plugin_type: Option<String>,
    pub enabled: Option<bool>,
}

impl Validate for UpdateNotificationRuleRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

// ── Response types ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationChannelResponse {
    pub id: Uuid,
    pub name: String,
    pub channel_type: NotificationChannelType,
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
pub struct NotificationRuleResponse {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub event_type: NotificationEventType,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub plugin_type: Option<String>,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationLogResponse {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub rule_id: Uuid,
    pub event_type: NotificationEventType,
    pub event_payload: serde_json::Value,
    pub status: NotificationDeliveryStatus,
    pub error_message: Option<String>,
    pub action_token: Option<Uuid>,
    pub action_taken: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub delivered_at: Option<OffsetDateTime>,
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

    const ALL_EVENT_TYPES: [NotificationEventType; 6] = [
        NotificationEventType::UpdateAvailable,
        NotificationEventType::UpdateCompleted,
        NotificationEventType::UpdateFailed,
        NotificationEventType::NewSoftwareDiscovered,
        NotificationEventType::NewServiceEnrolled,
        NotificationEventType::CaRotated,
    ];

    const ALL_CHANNEL_TYPES: [NotificationChannelType; 2] = [
        NotificationChannelType::Webhook,
        NotificationChannelType::Telegram,
    ];

    const ALL_DELIVERY_STATUSES: [NotificationDeliveryStatus; 3] = [
        NotificationDeliveryStatus::Pending,
        NotificationDeliveryStatus::Delivered,
        NotificationDeliveryStatus::Failed,
    ];

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── NotificationEventType ───────────────────────────────────────────

    #[test]
    fn event_type_serde_round_trip() {
        for event in &ALL_EVENT_TYPES {
            let json = serde_json::to_string(event).expect("serialization should succeed");
            let deserialized: NotificationEventType =
                serde_json::from_str(&json).expect("deserialization should succeed");
            assert_eq!(&deserialized, event);
        }
    }

    #[test]
    fn event_type_as_str_values() {
        assert_eq!(
            NotificationEventType::UpdateAvailable.as_str(),
            "update_available"
        );
        assert_eq!(
            NotificationEventType::UpdateCompleted.as_str(),
            "update_completed"
        );
        assert_eq!(
            NotificationEventType::UpdateFailed.as_str(),
            "update_failed"
        );
        assert_eq!(
            NotificationEventType::NewSoftwareDiscovered.as_str(),
            "new_software_discovered"
        );
        assert_eq!(
            NotificationEventType::NewServiceEnrolled.as_str(),
            "new_service_enrolled"
        );
        assert_eq!(NotificationEventType::CaRotated.as_str(), "ca_rotated");
    }

    #[test]
    fn event_type_from_str_valid() {
        assert_eq!(
            "update_available".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::UpdateAvailable)
        );
        assert_eq!(
            "update_completed".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::UpdateCompleted)
        );
        assert_eq!(
            "update_failed".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::UpdateFailed)
        );
        assert_eq!(
            "new_software_discovered"
                .parse::<NotificationEventType>()
                .ok(),
            Some(NotificationEventType::NewSoftwareDiscovered)
        );
        assert_eq!(
            "new_service_enrolled"
                .parse::<NotificationEventType>()
                .ok(),
            Some(NotificationEventType::NewServiceEnrolled)
        );
        assert_eq!(
            "ca_rotated".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::CaRotated)
        );
    }

    #[test]
    fn event_type_from_str_invalid_returns_err() {
        assert!("nonexistent".parse::<NotificationEventType>().is_err());
        assert!("".parse::<NotificationEventType>().is_err());
        assert!("UPDATE_AVAILABLE".parse::<NotificationEventType>().is_err());
    }

    #[test]
    fn event_type_display_matches_as_str() {
        for event in &ALL_EVENT_TYPES {
            assert_eq!(format!("{event}"), event.as_str());
        }
    }

    #[test]
    fn event_type_as_str_round_trips_through_from_str() {
        for event in &ALL_EVENT_TYPES {
            let s = event.as_str();
            let parsed: NotificationEventType = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, event);
        }
    }

    #[test]
    fn parse_event_type_error_display_message() {
        let err = ParseNotificationEventTypeError;
        assert_eq!(err.to_string(), "invalid notification event type");
    }

    // ── NotificationChannelType ─────────────────────────────────────────

    #[test]
    fn channel_type_serde_round_trip() {
        for ct in &ALL_CHANNEL_TYPES {
            let json = serde_json::to_string(ct).expect("serialization should succeed");
            let deserialized: NotificationChannelType =
                serde_json::from_str(&json).expect("deserialization should succeed");
            assert_eq!(&deserialized, ct);
        }
    }

    #[test]
    fn channel_type_as_str_values() {
        assert_eq!(NotificationChannelType::Webhook.as_str(), "webhook");
        assert_eq!(NotificationChannelType::Telegram.as_str(), "telegram");
    }

    #[test]
    fn channel_type_from_str_valid() {
        assert_eq!(
            "webhook".parse::<NotificationChannelType>().ok(),
            Some(NotificationChannelType::Webhook)
        );
        assert_eq!(
            "telegram".parse::<NotificationChannelType>().ok(),
            Some(NotificationChannelType::Telegram)
        );
    }

    #[test]
    fn channel_type_from_str_invalid_returns_err() {
        assert!("email".parse::<NotificationChannelType>().is_err());
        assert!("".parse::<NotificationChannelType>().is_err());
        assert!("WEBHOOK".parse::<NotificationChannelType>().is_err());
    }

    #[test]
    fn channel_type_display_matches_as_str() {
        for ct in &ALL_CHANNEL_TYPES {
            assert_eq!(format!("{ct}"), ct.as_str());
        }
    }

    #[test]
    fn channel_type_as_str_round_trips_through_from_str() {
        for ct in &ALL_CHANNEL_TYPES {
            let s = ct.as_str();
            let parsed: NotificationChannelType = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, ct);
        }
    }

    #[test]
    fn parse_channel_type_error_display_message() {
        let err = ParseNotificationChannelTypeError;
        assert_eq!(err.to_string(), "invalid notification channel type");
    }

    // ── NotificationDeliveryStatus ──────────────────────────────────────

    #[test]
    fn delivery_status_serde_round_trip() {
        for status in &ALL_DELIVERY_STATUSES {
            let json = serde_json::to_string(status).expect("serialization should succeed");
            let deserialized: NotificationDeliveryStatus =
                serde_json::from_str(&json).expect("deserialization should succeed");
            assert_eq!(&deserialized, status);
        }
    }

    #[test]
    fn delivery_status_as_str_values() {
        assert_eq!(NotificationDeliveryStatus::Pending.as_str(), "pending");
        assert_eq!(NotificationDeliveryStatus::Delivered.as_str(), "delivered");
        assert_eq!(NotificationDeliveryStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn delivery_status_from_str_valid() {
        assert_eq!(
            "pending".parse::<NotificationDeliveryStatus>().ok(),
            Some(NotificationDeliveryStatus::Pending)
        );
        assert_eq!(
            "delivered".parse::<NotificationDeliveryStatus>().ok(),
            Some(NotificationDeliveryStatus::Delivered)
        );
        assert_eq!(
            "failed".parse::<NotificationDeliveryStatus>().ok(),
            Some(NotificationDeliveryStatus::Failed)
        );
    }

    #[test]
    fn delivery_status_from_str_invalid_returns_err() {
        assert!("unknown".parse::<NotificationDeliveryStatus>().is_err());
        assert!("".parse::<NotificationDeliveryStatus>().is_err());
        assert!("PENDING".parse::<NotificationDeliveryStatus>().is_err());
    }

    #[test]
    fn delivery_status_display_matches_as_str() {
        for status in &ALL_DELIVERY_STATUSES {
            assert_eq!(format!("{status}"), status.as_str());
        }
    }

    #[test]
    fn delivery_status_as_str_round_trips_through_from_str() {
        for status in &ALL_DELIVERY_STATUSES {
            let s = status.as_str();
            let parsed: NotificationDeliveryStatus = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, status);
        }
    }

    #[test]
    fn parse_delivery_status_error_display_message() {
        let err = ParseNotificationDeliveryStatusError;
        assert_eq!(err.to_string(), "invalid notification delivery status");
    }

    // ── CreateNotificationChannelRequest ────────────────────────────────

    #[test]
    fn create_channel_request_round_trip() {
        let req = CreateNotificationChannelRequest {
            name: "My Webhook".to_string(),
            channel_type: NotificationChannelType::Webhook,
            config: serde_json::json!({"url": "https://example.com/hook"}),
            enabled: true,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateNotificationChannelRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "My Webhook");
        assert_eq!(deserialized.channel_type, NotificationChannelType::Webhook);
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
            channel_type: NotificationChannelType::Webhook,
            config: serde_json::json!({"url": "https://example.com/hook"}),
            enabled: true,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_create_channel_empty_name() {
        let req = CreateNotificationChannelRequest {
            name: "".to_string(),
            channel_type: NotificationChannelType::Webhook,
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
            channel_type: NotificationChannelType::Webhook,
            config: serde_json::json!({}),
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_create_channel_non_object_config() {
        let req = CreateNotificationChannelRequest {
            name: "Test".to_string(),
            channel_type: NotificationChannelType::Webhook,
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
            channel_type: NotificationChannelType::Webhook,
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

    // ── CreateNotificationRuleRequest ───────────────────────────────────

    #[test]
    fn create_rule_request_round_trip() {
        let req = CreateNotificationRuleRequest {
            channel_id: sample_uuid(),
            event_type: NotificationEventType::UpdateAvailable,
            host_id: None,
            software_item_id: None,
            plugin_type: None,
            enabled: true,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateNotificationRuleRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.channel_id, sample_uuid());
        assert_eq!(
            deserialized.event_type,
            NotificationEventType::UpdateAvailable
        );
        assert!(deserialized.host_id.is_none());
        assert!(deserialized.software_item_id.is_none());
        assert!(deserialized.plugin_type.is_none());
        assert!(deserialized.enabled);
    }

    #[test]
    fn create_rule_request_default_enabled() {
        let json = serde_json::json!({
            "channel_id": "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6",
            "event_type": "update_available"
        });
        let req: CreateNotificationRuleRequest =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert!(req.enabled);
    }

    #[test]
    fn validate_create_rule_valid() {
        let req = CreateNotificationRuleRequest {
            channel_id: sample_uuid(),
            event_type: NotificationEventType::UpdateFailed,
            host_id: Some(sample_uuid()),
            software_item_id: None,
            plugin_type: Some("releases_github".to_string()),
            enabled: true,
        };
        assert!(req.validate().is_ok());
    }

    // ── UpdateNotificationRuleRequest ───────────────────────────────────

    #[test]
    fn update_rule_request_round_trip() {
        let req = UpdateNotificationRuleRequest {
            event_type: Some(NotificationEventType::CaRotated),
            host_id: None,
            software_item_id: None,
            plugin_type: None,
            enabled: Some(false),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateNotificationRuleRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(
            deserialized.event_type,
            Some(NotificationEventType::CaRotated)
        );
        assert_eq!(deserialized.enabled, Some(false));
    }

    #[test]
    fn validate_update_rule_all_none() {
        let req = UpdateNotificationRuleRequest {
            event_type: None,
            host_id: None,
            software_item_id: None,
            plugin_type: None,
            enabled: None,
        };
        assert!(req.validate().is_ok());
    }

    // ── NotificationChannelResponse ────────────────────────────────────

    #[test]
    fn channel_response_round_trip() {
        let resp = NotificationChannelResponse {
            id: sample_uuid(),
            name: "My Webhook".to_string(),
            channel_type: NotificationChannelType::Webhook,
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
        assert_eq!(deserialized.channel_type, NotificationChannelType::Webhook);
        assert!(deserialized.config.is_object());
        assert!(deserialized.enabled);
    }

    #[test]
    fn channel_response_timestamps_serialize_as_rfc3339() {
        let resp = NotificationChannelResponse {
            id: sample_uuid(),
            name: "Test".to_string(),
            channel_type: NotificationChannelType::Telegram,
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

    // ── NotificationRuleResponse ────────────────────────────────────────

    #[test]
    fn rule_response_round_trip() {
        let resp = NotificationRuleResponse {
            id: sample_uuid(),
            channel_id: sample_uuid(),
            event_type: NotificationEventType::NewSoftwareDiscovered,
            host_id: None,
            software_item_id: Some(sample_uuid()),
            plugin_type: Some("releases_github".to_string()),
            enabled: true,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: NotificationRuleResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(
            deserialized.event_type,
            NotificationEventType::NewSoftwareDiscovered
        );
        assert!(deserialized.host_id.is_none());
        assert_eq!(deserialized.software_item_id, Some(sample_uuid()));
        assert_eq!(deserialized.plugin_type.as_deref(), Some("releases_github"));
        assert!(deserialized.enabled);
    }

    // ── NotificationLogResponse ─────────────────────────────────────────

    #[test]
    fn log_response_round_trip_all_fields() {
        let resp = NotificationLogResponse {
            id: sample_uuid(),
            channel_id: sample_uuid(),
            rule_id: sample_uuid(),
            event_type: NotificationEventType::UpdateCompleted,
            event_payload: serde_json::json!({"version": "1.2.3"}),
            status: NotificationDeliveryStatus::Delivered,
            error_message: None,
            action_token: Some(sample_uuid()),
            action_taken: Some("acknowledged".to_string()),
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            delivered_at: Some(datetime!(2025-01-01 0:00:01 UTC)),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: NotificationLogResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(
            deserialized.event_type,
            NotificationEventType::UpdateCompleted
        );
        assert_eq!(
            deserialized.status,
            NotificationDeliveryStatus::Delivered
        );
        assert!(deserialized.error_message.is_none());
        assert_eq!(deserialized.action_token, Some(sample_uuid()));
        assert_eq!(deserialized.action_taken.as_deref(), Some("acknowledged"));
        assert!(deserialized.delivered_at.is_some());
    }

    #[test]
    fn log_response_round_trip_none_fields() {
        let resp = NotificationLogResponse {
            id: sample_uuid(),
            channel_id: sample_uuid(),
            rule_id: sample_uuid(),
            event_type: NotificationEventType::UpdateFailed,
            event_payload: serde_json::json!({}),
            status: NotificationDeliveryStatus::Failed,
            error_message: Some("connection refused".to_string()),
            action_token: None,
            action_taken: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            delivered_at: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: NotificationLogResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(
            deserialized.status,
            NotificationDeliveryStatus::Failed
        );
        assert_eq!(
            deserialized.error_message.as_deref(),
            Some("connection refused")
        );
        assert!(deserialized.action_token.is_none());
        assert!(deserialized.action_taken.is_none());
        assert!(deserialized.delivered_at.is_none());
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
