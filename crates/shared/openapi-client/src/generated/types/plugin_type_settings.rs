use crate::generated::shared_types::PluginTypeId;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
/// Response returned by `GET /api/v1/plugin-type-settings/:plugin_type`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginTypeSettingsResponse {
    pub plugin_type: PluginTypeId,
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
