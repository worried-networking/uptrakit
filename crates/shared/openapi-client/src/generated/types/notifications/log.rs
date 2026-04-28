// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::notifications::event_types::{
    NotificationDeliveryStatus, NotificationEventType,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub delivered_at: Option<OffsetDateTime>,
}
