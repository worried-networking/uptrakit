// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::notifications::deserialize_nullable_value;
use crate::generated::types::notifications::event_types::NotificationEventType;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateNotificationRuleRequest {
    pub channel_id: Uuid,
    pub event_type: NotificationEventType,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub plugin_type: Option<String>,
    #[serde(default = "crate::generated::types::default_enabled")]
    pub enabled: bool,
}
impl Validate for CreateNotificationRuleRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateNotificationRuleRequest {
    pub event_type: Option<NotificationEventType>,
    /// Scope filter: absent = keep current value, `null` = clear, `"uuid"` = set.
    #[serde(
        default,
        deserialize_with = "deserialize_nullable_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub host_id: Option<serde_json::Value>,
    /// Scope filter: absent = keep current value, `null` = clear, `"uuid"` = set.
    #[serde(
        default,
        deserialize_with = "deserialize_nullable_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub software_item_id: Option<serde_json::Value>,
    /// Scope filter: absent = keep current value, `null` = clear, `"string"` = set.
    #[serde(
        default,
        deserialize_with = "deserialize_nullable_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub plugin_type: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}
impl Validate for UpdateNotificationRuleRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(val) = &self.host_id {
            match val {
                serde_json::Value::Null => {}
                serde_json::Value::String(s) => {
                    if Uuid::parse_str(s).is_err() {
                        return Err(ValidationError {
                            field: "host_id",
                            message: "must be a valid UUID string or null".to_string(),
                        });
                    }
                }
                _ => {
                    return Err(ValidationError {
                        field: "host_id",
                        message: "must be a UUID string or null".to_string(),
                    });
                }
            }
        }
        if let Some(val) = &self.software_item_id {
            match val {
                serde_json::Value::Null => {}
                serde_json::Value::String(s) => {
                    if Uuid::parse_str(s).is_err() {
                        return Err(ValidationError {
                            field: "software_item_id",
                            message: "must be a valid UUID string or null".to_string(),
                        });
                    }
                }
                _ => {
                    return Err(ValidationError {
                        field: "software_item_id",
                        message: "must be a UUID string or null".to_string(),
                    });
                }
            }
        }
        if let Some(val) = &self.plugin_type {
            match val {
                serde_json::Value::Null => {}
                serde_json::Value::String(s) => {
                    if s.trim().is_empty() {
                        return Err(ValidationError {
                            field: "plugin_type",
                            message: "must not be empty".to_string(),
                        });
                    }
                }
                _ => {
                    return Err(ValidationError {
                        field: "plugin_type",
                        message: "must be a string or null".to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationRuleResponse {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub event_type: NotificationEventType,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub plugin_type: Option<String>,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}
