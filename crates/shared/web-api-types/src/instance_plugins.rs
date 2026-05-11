//! DTOs for `/api/v1/instance-plugins`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::PluginTypeId;

use crate::plugin_configs::FormField;
use crate::validation::{Validate, ValidationError};

/// One row in the Instance Plugins admin section. Returned only to users
/// holding `ManageGlobalSettings`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InstancePluginSummary {
    pub plugin_type: PluginTypeId,
    pub display_name: String,
    pub enabled: bool,
    pub running_enabled: bool,
    pub has_instance_config: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<serde_json::Value>))]
    pub instance_config_form_fields: Vec<FormField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<serde_json::Value>))]
    pub type_settings_form_fields: Vec<FormField>,
    pub current_config: serde_json::Value,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub updated_at: Option<OffsetDateTime>,
}

/// Detailed view (currently identical to summary; reserved for future fields
/// such as last-toggled-by, audit trail, etc.).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InstancePluginDetail {
    #[serde(flatten)]
    pub summary: InstancePluginSummary,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SetInstancePluginEnabledRequest {
    pub enabled: bool,
}

impl Validate for SetInstancePluginEnabledRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpsertInstancePluginConfigRequest {
    pub config: serde_json::Value,
}

impl Validate for UpsertInstancePluginConfigRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if !self.config.is_object() {
            return Err(ValidationError {
                field: "config",
                message: "must be a JSON object".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]

    use super::*;

    #[test]
    fn upsert_config_rejects_non_object() {
        let req = UpsertInstancePluginConfigRequest {
            config: serde_json::json!(null),
        };
        assert!(req.validate().is_err());
        let req = UpsertInstancePluginConfigRequest {
            config: serde_json::json!([]),
        };
        assert!(req.validate().is_err());
        let req = UpsertInstancePluginConfigRequest {
            config: serde_json::json!("foo"),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn upsert_config_accepts_empty_object() {
        let req = UpsertInstancePluginConfigRequest {
            config: serde_json::json!({}),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn set_enabled_always_valid() {
        assert!(
            SetInstancePluginEnabledRequest { enabled: true }
                .validate()
                .is_ok()
        );
        assert!(
            SetInstancePluginEnabledRequest { enabled: false }
                .validate()
                .is_ok()
        );
    }
}
