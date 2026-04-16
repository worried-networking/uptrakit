use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::{PluginCapability, PluginTypeId};
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreatePluginConfigRequest {
    pub name: String,
    /// Plugin type identifier (e.g. `github_releases`, `proxmox_helper_scripts`).
    pub plugin_type: PluginTypeId,
    /// Plugin-specific configuration blob.
    pub config: serde_json::Value,
    /// Whether the config is enabled. Defaults to true.
    #[serde(default = "crate::default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdatePluginConfigRequest {
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginConfigResponse {
    pub id: Uuid,
    pub name: String,
    pub plugin_type: PluginTypeId,
    /// Plugin-specific configuration with secrets masked.
    pub config: serde_json::Value,
    pub enabled: bool,
    /// Capabilities declared by this plugin type, e.g. `["discover_local_software"]`.
    pub capabilities: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

/// Dynamic data source for options in `select` and `multi_select` fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SelectSource {
    /// Fetch options from a REST API endpoint.
    RestApi {
        path: String,
        value_field: String,
        label_field: String,
    },
    /// Fetch options by invoking a surface action.
    Action { action_id: String },
}

/// A single option in a `select` or `multi_select` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// Condition for conditional field visibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleWhen {
    pub field: String,
    pub values: Vec<String>,
}

/// Input field type.
///
/// # Wire forward-compatibility
///
/// `Other(String)` preserves unknown field types from newer peers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FieldType {
    /// Single-line text input.
    #[default]
    Text,
    /// Password input (masked).
    Password,
    /// Numeric input.
    Number,
    /// Dropdown select.
    Select,
    /// Checkbox list allowing multiple selections.
    MultiSelect,
    /// Multi-line text input.
    Textarea,
    /// Boolean toggle.
    Toggle,
    /// Hidden field (not displayed, included in submission).
    Hidden,
    /// SSH private key field.
    SshPrivateKey,
    /// Forward-compatible catch-all for unknown field types.
    Other(String),
}

impl FieldType {
    /// Returns the snake_case wire string for this field type.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Password => "password",
            Self::Number => "number",
            Self::Select => "select",
            Self::MultiSelect => "multi_select",
            Self::Textarea => "textarea",
            Self::Toggle => "toggle",
            Self::Hidden => "hidden",
            Self::SshPrivateKey => "ssh_private_key",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for FieldType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "text" => Self::Text,
            "password" => Self::Password,
            "number" => Self::Number,
            "select" => Self::Select,
            "multi_select" => Self::MultiSelect,
            "textarea" => Self::Textarea,
            "toggle" => Self::Toggle,
            "hidden" => Self::Hidden,
            "ssh_private_key" => Self::SshPrivateKey,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for FieldType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FieldType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(FieldType::from)
    }
}

/// Form field definition exposed by the API for plugin config and type settings forms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    /// Field key used in form submission.
    pub key: String,
    /// Human-readable field label.
    pub label: String,
    /// Field input type.
    #[serde(default)]
    pub field_type: FieldType,
    /// Whether this field is required.
    #[serde(default)]
    pub required: bool,
    /// Placeholder text for the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Help text displayed below the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
    /// Default value for the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    /// Static options for `select`/`multi_select`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    /// Dynamic source for select options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_source: Option<SelectSource>,
    /// Whether this field contains sensitive data.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// Whether this field is represented as a newline-separated list.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub list: bool,
    /// Conditional field visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<VisibleWhen>,
}

/// Static metadata for a single plugin type, returned by `GET /api/v1/plugin-types`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginTypeInfo {
    /// Wire identifier for the plugin type (e.g. `"releases_github"`).
    pub plugin_type: PluginTypeId,
    /// Human-readable display name (e.g. `"GitHub Releases"`).
    pub display_name: String,
    /// Whether this plugin type supports tenant-scoped per-instance plugin configs.
    pub supports_plugin_configs: bool,
    /// Capabilities declared by this plugin type.
    pub capabilities: Vec<PluginCapability>,
    /// A sample/default configuration JSON for this plugin type.
    ///
    /// Clients may pre-fill the config textarea with this value when creating
    /// a new plugin config, so end-users see all available fields with their
    /// defaults rather than a blank `{}`.
    pub sample_config: serde_json::Value,
    /// Form field definitions for this plugin type.
    ///
    /// When non-empty, the frontend renders a typed form instead of a raw JSON
    /// textarea. Empty for plugins with no configurable fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<serde_json::Value>))]
    pub config_form_fields: Vec<FormField>,
    /// Form field definitions for tenant-level type settings.
    ///
    /// When non-empty, the Settings page shows a per-type-settings form backed
    /// by the `plugin_type_settings` table (e.g., APT `discovery_filter`).
    /// Empty for plugins that have no type-level settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<serde_json::Value>))]
    pub type_settings_form_fields: Vec<FormField>,
    /// Sample/default JSON for type settings.
    #[serde(default, skip_serializing_if = "is_empty_object")]
    pub type_settings_sample: serde_json::Value,
}

fn is_empty_object(v: &serde_json::Value) -> bool {
    matches!(v, serde_json::Value::Object(m) if m.is_empty())
}

impl Validate for CreatePluginConfigRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "name must not be empty".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::{PluginCapability, plugin_ids};

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── CreatePluginConfigRequest ──────────────────────────────────

    #[test]
    fn create_request_round_trip() {
        let req = CreatePluginConfigRequest {
            name: "my-github".to_string(),
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            config: serde_json::json!({"tag_strip_prefix": "v"}),
            enabled: true,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: CreatePluginConfigRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name, "my-github");
        assert_eq!(de.plugin_type, plugin_ids::RELEASES_GITHUB.clone());
        assert!(de.enabled);
    }

    #[test]
    fn create_request_enabled_defaults_to_true() {
        let json = r#"{"name":"test","plugin_type":"releases_github","config":{}}"#;
        let de: CreatePluginConfigRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(de.enabled, "enabled should default to true");
    }

    #[test]
    fn create_request_validate_rejects_empty_name() {
        let req = CreatePluginConfigRequest {
            name: "   ".to_string(),
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            config: serde_json::json!({}),
            enabled: true,
        };
        let err = req
            .validate()
            .expect_err("should reject whitespace-only name");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn create_request_validate_accepts_valid() {
        let req = CreatePluginConfigRequest {
            name: "my-plugin".to_string(),
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            config: serde_json::json!({}),
            enabled: true,
        };
        assert!(req.validate().is_ok());
    }

    // ── UpdatePluginConfigRequest ──────────────────────────────────

    #[test]
    fn update_request_round_trip_all_fields() {
        let req = UpdatePluginConfigRequest {
            name: Some("renamed".to_string()),
            config: Some(serde_json::json!({"key": "val"})),
            enabled: Some(false),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdatePluginConfigRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name.as_deref(), Some("renamed"));
        assert_eq!(de.enabled, Some(false));
    }

    #[test]
    fn update_request_round_trip_none_fields() {
        let req = UpdatePluginConfigRequest {
            name: None,
            config: None,
            enabled: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdatePluginConfigRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(de.name.is_none());
        assert!(de.config.is_none());
        assert!(de.enabled.is_none());
    }

    // ── PluginTypeInfo ─────────────────────────────────────────────

    #[test]
    fn field_type_unknown_round_trip_preserves_value() {
        let parsed: FieldType = serde_json::from_str("\"future_picker\"")
            .expect("unknown field_type should deserialize");
        assert_eq!(parsed, FieldType::Other("future_picker".to_string()));
        let serialized =
            serde_json::to_string(&parsed).expect("unknown field_type should serialize");
        assert_eq!(serialized, "\"future_picker\"");
    }

    #[test]
    fn field_def_default_value_keeps_json_type() {
        let field = FormField {
            key: "complex".to_string(),
            label: "Complex".to_string(),
            field_type: FieldType::Text,
            required: false,
            placeholder: None,
            help_text: None,
            default_value: Some(serde_json::json!({
                "numbers": [1, 2],
                "toggle": true
            })),
            options: vec![],
            select_source: None,
            sensitive: false,
            list: false,
            visible_when: None,
        };
        let json = serde_json::to_value(&field).expect("serialization should succeed");
        assert_eq!(
            json.get("default_value"),
            Some(&serde_json::json!({
                "numbers": [1, 2],
                "toggle": true
            }))
        );
    }

    #[test]
    fn plugin_type_info_round_trip() {
        let info = PluginTypeInfo {
            plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
            display_name: "Docker".to_string(),
            supports_plugin_configs: true,
            capabilities: vec![
                PluginCapability::DiscoverLocalSoftware,
                PluginCapability::ControllerSideFetchReleases,
            ],
            sample_config: serde_json::json!({"tracking_mode": "semver_tags"}),
            config_form_fields: vec![],
            type_settings_form_fields: vec![],
            type_settings_sample: serde_json::json!({}),
        };
        let json = serde_json::to_string(&info).expect("serialization should succeed");
        let de: PluginTypeInfo =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.plugin_type, plugin_ids::RELEASES_DOCKER.clone());
        assert_eq!(de.display_name, "Docker");
        assert!(de.supports_plugin_configs);
        assert_eq!(
            de.capabilities,
            vec![
                PluginCapability::DiscoverLocalSoftware,
                PluginCapability::ControllerSideFetchReleases
            ]
        );
        assert_eq!(de.sample_config["tracking_mode"], "semver_tags");
    }

    #[test]
    fn plugin_type_info_capabilities_serialize_snake_case() {
        let info = PluginTypeInfo {
            plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
            display_name: "GitHub Releases".to_string(),
            supports_plugin_configs: true,
            capabilities: vec![PluginCapability::ControllerSideFetchReleases],
            sample_config: serde_json::json!({}),
            config_form_fields: vec![],
            type_settings_form_fields: vec![],
            type_settings_sample: serde_json::json!({}),
        };
        let json = serde_json::to_string(&info).expect("serialization should succeed");
        assert!(
            json.contains("controller_side_fetch_releases"),
            "capability should serialize as snake_case"
        );
    }

    // ── PluginConfigResponse ───────────────────────────────────────

    #[test]
    fn response_round_trip() {
        use time::macros::datetime;
        let resp = PluginConfigResponse {
            id: sample_uuid(),
            name: "docker-hub".to_string(),
            plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
            config: serde_json::json!({}),
            enabled: true,
            capabilities: vec!["discover_local_software".to_string()],
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: PluginConfigResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.id, sample_uuid());
        assert_eq!(de.name, "docker-hub");
        assert_eq!(de.plugin_type, plugin_ids::RELEASES_DOCKER.clone());
        assert!(de.enabled);
    }
}
