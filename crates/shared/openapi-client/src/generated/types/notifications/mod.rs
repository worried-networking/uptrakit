// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
pub mod channels;
pub mod event_types;
pub mod log;
pub mod rules;
pub use channels::{
    CreateNotificationChannelRequest, NotificationChannelResponse, TestNotificationResponse,
    UpdateNotificationChannelRequest,
};
pub use event_types::{
    NotificationDeliveryStatus, NotificationEventType, ParseNotificationDeliveryStatusError,
    ParseNotificationEventTypeError,
};
pub use log::NotificationLogResponse;
pub use rules::{
    CreateNotificationRuleRequest, NotificationRuleResponse, UpdateNotificationRuleRequest,
};
/// Deserializes a field that participates in the nullable-update pattern.
///
/// - Field **absent** from JSON → serde `#[default]` kicks in → `None` (leave unchanged)
/// - Field present as JSON **`null`** → `Some(Value::Null)` (clear to NULL)
/// - Field present as any other JSON value → `Some(value)` (set to that value)
///
/// Must be paired with `#[serde(default)]` on the field so that absent fields
/// bypass this function entirely and produce `None` via `Default`.
pub(in crate::generated::types) fn deserialize_nullable_value<'de, D>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let val = serde_json::Value::deserialize(deserializer)?;
    Ok(Some(val))
}
