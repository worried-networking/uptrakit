use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::notifications::deserialize_nullable_value;
use crate::notifications::event_types::NotificationEventType;
use crate::validation::{Validate, ValidationError};

// ── Request types ────────────────────────────────────────────────────────

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

// ── Response types ───────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;
    use crate::notifications::event_types::NotificationEventType;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
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
        // None scope fields are skipped in serialization (skip_serializing_if)
        assert!(
            !json.contains("host_id"),
            "None host_id must not be serialized: {json}"
        );
        let deserialized: UpdateNotificationRuleRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(
            deserialized.event_type,
            Some(NotificationEventType::CaRotated)
        );
        assert_eq!(deserialized.enabled, Some(false));
        assert!(
            deserialized.host_id.is_none(),
            "absent host_id must deserialize to None"
        );
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

    #[test]
    fn update_rule_scope_null_deserializes_to_some_null() {
        let json = r#"{"host_id": null, "software_item_id": null, "plugin_type": null}"#;
        let req: UpdateNotificationRuleRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(req.host_id, Some(serde_json::Value::Null));
        assert_eq!(req.software_item_id, Some(serde_json::Value::Null));
        assert_eq!(req.plugin_type, Some(serde_json::Value::Null));
    }

    #[test]
    fn update_rule_scope_uuid_string_deserializes_to_some_string() {
        let uuid_str = "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6";
        let json = format!(r#"{{"host_id": "{uuid_str}"}}"#);
        let req: UpdateNotificationRuleRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(
            req.host_id,
            Some(serde_json::Value::String(uuid_str.to_string()))
        );
    }

    #[test]
    fn validate_update_rule_null_scope_fields_ok() {
        let req = UpdateNotificationRuleRequest {
            event_type: None,
            host_id: Some(serde_json::Value::Null),
            software_item_id: Some(serde_json::Value::Null),
            plugin_type: Some(serde_json::Value::Null),
            enabled: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_update_rule_valid_uuid_string_ok() {
        let uuid_str = "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6".to_string();
        let req = UpdateNotificationRuleRequest {
            event_type: None,
            host_id: Some(serde_json::Value::String(uuid_str.clone())),
            software_item_id: Some(serde_json::Value::String(uuid_str)),
            plugin_type: Some(serde_json::Value::String("releases_github".to_string())),
            enabled: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_update_rule_invalid_uuid_host_id_rejected() {
        let req = UpdateNotificationRuleRequest {
            event_type: None,
            host_id: Some(serde_json::Value::String("not-a-uuid".to_string())),
            software_item_id: None,
            plugin_type: None,
            enabled: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "host_id");
    }

    #[test]
    fn validate_update_rule_invalid_uuid_software_item_id_rejected() {
        let req = UpdateNotificationRuleRequest {
            event_type: None,
            host_id: None,
            software_item_id: Some(serde_json::Value::String("not-a-uuid".to_string())),
            plugin_type: None,
            enabled: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "software_item_id");
    }

    #[test]
    fn validate_update_rule_empty_plugin_type_rejected() {
        let req = UpdateNotificationRuleRequest {
            event_type: None,
            host_id: None,
            software_item_id: None,
            plugin_type: Some(serde_json::Value::String("   ".to_string())),
            enabled: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "plugin_type");
    }

    #[test]
    fn validate_update_rule_non_string_host_id_rejected() {
        let req = UpdateNotificationRuleRequest {
            event_type: None,
            host_id: Some(serde_json::Value::Number(serde_json::Number::from(42))),
            software_item_id: None,
            plugin_type: None,
            enabled: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "host_id");
    }

    // ── NotificationRuleResponse ────────────────────────────────────────

    #[test]
    fn rule_response_round_trip() {
        use time::macros::datetime;

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
}
