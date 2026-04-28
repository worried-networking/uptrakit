// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value", into = "serde_json::Value")]
pub struct JsonObjectMap(serde_json::Map<String, serde_json::Value>);
impl TryFrom<serde_json::Value> for JsonObjectMap {
    type Error = ValidationError;
    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Object(map) => Ok(Self(map)),
            _ => Err(ValidationError {
                field: "config",
                message: "must be a JSON object".to_string(),
            }),
        }
    }
}
impl JsonObjectMap {
    pub fn is_object(&self) -> bool {
        true
    }
    pub fn as_object(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
}
impl From<JsonObjectMap> for serde_json::Value {
    fn from(value: JsonObjectMap) -> Self {
        serde_json::Value::Object(value.0)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonObjectInput(serde_json::Value);
impl JsonObjectInput {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.0.is_object() {
            Ok(())
        } else {
            Err(ValidationError {
                field: "config",
                message: "must be a JSON object".to_string(),
            })
        }
    }
    pub fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
    pub fn to_object_map(&self) -> Result<JsonObjectMap, ValidationError> {
        JsonObjectMap::try_from(self.0.clone())
    }
}
impl From<JsonObjectMap> for JsonObjectInput {
    fn from(value: JsonObjectMap) -> Self {
        Self(value.into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateNotificationChannelRequest {
    pub name: String,
    pub channel_type: String,
    pub config: JsonObjectInput,
    #[serde(default = "crate::generated::types::default_enabled")]
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
        self.config.validate()?;
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateNotificationChannelRequest {
    pub name: Option<String>,
    pub config: Option<JsonObjectInput>,
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
        if let Some(config) = &self.config {
            config.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationChannelResponse {
    pub id: Uuid,
    pub name: String,
    pub channel_type: String,
    pub config: JsonObjectMap,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestNotificationResponse {
    pub success: bool,
    pub message: String,
}
