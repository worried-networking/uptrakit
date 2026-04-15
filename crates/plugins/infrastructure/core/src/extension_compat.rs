//! Internal compatibility seam for extension-framework bridge types.
//!
//! These definitions are intentionally preserved as-is for behavior parity
//! during the Task 8 migration cutover.

use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;

/// Root descriptor for a UI extension.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceManifest {
    /// Unique extension identifier (e.g., `"ssh-agent.host-management"`).
    pub id: String,
    /// Human-readable name displayed in the UI.
    pub label: String,
    /// Ordering priority - lower values appear first.
    pub priority: i32,
    /// Where this extension appears in the UI.
    pub placement: SurfacePlacement,
    /// Permission required to see and use this extension.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub required_permission: String,
    /// How actions should be routed to service instances.
    #[serde(default)]
    pub targeting: SurfaceTargeting,
    /// The UI definition for this extension.
    pub ui: SurfaceUiDefinition,
}

impl SurfaceManifest {
    /// Create a new extension manifest.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        priority: i32,
        placement: SurfacePlacement,
        ui: SurfaceUiDefinition,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            priority,
            placement,
            required_permission: String::new(),
            targeting: SurfaceTargeting::default(),
            ui,
        }
    }

    /// Set the required permission.
    pub fn with_permission(mut self, permission: impl Into<String>) -> Self {
        self.required_permission = permission.into();
        self
    }

    /// Set the targeting mode.
    pub fn with_targeting(mut self, targeting: SurfaceTargeting) -> Self {
        self.targeting = targeting;
        self
    }
}

/// How actions should be routed to service instances.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SurfaceTargeting {
    /// Any connected instance of the source service type can handle actions.
    #[default]
    Universal,
    /// Actions must be routed to a specific service instance selected by the user.
    Targeted,
    /// An unknown targeting mode received from a newer peer.
    Other(String),
}

impl SurfaceTargeting {
    /// Returns the snake_case wire string for this targeting mode.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Universal => "universal",
            Self::Targeted => "targeted",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for SurfaceTargeting {
    fn from(s: String) -> Self {
        match s.as_str() {
            "universal" => Self::Universal,
            "targeted" => Self::Targeted,
            _ => {
                tracing::debug!(value = s, "received unknown SurfaceTargeting from peer");
                Self::Other(s)
            }
        }
    }
}

impl Serialize for SurfaceTargeting {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SurfaceTargeting {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(SurfaceTargeting::from)
    }
}

/// Where an extension appears in the UI.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SurfacePlacement {
    /// Full sidebar page.
    Page {
        /// Navigation section.
        nav_section: String,
        /// Icon identifier for the navigation item.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    /// Panel injected into an existing page.
    Panel {
        /// Target page identifier.
        target_page: String,
        /// Where on the page to place the panel.
        #[serde(default)]
        position: SurfacePanelPosition,
        /// Shared tab group key for grouped tab rendering.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_group: Option<String>,
    },
    /// Action group added to an entity's context menu.
    ContextMenuGroup {
        /// Entity type this group targets.
        target_entity: String,
        /// Label for the submenu group header.
        group_label: String,
    },
    /// Extra columns added to an existing table.
    TableColumns {
        /// Target table identifier.
        target_table: String,
        /// Column definitions.
        columns: Vec<ExtensionColumn>,
    },
}

/// Position of a panel on an existing page.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SurfacePanelPosition {
    /// Rendered as a tab alongside existing tabs.
    #[default]
    Tab,
    /// Below the main content.
    Below,
    /// Above the main content.
    Above,
    /// Forward-compatible catch-all for unknown positions.
    Other(String),
}

impl Serialize for SurfacePanelPosition {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let type_str = match self {
            Self::Tab => "tab",
            Self::Below => "below",
            Self::Above => "above",
            Self::Other(s) => s.as_str(),
        };
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("type", type_str)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for SurfacePanelPosition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, Visitor};

        struct PanelPositionVisitor;

        impl<'de> Visitor<'de> for PanelPositionVisitor {
            type Value = SurfacePanelPosition;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map with a \"type\" field")
            }

            fn visit_map<V: MapAccess<'de>>(
                self,
                mut map: V,
            ) -> Result<SurfacePanelPosition, V::Error> {
                let mut type_str: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "type" {
                        type_str = Some(map.next_value()?);
                    } else {
                        let _: serde::de::IgnoredAny = map.next_value()?;
                    }
                }
                let type_str = type_str.ok_or_else(|| serde::de::Error::missing_field("type"))?;
                Ok(match type_str.as_str() {
                    "tab" => SurfacePanelPosition::Tab,
                    "below" => SurfacePanelPosition::Below,
                    "above" => SurfacePanelPosition::Above,
                    _ => {
                        tracing::debug!(
                            value = %type_str,
                            "received unknown SurfacePanelPosition from peer"
                        );
                        SurfacePanelPosition::Other(type_str)
                    }
                })
            }
        }

        deserializer.deserialize_map(PanelPositionVisitor)
    }
}

/// Extra column added to an existing table by a `TableColumns` extension.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionColumn {
    /// Column key used in action responses.
    pub key: String,
    /// Column header label.
    pub label: String,
    /// Action to call with row entity IDs to fetch column values.
    pub data_action: String,
}

impl ExtensionColumn {
    /// Create a new extension column.
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        data_action: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            data_action: data_action.into(),
        }
    }
}

/// Source for context selector options.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextSelectorSourceDescriptor {
    /// Call an extension action to populate options.
    Action {
        /// The action ID to invoke.
        action_id: String,
    },
    /// Fetch plugin configurations of a specific type via the REST API.
    PluginConfigs {
        /// Plugin type string to filter by.
        plugin_type: String,
    },
}

/// Context selector shown above a `DataTable` UI.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSelectorDescriptor {
    /// Parameter key injected into all action params when a value is selected.
    pub param_key: String,
    /// Label for the selector dropdown.
    pub label: String,
    /// How to populate the selector options.
    pub source: ContextSelectorSourceDescriptor,
    /// Optional action ID for a "Create" button next to the selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_action: Option<String>,
    /// Message shown when no options are available and no `add_action` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_message: Option<String>,
}

impl ContextSelectorDescriptor {
    /// Create a new context selector.
    pub fn new(
        param_key: impl Into<String>,
        label: impl Into<String>,
        source: ContextSelectorSourceDescriptor,
    ) -> Self {
        Self {
            param_key: param_key.into(),
            label: label.into(),
            source,
            add_action: None,
            empty_message: None,
        }
    }

    /// Set the action ID for a "Create" button.
    pub fn with_add_action(mut self, action_id: impl Into<String>) -> Self {
        self.add_action = Some(action_id.into());
        self
    }

    /// Set the message shown when no options are available.
    pub fn with_empty_message(mut self, message: impl Into<String>) -> Self {
        self.empty_message = Some(message.into());
        self
    }
}

/// Describes a direct REST API call as the submit target for a form action.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiSubmitDescriptor {
    /// HTTP method (e.g., `"POST"`, `"PUT"`, `"PATCH"`, `"DELETE"`).
    pub method: String,
    /// API path relative to the controller base URL.
    pub path: String,
    /// JSON body template.
    pub body: serde_json::Value,
    /// Field in the JSON response containing the new item's identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id_field: Option<String>,
    /// Field in the JSON response containing the new item's display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_label_field: Option<String>,
}

impl ApiSubmitDescriptor {
    /// Create a new API submit definition.
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        body: serde_json::Value,
    ) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            body,
            response_id_field: None,
            response_label_field: None,
        }
    }

    /// Set the response field containing the item's ID.
    pub fn with_response_id_field(mut self, field: impl Into<String>) -> Self {
        self.response_id_field = Some(field.into());
        self
    }

    /// Set the response field containing the item's display label.
    pub fn with_response_label_field(mut self, field: impl Into<String>) -> Self {
        self.response_label_field = Some(field.into());
        self
    }
}

/// Schema-driven UI definition for an extension.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SurfaceUiDefinition {
    /// Data table with columns, row actions, and primary actions.
    DataTable {
        /// Column definitions.
        columns: Vec<SurfaceTableColumn>,
        /// Action ID to invoke to fetch table data.
        data_action: String,
        /// Action IDs available for each row (context menu).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        row_actions: Vec<String>,
        /// Action IDs for primary actions (buttons above the table).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        primary_actions: Vec<String>,
        /// Optional context selector shown above the table.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_selector: Option<Box<ContextSelectorDescriptor>>,
        /// Default number of items per page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_per_page: Option<u64>,
    },
    /// Input form.
    Form(SurfaceFormDescriptor),
    /// Read-only key-value display.
    KeyValue {
        /// Action ID to invoke to fetch data.
        data_action: String,
    },
    /// List of action IDs (used for `ContextMenuGroup` placement).
    Actions {
        /// Action ID references.
        actions: Vec<String>,
    },
}

/// Column descriptor for a `DataTable` UI.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTableColumn {
    /// Column key matching the data field name.
    pub key: String,
    /// Column header label.
    pub label: String,
    /// Whether this column is sortable.
    #[serde(default)]
    pub sortable: bool,
}

impl SurfaceTableColumn {
    /// Create a new table column.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            sortable: false,
        }
    }

    /// Set whether this column is sortable.
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }
}

/// Action descriptor exposed by an extension.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceActionDescriptor {
    /// Action identifier (unique within the source's action library).
    pub action_id: String,
    /// Human-readable label for the action button/menu item.
    pub label: String,
    /// Optional UI shown before the action is invoked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<SurfaceActionUi>,
    /// Permission required to invoke this action.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub permission: String,
    /// Whether this action is destructive (shown with warning styling).
    #[serde(default)]
    pub destructive: bool,
    /// Timeout in seconds for the action invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// When set, form submission calls this REST API endpoint directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_submit: Option<Box<ApiSubmitDescriptor>>,
    /// Conditional visibility for row actions in a `DataTable`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_visible_when: Option<SurfaceRowVisibleWhen>,
    /// Row data field used as entity name in confirmation dialogs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_entity_field: Option<String>,
    /// Whether this action supports batch invocation.
    #[serde(default)]
    pub batch_action: bool,
}

impl SurfaceActionDescriptor {
    /// Create a new action definition.
    pub fn new(action_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
            label: label.into(),
            ui: None,
            permission: String::new(),
            destructive: false,
            timeout_seconds: None,
            api_submit: None,
            row_visible_when: None,
            confirm_entity_field: None,
            batch_action: false,
        }
    }

    /// Set the action UI (form or wizard shown before invocation).
    pub fn with_ui(mut self, ui: SurfaceActionUi) -> Self {
        self.ui = Some(ui);
        self
    }

    /// Set the required permission.
    pub fn with_permission(mut self, permission: impl Into<String>) -> Self {
        self.permission = permission.into();
        self
    }

    /// Mark this action as destructive.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Set the timeout in seconds.
    pub fn with_timeout(mut self, seconds: u32) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }

    /// Set the REST API submit target.
    pub fn with_api_submit(mut self, api_submit: ApiSubmitDescriptor) -> Self {
        self.api_submit = Some(Box::new(api_submit));
        self
    }

    /// Set conditional visibility for row actions in a `DataTable`.
    pub fn with_row_visible_when(
        mut self,
        field: impl Into<String>,
        condition: SurfaceRowCondition,
    ) -> Self {
        self.row_visible_when = Some(SurfaceRowVisibleWhen {
            field: field.into(),
            condition,
        });
        self
    }

    /// Set the row data field used as the entity name in confirmation dialogs.
    pub fn with_confirm_entity_field(mut self, field: impl Into<String>) -> Self {
        self.confirm_entity_field = Some(field.into());
        self
    }

    /// Mark this action as supporting batch invocation.
    pub fn batch(mut self) -> Self {
        self.batch_action = true;
        self
    }
}

/// UI shown before invoking an action.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SurfaceActionUi {
    /// Single form.
    Form(SurfaceFormDescriptor),
    /// Multi-step wizard.
    Wizard {
        /// Ordered list of wizard steps.
        steps: Vec<SurfaceWorkflowStep>,
    },
}

/// A single step in a wizard.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceWorkflowStep {
    /// Step identifier.
    pub step_id: String,
    /// Step label displayed in the step indicator.
    pub label: String,
    /// Form fields for this step.
    pub form: SurfaceFormDescriptor,
    /// Optional action to submit this step's data before proceeding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_action: Option<String>,
    /// When `true`, render previous step's response data instead of a form.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub render_previous_response: bool,
}

impl SurfaceWorkflowStep {
    /// Create a new wizard step.
    pub fn new(
        step_id: impl Into<String>,
        label: impl Into<String>,
        form: SurfaceFormDescriptor,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            label: label.into(),
            form,
            submit_action: None,
            render_previous_response: false,
        }
    }

    /// Set the action to submit this step's data before proceeding.
    pub fn with_submit_action(mut self, action_id: impl Into<String>) -> Self {
        self.submit_action = Some(action_id.into());
        self
    }

    /// Mark this step as rendering the previous step's response data.
    pub fn with_render_previous_response(mut self) -> Self {
        self.render_previous_response = true;
        self
    }
}

/// A form definition with typed fields.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceFormDescriptor {
    /// Ordered list of form fields.
    pub fields: Vec<FormFieldDescriptor>,
    /// Action ID to invoke when the form opens, to pre-populate field values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_load_action: Option<String>,
    /// Action IDs to render as buttons below the form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footer_actions: Vec<String>,
}

impl SurfaceFormDescriptor {
    /// Create a new form definition.
    pub fn new(fields: Vec<FormFieldDescriptor>) -> Self {
        Self {
            fields,
            pre_load_action: None,
            footer_actions: Vec::new(),
        }
    }

    /// Set the action ID to invoke when the form opens for pre-population.
    pub fn with_pre_load_action(mut self, action_id: impl Into<String>) -> Self {
        self.pre_load_action = Some(action_id.into());
        self
    }

    /// Set action IDs to render as buttons below the form.
    pub fn with_footer_actions(mut self, actions: Vec<String>) -> Self {
        self.footer_actions = actions;
        self
    }
}

/// Dynamic data source for `Select` and `MultiSelect` field options.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FormSelectSourceDescriptor {
    /// Fetch options from an authenticated REST API endpoint via `GET`.
    RestApi {
        /// API path relative to the controller base URL.
        path: String,
        /// Field to use as the option value.
        value_field: String,
        /// Field to use as the option label.
        label_field: String,
    },
    /// Fetch options by invoking an extension action.
    Action {
        /// The action ID to invoke.
        action_id: String,
    },
}

/// A single form field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormFieldDescriptor {
    /// Field key used in form submission.
    pub key: String,
    /// Human-readable field label.
    pub label: String,
    /// Field input type.
    #[serde(default)]
    pub field_type: FormFieldType,
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
    /// Static options for `Select` field type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<FormSelectOptionDescriptor>,
    /// Dynamic source for `Select` field options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_source: Option<FormSelectSourceDescriptor>,
    /// Whether this field contains sensitive data.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// Whether this field serializes as newline-separated list.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub list: bool,
    /// Conditional visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<VisibleWhen>,
}

impl FormFieldDescriptor {
    /// Create a new field definition.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            field_type: FormFieldType::default(),
            required: false,
            placeholder: None,
            help_text: None,
            default_value: None,
            options: vec![],
            select_source: None,
            sensitive: false,
            list: false,
            visible_when: None,
        }
    }

    /// Set the field type.
    pub fn with_type(mut self, field_type: FormFieldType) -> Self {
        self.field_type = field_type;
        self
    }

    /// Mark this field as sensitive.
    pub fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }

    /// Mark this field as a newline-separated list.
    pub fn list(mut self) -> Self {
        self.list = true;
        self
    }

    /// Mark this field as required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set placeholder text.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set help text displayed below the field.
    pub fn with_help_text(mut self, help_text: impl Into<String>) -> Self {
        self.help_text = Some(help_text.into());
        self
    }

    /// Set the default value.
    pub fn with_default_value(mut self, value: impl Into<serde_json::Value>) -> Self {
        self.default_value = Some(value.into());
        self
    }

    /// Set static options for a `Select` field.
    pub fn with_options(mut self, options: Vec<FormSelectOptionDescriptor>) -> Self {
        self.options = options;
        self
    }

    /// Set a dynamic source for `Select` field options.
    pub fn with_select_source(mut self, source: FormSelectSourceDescriptor) -> Self {
        self.select_source = Some(source);
        self
    }

    /// Set conditional visibility based on another field's value.
    pub fn with_visible_when(mut self, field: impl Into<String>, values: Vec<String>) -> Self {
        self.visible_when = Some(VisibleWhen {
            field: field.into(),
            values,
        });
        self
    }
}

/// Input field type.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FormFieldType {
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
    /// SSH private key file input.
    SshPrivateKey,
    /// Forward-compatible catch-all for unknown field types.
    Other(String),
}

impl FormFieldType {
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

impl From<String> for FormFieldType {
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
            _ => {
                tracing::debug!(value = s, "received unknown FormFieldType from peer");
                Self::Other(s)
            }
        }
    }
}

impl Serialize for FormFieldType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FormFieldType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(FormFieldType::from)
    }
}

/// Condition for conditional field visibility.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleWhen {
    /// Key of the controlling field.
    pub field: String,
    /// Field is visible when controlling field value is in this list.
    pub values: Vec<String>,
}

/// Conditional visibility for a row action in a `DataTable`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRowVisibleWhen {
    /// Key of the row data field to check.
    pub field: String,
    /// The condition that must hold for the action to be visible.
    pub condition: SurfaceRowCondition,
}

/// Condition type for [`SurfaceRowVisibleWhen`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceRowCondition {
    /// The field must be present and non-null.
    Present,
    /// The field must be absent or `null`.
    Absent,
    /// An unknown condition type received from a newer peer.
    Other(String),
}

impl SurfaceRowCondition {
    /// Returns the snake_case wire string for this condition.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for SurfaceRowCondition {
    fn from(s: String) -> Self {
        match s.as_str() {
            "present" => Self::Present,
            "absent" => Self::Absent,
            _ => {
                tracing::debug!(value = s, "received unknown SurfaceRowCondition from peer");
                Self::Other(s)
            }
        }
    }
}

impl Serialize for SurfaceRowCondition {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SurfaceRowCondition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(SurfaceRowCondition::from)
    }
}

/// A single option in a `Select` field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormSelectOptionDescriptor {
    /// Value submitted when this option is selected.
    pub value: String,
    /// Human-readable label displayed in the dropdown.
    pub label: String,
}

impl FormSelectOptionDescriptor {
    /// Create a new select option.
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

/// Payload for proxied extension action invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct ExtensionRequestPayload {
    /// Correlation ID (UUID v7) for matching request to response.
    pub request_id: String,
    /// Extension ID the action belongs to.
    pub extension_id: String,
    /// Action ID to invoke.
    pub action_id: String,
    /// Action parameters as JSON (non-sensitive fields only).
    #[serde(default)]
    pub params: serde_json::Value,
    /// ECIES sealed-box ciphertext containing sensitive parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive_params: Option<SecretString>,
    /// Tenant context for the requesting user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<uuid::Uuid>,
}

/// Payload for proxied extension action response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct ExtensionResponsePayload {
    /// Correlation ID matching the request.
    pub request_id: String,
    /// Whether the action succeeded.
    pub success: bool,
    /// Result data (on success) or additional context (on failure).
    #[serde(default)]
    pub data: serde_json::Value,
    /// Error message (on failure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::Permission;

    #[test]
    fn extension_manifest_roundtrip_page() {
        let manifest = SurfaceManifest {
            id: "ssh-agent.host-management".to_string(),
            label: "SSH Host Management".to_string(),
            priority: 250,
            placement: SurfacePlacement::Page {
                nav_section: "management".to_string(),
                icon: Some("server".to_string()),
            },
            required_permission: Permission::UpdateHosts.into(),
            targeting: SurfaceTargeting::Targeted,
            ui: SurfaceUiDefinition::DataTable {
                columns: vec![SurfaceTableColumn {
                    key: "hostname".to_string(),
                    label: "Hostname".to_string(),
                    sortable: true,
                }],
                data_action: "list-hosts".to_string(),
                row_actions: vec![],
                primary_actions: vec![],
                context_selector: None,
                default_per_page: None,
            },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        let roundtripped: SurfaceManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_panel() {
        let manifest = SurfaceManifest {
            id: "proxmox.lxc-panel".to_string(),
            label: "LXC Matching".to_string(),
            priority: 0,
            placement: SurfacePlacement::Panel {
                target_page: "hosts".to_string(),
                position: SurfacePanelPosition::Below,
                tab_group: None,
            },
            required_permission: String::new(),
            targeting: SurfaceTargeting::Universal,
            ui: SurfaceUiDefinition::KeyValue {
                data_action: "get-lxc-info".to_string(),
            },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        assert!(
            !json.contains("tab_group"),
            "tab_group should be omitted when None"
        );
        let roundtripped: SurfaceManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_panel_with_tab_group() {
        let manifest = SurfaceManifest {
            id: "notifications.webhook".to_string(),
            label: "Webhook Channels".to_string(),
            priority: 500,
            placement: SurfacePlacement::Panel {
                target_page: "settings".to_string(),
                position: SurfacePanelPosition::Tab,
                tab_group: Some("Notification Channels".to_string()),
            },
            required_permission: "view_notifications".to_string(),
            targeting: SurfaceTargeting::Universal,
            ui: SurfaceUiDefinition::Actions {
                actions: vec!["list".to_string()],
            },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        assert!(
            json.contains(r#""tab_group":"Notification Channels""#),
            "tab_group should be present when Some"
        );
        let roundtripped: SurfaceManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_context_menu() {
        let manifest = SurfaceManifest {
            id: "ssh-agent.host-actions".to_string(),
            label: "SSH Actions".to_string(),
            priority: 100,
            placement: SurfacePlacement::ContextMenuGroup {
                target_entity: "host".to_string(),
                group_label: "SSH Agent".to_string(),
            },
            required_permission: String::new(),
            targeting: SurfaceTargeting::Targeted,
            ui: SurfaceUiDefinition::Actions {
                actions: vec!["bootstrap".to_string()],
            },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        let roundtripped: SurfaceManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_table_columns() {
        let manifest = SurfaceManifest {
            id: "ssh-agent.host-columns".to_string(),
            label: "SSH Status".to_string(),
            priority: 50,
            placement: SurfacePlacement::TableColumns {
                target_table: "hosts".to_string(),
                columns: vec![ExtensionColumn {
                    key: "ssh_status".to_string(),
                    label: "SSH Status".to_string(),
                    data_action: "get-ssh-status".to_string(),
                }],
            },
            required_permission: String::new(),
            targeting: SurfaceTargeting::Universal,
            ui: SurfaceUiDefinition::Actions { actions: vec![] },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        let roundtripped: SurfaceManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn panel_position_default_is_tab() {
        assert_eq!(SurfacePanelPosition::default(), SurfacePanelPosition::Tab);
    }

    #[test]
    fn panel_position_tab_serializes_as_object() {
        let pos = SurfacePanelPosition::Tab;
        let json = serde_json::to_string(&pos).expect("serialize should succeed");
        assert_eq!(json, r#"{"type":"tab"}"#);
        let roundtripped: SurfacePanelPosition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(pos, roundtripped);
    }

    #[test]
    fn panel_position_below_serializes_as_object() {
        let pos = SurfacePanelPosition::Below;
        let json = serde_json::to_string(&pos).expect("serialize should succeed");
        assert_eq!(json, r#"{"type":"below"}"#);
        let roundtripped: SurfacePanelPosition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(pos, roundtripped);
    }

    #[test]
    fn panel_position_other_roundtrip() {
        let pos = SurfacePanelPosition::Other("sidebar".to_string());
        let json = serde_json::to_string(&pos).expect("serialize should succeed");
        assert_eq!(json, r#"{"type":"sidebar"}"#);
        let roundtripped: SurfacePanelPosition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(pos, roundtripped);
    }

    #[test]
    fn panel_position_unknown_type_deserializes_to_other() {
        let json = r#"{"type":"floating"}"#;
        let pos: SurfacePanelPosition =
            serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(pos, SurfacePanelPosition::Other("floating".to_string()));
    }

    #[test]
    fn field_type_default_is_text() {
        assert_eq!(FormFieldType::default(), FormFieldType::Text);
    }

    #[test]
    fn field_type_known_variants_serialize_as_plain_string() {
        assert_eq!(
            serde_json::to_string(&FormFieldType::Text).unwrap(),
            r#""text""#
        );
        assert_eq!(
            serde_json::to_string(&FormFieldType::MultiSelect).unwrap(),
            r#""multi_select""#
        );
        assert_eq!(
            serde_json::to_string(&FormFieldType::Toggle).unwrap(),
            r#""toggle""#
        );
    }

    #[test]
    fn field_type_other_roundtrip() {
        let ft = FormFieldType::Other("color_picker".to_string());
        let json = serde_json::to_string(&ft).expect("serialize should succeed");
        assert_eq!(json, r#""color_picker""#);
        let roundtripped: FormFieldType =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(ft, roundtripped);
    }

    #[test]
    fn field_type_unknown_deserializes_to_other() {
        let json = r#""date_picker""#;
        let ft: FormFieldType = serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(ft, FormFieldType::Other("date_picker".to_string()));
    }

    #[test]
    fn extension_targeting_default_is_universal() {
        assert_eq!(SurfaceTargeting::default(), SurfaceTargeting::Universal);
    }

    #[test]
    fn extension_targeting_known_variants_roundtrip() {
        let variants = [SurfaceTargeting::Universal, SurfaceTargeting::Targeted];
        for v in &variants {
            let json = serde_json::to_string(v).expect("serialize should succeed");
            let roundtripped: SurfaceTargeting =
                serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(v, &roundtripped);
        }
    }

    #[test]
    fn extension_targeting_other_roundtrip() {
        let t = SurfaceTargeting::Other("scoped".to_string());
        let json = serde_json::to_string(&t).expect("serialize should succeed");
        assert_eq!(json, r#""scoped""#);
        let roundtripped: SurfaceTargeting =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(t, roundtripped);
    }

    #[test]
    fn extension_targeting_unknown_deserializes_to_other() {
        let json = r#""tenant_specific""#;
        let t: SurfaceTargeting = serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(t, SurfaceTargeting::Other("tenant_specific".to_string()));
    }

    #[test]
    fn row_condition_known_variants_roundtrip() {
        let variants = [SurfaceRowCondition::Present, SurfaceRowCondition::Absent];
        for v in &variants {
            let json = serde_json::to_string(v).expect("serialize should succeed");
            let roundtripped: SurfaceRowCondition =
                serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(v, &roundtripped);
        }
    }

    #[test]
    fn row_condition_other_roundtrip() {
        let c = SurfaceRowCondition::Other("non_empty".to_string());
        let json = serde_json::to_string(&c).expect("serialize should succeed");
        assert_eq!(json, r#""non_empty""#);
        let roundtripped: SurfaceRowCondition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(c, roundtripped);
    }

    #[test]
    fn row_condition_unknown_deserializes_to_other() {
        let json = r#""matches_regex""#;
        let c: SurfaceRowCondition =
            serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(c, SurfaceRowCondition::Other("matches_regex".to_string()));
    }

    #[test]
    fn wizard_step_roundtrip() {
        let step = SurfaceWorkflowStep {
            step_id: "step1".to_string(),
            label: "Enter Details".to_string(),
            form: SurfaceFormDescriptor {
                fields: vec![FormFieldDescriptor {
                    key: "host".to_string(),
                    label: "Host".to_string(),
                    field_type: FormFieldType::Text,
                    required: true,
                    placeholder: None,
                    help_text: Some("Enter the hostname".to_string()),
                    default_value: Some(serde_json::Value::String("localhost".to_string())),
                    options: vec![],
                    select_source: None,
                    sensitive: false,
                    list: false,
                    visible_when: None,
                }],
                pre_load_action: None,
                footer_actions: vec![],
            },
            submit_action: Some("validate-host".to_string()),
            render_previous_response: false,
        };

        let json = serde_json::to_string(&step).expect("serialize should succeed");
        let roundtripped: SurfaceWorkflowStep =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(step, roundtripped);
    }

    #[test]
    fn action_ui_wizard_roundtrip() {
        let ui = SurfaceActionUi::Wizard {
            steps: vec![SurfaceWorkflowStep {
                step_id: "s1".to_string(),
                label: "Step 1".to_string(),
                form: SurfaceFormDescriptor {
                    fields: vec![],
                    pre_load_action: None,
                    footer_actions: vec![],
                },
                submit_action: None,
                render_previous_response: false,
            }],
        };

        let json = serde_json::to_string(&ui).expect("serialize should succeed");
        let roundtripped: SurfaceActionUi =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(ui, roundtripped);
    }

    #[test]
    fn select_option_roundtrip() {
        let opt = FormSelectOptionDescriptor {
            value: "opt1".to_string(),
            label: "Option 1".to_string(),
        };
        let json = serde_json::to_string(&opt).expect("serialize should succeed");
        let roundtripped: FormSelectOptionDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(opt, roundtripped);
    }

    #[test]
    fn extension_request_payload_roundtrip() {
        let payload = ExtensionRequestPayload {
            request_id: "req-123".to_string(),
            extension_id: "test.ext".to_string(),
            action_id: "do-thing".to_string(),
            params: serde_json::json!({"key": "value"}),
            sensitive_params: None,
            tenant_id: None,
        };

        let json = serde_json::to_string(&payload).expect("serialize should succeed");
        let roundtripped: ExtensionRequestPayload =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(payload, roundtripped);
    }

    #[test]
    fn extension_response_payload_roundtrip_success() {
        let payload = ExtensionResponsePayload {
            request_id: "req-123".to_string(),
            success: true,
            data: serde_json::json!({"rows": []}),
            error: None,
        };

        let json = serde_json::to_string(&payload).expect("serialize should succeed");
        assert!(!json.contains("error"));
        let roundtripped: ExtensionResponsePayload =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(payload, roundtripped);
    }

    #[test]
    fn extension_response_payload_roundtrip_error() {
        let payload = ExtensionResponsePayload {
            request_id: "req-456".to_string(),
            success: false,
            data: serde_json::Value::Null,
            error: Some("action failed".to_string()),
        };

        let json = serde_json::to_string(&payload).expect("serialize should succeed");
        let roundtripped: ExtensionResponsePayload =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(payload, roundtripped);
    }

    #[test]
    fn destructive_action_serialization() {
        let action = SurfaceActionDescriptor::new("delete-all", "Delete All")
            .with_permission(Permission::UpdateHosts)
            .destructive();

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(json.contains(r#""destructive":true"#));
        let roundtripped: SurfaceActionDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(action, roundtripped);
    }

    #[test]
    fn optional_fields_omitted_when_default() {
        let manifest = SurfaceManifest {
            id: "test".to_string(),
            label: "Test".to_string(),
            priority: 0,
            placement: SurfacePlacement::Page {
                nav_section: "test".to_string(),
                icon: None,
            },
            required_permission: String::new(),
            targeting: SurfaceTargeting::Universal,
            ui: SurfaceUiDefinition::Actions { actions: vec![] },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        assert!(!json.contains("required_permission"));
        assert!(!json.contains("icon"));
    }

    #[test]
    fn select_source_rest_api_roundtrip() {
        let source = FormSelectSourceDescriptor::RestApi {
            path: "/api/v1/hosts".to_string(),
            value_field: "id".to_string(),
            label_field: "friendly_name".to_string(),
        };
        let json = serde_json::to_string(&source).expect("serialize should succeed");
        assert!(json.contains(r#""type":"rest_api""#));
        let roundtripped: FormSelectSourceDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(source, roundtripped);
    }

    #[test]
    fn field_def_with_select_source_roundtrip() {
        let field = FormFieldDescriptor::new("host_id", "Host")
            .with_type(FormFieldType::Select)
            .required()
            .with_select_source(FormSelectSourceDescriptor::RestApi {
                path: "/api/v1/hosts".to_string(),
                value_field: "id".to_string(),
                label_field: "friendly_name".to_string(),
            });
        let json = serde_json::to_string(&field).expect("serialize should succeed");
        assert!(json.contains("select_source"));
        assert!(!json.contains("\"options\""));
        let roundtripped: FormFieldDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(field, roundtripped);
    }

    #[test]
    fn form_with_select_field() {
        let form = SurfaceFormDescriptor {
            fields: vec![FormFieldDescriptor {
                key: "region".to_string(),
                label: "Region".to_string(),
                field_type: FormFieldType::Select,
                required: true,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: vec![
                    FormSelectOptionDescriptor {
                        value: "us-east".to_string(),
                        label: "US East".to_string(),
                    },
                    FormSelectOptionDescriptor {
                        value: "eu-west".to_string(),
                        label: "EU West".to_string(),
                    },
                ],
                select_source: None,
                sensitive: false,
                list: false,
                visible_when: None,
            }],
            pre_load_action: None,
            footer_actions: vec![],
        };

        let json = serde_json::to_string(&form).expect("serialize should succeed");
        let roundtripped: SurfaceFormDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(form, roundtripped);
    }

    #[test]
    fn priority_sorting() {
        let mut items = [("B", 200), ("A", 200), ("C", 100), ("D", 300)];
        items.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        let labels: Vec<&str> = items.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, vec!["C", "A", "B", "D"]);
    }

    #[test]
    fn ui_action_refs_are_strings() {
        let ui = SurfaceUiDefinition::DataTable {
            columns: vec![],
            data_action: "list".to_string(),
            row_actions: vec!["edit".to_string(), "delete".to_string()],
            primary_actions: vec!["create".to_string()],
            context_selector: None,
            default_per_page: None,
        };

        let json = serde_json::to_string(&ui).expect("serialize should succeed");
        assert!(json.contains(r#""row_actions":["edit","delete"]"#));
        let roundtripped: SurfaceUiDefinition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(ui, roundtripped);
    }

    #[test]
    fn context_selector_add_action_is_string_ref() {
        let cs = ContextSelectorDescriptor::new(
            "config_id",
            "Configuration",
            ContextSelectorSourceDescriptor::PluginConfigs {
                plugin_type: "infrastructure_proxmox".to_string(),
            },
        )
        .with_add_action("add-config")
        .with_empty_message("No configurations found.");

        let json = serde_json::to_string(&cs).expect("serialize should succeed");
        assert!(json.contains(r#""add_action":"add-config""#));
        let roundtripped: ContextSelectorDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(cs, roundtripped);
    }

    #[test]
    fn select_option_new() {
        let opt = FormSelectOptionDescriptor::new("val", "Label");
        assert_eq!(opt.value, "val");
        assert_eq!(opt.label, "Label");
    }

    #[test]
    fn wizard_step_new_with_submit_action() {
        let step = SurfaceWorkflowStep::new("s1", "Step 1", SurfaceFormDescriptor::new(vec![]))
            .with_submit_action("validate");
        assert_eq!(step.step_id, "s1");
        assert_eq!(step.label, "Step 1");
        assert_eq!(step.submit_action.as_deref(), Some("validate"));
    }

    #[test]
    fn extension_column_new() {
        let col = ExtensionColumn::new("ssh_status", "SSH Status", "get-ssh-status");
        assert_eq!(col.key, "ssh_status");
        assert_eq!(col.label, "SSH Status");
        assert_eq!(col.data_action, "get-ssh-status");
    }

    #[test]
    fn field_def_with_default_value() {
        let field = FormFieldDescriptor::new("name", "Name").with_default_value("default");
        assert_eq!(
            field.default_value,
            Some(serde_json::Value::String("default".to_string()))
        );
    }

    #[test]
    fn field_def_with_options() {
        let field = FormFieldDescriptor::new("region", "Region")
            .with_type(FormFieldType::Select)
            .with_options(vec![
                FormSelectOptionDescriptor::new("us", "US"),
                FormSelectOptionDescriptor::new("eu", "EU"),
            ]);
        assert_eq!(field.options.len(), 2);
        assert_eq!(field.options[0].value, "us");
    }

    #[test]
    fn select_source_action_roundtrip() {
        let source = FormSelectSourceDescriptor::Action {
            action_id: "list-discovered-guests".to_string(),
        };
        let json = serde_json::to_string(&source).expect("serialize should succeed");
        assert!(json.contains(r#""type":"action""#));
        let roundtripped: FormSelectSourceDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(source, roundtripped);
    }

    #[test]
    fn visible_when_roundtrip() {
        let field = FormFieldDescriptor::new("auth_username", "Username")
            .with_visible_when("auth_type", vec!["basic".to_string()]);
        let json = serde_json::to_string(&field).expect("serialize should succeed");
        assert!(json.contains("visible_when"));
        let roundtripped: FormFieldDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(field, roundtripped);
        let vw = roundtripped.visible_when.expect("should have visible_when");
        assert_eq!(vw.field, "auth_type");
        assert_eq!(vw.values, vec!["basic"]);
    }

    #[test]
    fn visible_when_omitted_when_none() {
        let field = FormFieldDescriptor::new("name", "Name");
        let json = serde_json::to_string(&field).expect("serialize should succeed");
        assert!(!json.contains("visible_when"));
    }

    #[test]
    fn row_visible_when_roundtrip() {
        let action = SurfaceActionDescriptor::new("approve-match", "Approve Match")
            .with_row_visible_when("suggested_host_id", SurfaceRowCondition::Present);

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        let roundtripped: SurfaceActionDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");

        let rvw = roundtripped.row_visible_when.expect("should be set");
        assert_eq!(rvw.field, "suggested_host_id");
        assert_eq!(rvw.condition, SurfaceRowCondition::Present);
    }

    #[test]
    fn row_visible_when_absent_condition() {
        let action = SurfaceActionDescriptor::new("unmatch", "Remove Match")
            .with_row_visible_when("matched_host", SurfaceRowCondition::Absent);

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        let roundtripped: SurfaceActionDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");

        let rvw = roundtripped.row_visible_when.expect("should be set");
        assert_eq!(rvw.field, "matched_host");
        assert_eq!(rvw.condition, SurfaceRowCondition::Absent);
    }

    #[test]
    fn row_visible_when_omitted_when_none() {
        let action = SurfaceActionDescriptor::new("match", "Manual Match");
        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(!json.contains("row_visible_when"));
    }

    #[test]
    fn confirm_entity_field_roundtrip() {
        let action = SurfaceActionDescriptor::new("remove-host", "Remove Host")
            .destructive()
            .with_confirm_entity_field("name");

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(json.contains(r#""confirm_entity_field":"name""#));
        let roundtripped: SurfaceActionDescriptor =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(roundtripped.confirm_entity_field.as_deref(), Some("name"));
    }

    #[test]
    fn confirm_entity_field_omitted_when_none() {
        let action = SurfaceActionDescriptor::new("list", "List Items");
        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(!json.contains("confirm_entity_field"));
    }
}
