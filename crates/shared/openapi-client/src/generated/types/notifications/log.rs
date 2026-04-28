use crate::generated::types::notifications::event_types::{
    NotificationDeliveryStatus, NotificationEventType,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
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
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub delivered_at: Option<OffsetDateTime>,
}
