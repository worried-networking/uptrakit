use crate::generated::surfaces::InteractionId;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormUiDescriptor {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FormFieldDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_load_interaction_id: Option<InteractionId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormFieldDescriptor {
    pub key: String,
    pub label: String,
    pub field_type: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<FormSelectOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_source: Option<FormSelectSource>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub list: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<FormVisibleWhen>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormSelectOption {
    pub value: String,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FormSelectSource {
    RestApi {
        path: String,
        value_field: String,
        label_field: String,
    },
    Action {
        action_id: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormVisibleWhen {
    pub field: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}
