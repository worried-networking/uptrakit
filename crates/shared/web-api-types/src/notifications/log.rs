use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::notifications::event_types::{NotificationDeliveryStatus, NotificationEventType};

// ── Log / delivery history response ─────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
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
        assert_eq!(deserialized.status, NotificationDeliveryStatus::Delivered);
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
        assert_eq!(deserialized.status, NotificationDeliveryStatus::Failed);
        assert_eq!(
            deserialized.error_message.as_deref(),
            Some("connection refused")
        );
        assert!(deserialized.action_token.is_none());
        assert!(deserialized.action_taken.is_none());
        assert!(deserialized.delivered_at.is_none());
    }
}
