// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::{PluginCapability, PluginTypeId};
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePluginConfigRequest {
    pub name: String,
    /// Plugin type identifier (e.g. `github_releases`, `proxmox_helper_scripts`).
    pub plugin_type: PluginTypeId,
    /// Plugin-specific configuration blob.
    pub config: serde_json::Value,
    /// Whether the config is enabled. Defaults to true.
    #[serde(default = "crate::generated::types::default_enabled")]
    pub enabled: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdatePluginConfigRequest {
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}
#[derive(Debug, Serialize, Deserialize)]
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
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
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
    pub config_form_fields: Vec<FormField>,
    /// Form field definitions for tenant-level type settings.
    ///
    /// When non-empty, the Settings page shows a per-type-settings form backed
    /// by the `plugin_type_settings` table (e.g., APT `discovery_filter`).
    /// Empty for plugins that have no type-level settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
