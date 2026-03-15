//! UI extension framework types.
//!
//! Extensions allow plugins and connected services to dynamically contribute
//! UI elements (pages, panels, context menu groups, table columns) and expose
//! actions that the controller proxies to the appropriate service instance.
//!
//! ## Action library
//!
//! Actions are defined centrally in an **action library** — a flat catalogue of
//! [`ActionDef`] structs registered via [`ExtensionActionsPayload`]. UI
//! definitions reference actions by `action_id` string only, never embedding
//! the full definition inline. This enables action reuse across multiple
//! extensions from the same source (service or plugin).
//!
//! All public types are `#[non_exhaustive]` for forward compatibility.

use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;

// ── Extension manifest ──────────────────────────────────────────────────────

/// Root descriptor for a UI extension.
///
/// Each extension is identified by a unique `id` (e.g., `"ssh-agent.host-management"`)
/// and declares where it should appear in the UI, what permissions are required,
/// and how actions should be routed.
///
/// Actions are **not** embedded in the manifest. Instead, they live in a
/// separate action library (registered via [`ExtensionActionsPayload`]) and
/// are referenced by `action_id` strings within the UI definition.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionManifest {
    /// Unique extension identifier (e.g., `"ssh-agent.host-management"`).
    pub id: String,
    /// Human-readable name displayed in the UI.
    pub label: String,
    /// Ordering priority — lower values appear first. Items with equal
    /// priority are sorted alphabetically by `label`.
    pub priority: i32,
    /// Where this extension appears in the UI.
    pub placement: ExtensionPlacement,
    /// Permission required to see and use this extension (e.g., `Permission::UpdateHosts`).
    ///
    /// Empty string means no permission required beyond authentication.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub required_permission: String,
    /// How actions should be routed to service instances.
    #[serde(default)]
    pub targeting: ExtensionTargeting,
    /// The UI definition for this extension.
    pub ui: ExtensionUi,
}

impl ExtensionManifest {
    /// Create a new extension manifest.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        priority: i32,
        placement: ExtensionPlacement,
        ui: ExtensionUi,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            priority,
            placement,
            required_permission: String::new(),
            targeting: ExtensionTargeting::default(),
            ui,
        }
    }

    /// Set the required permission.
    pub fn with_permission(mut self, permission: impl Into<String>) -> Self {
        self.required_permission = permission.into();
        self
    }

    /// Set the targeting mode.
    pub fn with_targeting(mut self, targeting: ExtensionTargeting) -> Self {
        self.targeting = targeting;
        self
    }
}

/// How actions should be routed to service instances.
///
/// # Wire forward-compatibility
///
/// `Other(String)` preserves unknown targeting modes from newer peers.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ExtensionTargeting {
    /// Any connected instance of the source service type can handle actions.
    /// The controller picks one automatically.
    #[default]
    Universal,
    /// Actions must be routed to a specific service instance selected by the user.
    Targeted,
    /// An unknown targeting mode received from a newer peer.
    Other(String),
}

impl ExtensionTargeting {
    /// Returns the snake_case wire string for this targeting mode.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Universal => "universal",
            Self::Targeted => "targeted",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for ExtensionTargeting {
    fn from(s: String) -> Self {
        match s.as_str() {
            "universal" => Self::Universal,
            "targeted" => Self::Targeted,
            _ => {
                tracing::debug!(value = s, "received unknown ExtensionTargeting from peer");
                Self::Other(s)
            }
        }
    }
}

impl Serialize for ExtensionTargeting {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ExtensionTargeting {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(ExtensionTargeting::from)
    }
}

/// Where an extension appears in the UI.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtensionPlacement {
    /// Full sidebar page.
    Page {
        /// Navigation section (e.g., `"monitoring"`, `"management"`).
        nav_section: String,
        /// Icon identifier for the navigation item.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    /// Panel injected into an existing page.
    Panel {
        /// Target page identifier (e.g., `"hosts"`, `"services"`).
        target_page: String,
        /// Where on the page to place the panel.
        #[serde(default)]
        position: PanelPosition,
        /// When set on Tab-positioned panels, all panels sharing the same
        /// `(target_page, tab_group)` render as sections within one shared tab.
        /// The tab label is the `tab_group` value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_group: Option<String>,
    },
    /// Action group added to an entity's context menu.
    ContextMenuGroup {
        /// Entity type this group targets (e.g., `"host"`, `"service"`).
        target_entity: String,
        /// Label for the submenu group header.
        group_label: String,
    },
    /// Extra columns added to an existing table.
    TableColumns {
        /// Target table identifier (e.g., `"hosts"`, `"services"`).
        target_table: String,
        /// Column definitions.
        columns: Vec<ExtensionColumn>,
    },
}

/// Position of a panel on an existing page.
///
/// Serializes as `{"type": "tab"}`, `{"type": "below"}`, etc. to match
/// the frontend `PanelPosition` TypeScript interface. Unknown positions
/// from newer peers are preserved via `Other(String)`.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PanelPosition {
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

impl Serialize for PanelPosition {
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

impl<'de> Deserialize<'de> for PanelPosition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, Visitor};

        struct PanelPositionVisitor;

        impl<'de> Visitor<'de> for PanelPositionVisitor {
            type Value = PanelPosition;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map with a \"type\" field")
            }

            fn visit_map<V: MapAccess<'de>>(self, mut map: V) -> Result<PanelPosition, V::Error> {
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
                    "tab" => PanelPosition::Tab,
                    "below" => PanelPosition::Below,
                    "above" => PanelPosition::Above,
                    _ => {
                        tracing::debug!(
                            value = %type_str,
                            "received unknown PanelPosition from peer"
                        );
                        PanelPosition::Other(type_str)
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

// ── UI definitions ──────────────────────────────────────────────────────────

/// Source for context selector options.
///
/// Determines how the frontend populates the dropdown that appears above a
/// `DataTable` when `context_selector` is set.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextSelectorSource {
    /// Call an extension action to populate options.
    ///
    /// The action must return a JSON array of `{ "value": "...", "label": "..." }` objects.
    Action {
        /// The action ID to invoke.
        action_id: String,
    },
    /// Fetch plugin configurations of a specific type via the REST API.
    ///
    /// The frontend calls `GET /api/v1/plugin-configs`, filters by `plugin_type`,
    /// and maps each result to `{ value: id, label: name }`. No extension action is needed.
    PluginConfigs {
        /// Plugin type string to filter by (e.g., `"infrastructure_proxmox"`).
        plugin_type: String,
    },
}

/// Context selector shown above a `DataTable` UI.
///
/// When present, the user must choose a context value (e.g., a plugin config)
/// before table data loads. The selected value is automatically injected into
/// every action invocation (data load, primary actions, row actions) under
/// `param_key`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSelectorDef {
    /// Parameter key injected into all action params when a value is selected.
    pub param_key: String,
    /// Label for the selector dropdown.
    pub label: String,
    /// How to populate the selector options.
    pub source: ContextSelectorSource,
    /// Optional action ID for a "Create" button next to the selector.
    ///
    /// The referenced action is looked up in the action library. It may route
    /// through the extension proxy (no `api_submit`) or call an existing REST
    /// API directly (with `api_submit` set). After the action completes the
    /// options list refreshes and the new item is auto-selected if the response
    /// includes the field named by `ActionDef::api_submit.response_id_field`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_action: Option<String>,
    /// Message shown when no options are available and no `add_action` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_message: Option<String>,
}

impl ContextSelectorDef {
    /// Create a new context selector.
    pub fn new(
        param_key: impl Into<String>,
        label: impl Into<String>,
        source: ContextSelectorSource,
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
///
/// When `ActionDef::api_submit` is set, form submission bypasses the extension
/// proxy and calls the specified REST API endpoint instead. This allows
/// extensions to expose existing API operations as first-class actions without
/// duplicating server-side logic in an extension handler.
///
/// ## Body template syntax
///
/// `body` is a JSON value tree. Any string leaf matching `{{field_name}}` or
/// `{{field_name:coercion}}` is replaced with the corresponding form-field value,
/// with optional type coercion:
///
/// | Syntax | Coercion |
/// |--------|----------|
/// | `{{name}}` | String (default) |
/// | `{{enabled:bool}}` | Converts `"true"` → `true`, anything else → `false` |
/// | `{{tags:csv_array}}` | Splits on `,`, trims whitespace, drops empty strings → JSON array |
/// | `{{count:number}}` | Converts the string to a JSON number |
///
/// ## Example — create a plugin config
///
/// ```json
/// {
///   "name": "{{name}}",
///   "plugin_type": "infrastructure_proxmox",
///   "enabled": true,
///   "config": {
///     "api_url": "{{api_url}}",
///     "api_token": "{{api_token}}",
///     "verify_tls": "{{verify_tls:bool}}",
///     "node_filter": "{{node_filter:csv_array}}"
///   }
/// }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiSubmitDef {
    /// HTTP method (e.g., `"POST"`, `"PUT"`, `"PATCH"`, `"DELETE"`).
    pub method: String,
    /// API path relative to the controller base URL (e.g., `"/api/v1/plugin-configs"`).
    pub path: String,
    /// JSON body template (see struct-level documentation for template syntax).
    pub body: serde_json::Value,
    /// Field in the JSON response containing the new item's identifier.
    ///
    /// When set, this value is returned to the caller (e.g., context selector) so
    /// the new item can be auto-selected after creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id_field: Option<String>,
    /// Field in the JSON response containing the new item's display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_label_field: Option<String>,
}

impl ApiSubmitDef {
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
///
/// Action fields (`row_actions`, `primary_actions`, etc.) contain `action_id`
/// strings that reference entries in the source's action library.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtensionUi {
    /// Data table with columns, row actions, and primary actions.
    DataTable {
        /// Column definitions.
        columns: Vec<TableColumn>,
        /// Action ID to invoke to fetch table data.
        data_action: String,
        /// Action IDs available for each row (context menu).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        row_actions: Vec<String>,
        /// Action IDs for primary actions (buttons above the table).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        primary_actions: Vec<String>,
        /// Optional context selector shown above the table.
        ///
        /// When set, the user must select a value before data loads. The selection
        /// is injected into all action params under `context_selector.param_key`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_selector: Option<Box<ContextSelectorDef>>,
        /// Default number of items per page.
        ///
        /// When set, the data action receives `page` and `per_page` params and
        /// must return a paginated response (`items`, `total`, `page`, `per_page`,
        /// `total_pages`). When `None`, defaults to 20.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_per_page: Option<u64>,
    },
    /// Input form.
    Form(FormDef),
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
pub struct TableColumn {
    /// Column key matching the data field name.
    pub key: String,
    /// Column header label.
    pub label: String,
    /// Whether this column is sortable.
    #[serde(default)]
    pub sortable: bool,
}

impl TableColumn {
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
///
/// Defined in the action library (via [`ExtensionActionsPayload`]) and
/// referenced by `action_id` from UI definitions.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDef {
    /// Action identifier (unique within the source's action library).
    pub action_id: String,
    /// Human-readable label for the action button/menu item.
    pub label: String,
    /// Optional UI shown before the action is invoked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<ActionUi>,
    /// Permission required to invoke this action.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub permission: String,
    /// Whether this action is destructive (shown with warning styling).
    #[serde(default)]
    pub destructive: bool,
    /// Timeout in seconds for the action invocation. `None` uses the default (30s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// When set, form submission calls this REST API endpoint directly instead of
    /// routing through the extension proxy. Allows extensions to use existing API
    /// operations without duplicating server-side handler logic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_submit: Option<Box<ApiSubmitDef>>,
    /// Conditional visibility for row actions in a `DataTable`.
    ///
    /// When set, the action button is only shown in a table row if the
    /// specified condition on a row data field is met. This allows actions
    /// like "Approve Match" to only appear when a suggestion exists, or
    /// "Remove Match" to only appear for already-matched rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_visible_when: Option<RowVisibleWhen>,
    /// Row data field to use as the entity name in the confirmation dialog
    /// for destructive actions without a form UI. When set, the frontend
    /// shows a confirmation dialog before invoking the action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_entity_field: Option<String>,
    /// Whether this action supports batch invocation on multiple selected rows.
    ///
    /// When `true`, the frontend shows this action in the batch action bar when
    /// multiple rows are selected. The `ids` parameter in the action params
    /// carries the selected row IDs.
    #[serde(default)]
    pub batch_action: bool,
}

impl ActionDef {
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
    pub fn with_ui(mut self, ui: ActionUi) -> Self {
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

    /// Set the REST API submit target (bypasses extension proxy on form submission).
    pub fn with_api_submit(mut self, api_submit: ApiSubmitDef) -> Self {
        self.api_submit = Some(Box::new(api_submit));
        self
    }

    /// Set conditional visibility for row actions in a `DataTable`.
    ///
    /// The action button is only shown in rows where the specified condition
    /// on the given field is met.
    pub fn with_row_visible_when(
        mut self,
        field: impl Into<String>,
        condition: RowCondition,
    ) -> Self {
        self.row_visible_when = Some(RowVisibleWhen {
            field: field.into(),
            condition,
        });
        self
    }

    /// Set the row data field used as the entity name in the confirmation
    /// dialog for destructive actions. The frontend shows a confirmation
    /// prompt before invoking the action, using the value of this field
    /// from the current row as the entity name.
    pub fn with_confirm_entity_field(mut self, field: impl Into<String>) -> Self {
        self.confirm_entity_field = Some(field.into());
        self
    }

    /// Mark this action as supporting batch invocation on multiple selected rows.
    pub fn batch(mut self) -> Self {
        self.batch_action = true;
        self
    }
}

/// UI shown before invoking an action.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionUi {
    /// Single form.
    Form(FormDef),
    /// Multi-step wizard.
    Wizard {
        /// Ordered list of wizard steps.
        steps: Vec<WizardStep>,
    },
}

/// A single step in a wizard.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WizardStep {
    /// Step identifier.
    pub step_id: String,
    /// Step label displayed in the step indicator.
    pub label: String,
    /// Form fields for this step.
    pub form: FormDef,
    /// Optional action to submit this step's data before proceeding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_action: Option<String>,
}

impl WizardStep {
    /// Create a new wizard step.
    pub fn new(step_id: impl Into<String>, label: impl Into<String>, form: FormDef) -> Self {
        Self {
            step_id: step_id.into(),
            label: label.into(),
            form,
            submit_action: None,
        }
    }

    /// Set the action to submit this step's data before proceeding.
    pub fn with_submit_action(mut self, action_id: impl Into<String>) -> Self {
        self.submit_action = Some(action_id.into());
        self
    }
}

/// A form definition with typed fields.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDef {
    /// Ordered list of form fields.
    pub fields: Vec<FieldDef>,
    /// Action ID to invoke when the form opens, to pre-populate field values.
    ///
    /// The action response is a flat JSON object whose keys map to field keys.
    /// Values are used as initial field values (overriding `default_value`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_load_action: Option<String>,
    /// Action IDs to render as buttons below the form (after the save button).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footer_actions: Vec<String>,
}

impl FormDef {
    /// Create a new form definition.
    pub fn new(fields: Vec<FieldDef>) -> Self {
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

    /// Set action IDs to render as buttons below the form (after the save button).
    pub fn with_footer_actions(mut self, actions: Vec<String>) -> Self {
        self.footer_actions = actions;
        self
    }
}

/// Dynamic data source for `Select` and `MultiSelect` field options.
///
/// When a `FieldDef` with `field_type = Select` or `field_type = MultiSelect`
/// has `select_source` set, the frontend calls the specified endpoint at
/// form-open time to populate the options. Static `options` and `select_source`
/// are mutually exclusive; when both are present `select_source` takes
/// precedence.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SelectSource {
    /// Fetch options from an authenticated REST API endpoint via `GET`.
    ///
    /// The frontend calls `GET {path}`, then maps each item in the response
    /// array (or the `items` field of a paginated response) using `value_field`
    /// as the option value and `label_field` as the option label.
    RestApi {
        /// API path relative to the controller base URL (e.g., `"/api/v1/hosts"`).
        path: String,
        /// Field in each response item to use as the submitted option value.
        value_field: String,
        /// Field in each response item to use as the human-readable label.
        label_field: String,
    },
    /// Fetch options by invoking an extension action.
    ///
    /// The frontend calls the extension action identified by `action_id` and
    /// expects the response `data` to contain an `options` array of
    /// `{ "value": "...", "label": "..." }` objects.
    Action {
        /// The action ID to invoke.
        action_id: String,
    },
}

/// A single form field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDef {
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
    /// Static options for `Select` field type.
    ///
    /// Ignored when `select_source` is also set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    /// Dynamic source for `Select` field options, loaded at form-open time.
    ///
    /// Takes precedence over `options` when both are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_source: Option<SelectSource>,
    /// Whether this field contains sensitive data (e.g., passwords, private keys).
    ///
    /// Sensitive fields are encrypted client-side using ECIES before submission
    /// and transmitted in `ExtensionRequestPayload::sensitive_params` instead of
    /// `params`. The controller never sees their plaintext.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// When `true`, the field value is a newline-separated list that
    /// serializes as a JSON array of strings. Used with `Textarea` fields
    /// to represent `Vec<String>` config values (e.g., regex patterns, node names).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub list: bool,
    /// Conditional visibility: show this field only when the controlling
    /// field's value matches one of the specified values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<VisibleWhen>,
}

impl FieldDef {
    /// Create a new field definition.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            field_type: FieldType::default(),
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
    pub fn with_type(mut self, field_type: FieldType) -> Self {
        self.field_type = field_type;
        self
    }

    /// Mark this field as sensitive (encrypted client-side via ECIES).
    pub fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }

    /// Mark this field as a newline-separated list (serialized as JSON array).
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
    pub fn with_options(mut self, options: Vec<SelectOption>) -> Self {
        self.options = options;
        self
    }

    /// Set a dynamic source for `Select` field options.
    ///
    /// When set, the frontend loads options at form-open time by calling the
    /// specified source. Takes precedence over static `options`.
    pub fn with_select_source(mut self, source: SelectSource) -> Self {
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
///
/// # Wire forward-compatibility
///
/// `Other(String)` preserves unknown field types from newer peers.
#[non_exhaustive]
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
    ///
    /// Value is submitted as a JSON array of strings. Uses `select_source` or
    /// `options` like `Select`.
    MultiSelect,
    /// Multi-line text input.
    Textarea,
    /// Boolean toggle.
    Toggle,
    /// Hidden field (not displayed, included in submission).
    Hidden,
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
            _ => {
                tracing::debug!(value = s, "received unknown FieldType from peer");
                Self::Other(s)
            }
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

/// Condition for conditional field visibility.
///
/// When present on a [`FieldDef`], the field is only shown in the UI when
/// the controlling field's current value is in the `values` list. This
/// enables tagged-enum patterns (e.g., show "username"/"password" fields
/// only when `auth_type` is `"basic"`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleWhen {
    /// Key of the controlling field.
    pub field: String,
    /// Field is visible when the controlling field's value is in this list.
    pub values: Vec<String>,
}

/// Conditional visibility for a row action in a `DataTable`.
///
/// When present on an [`ActionDef`], the action button is only rendered in
/// a table row if the specified condition on a row data field is satisfied.
///
/// # Example
///
/// ```
/// # use uptrakit_extension_framework::{ActionDef, RowCondition};
/// let action = ActionDef::new("approve-match", "Approve Match")
///     .with_row_visible_when("suggested_host_id", RowCondition::Present);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowVisibleWhen {
    /// Key of the row data field to check.
    pub field: String,
    /// The condition that must hold for the action to be visible.
    pub condition: RowCondition,
}

/// Condition type for [`RowVisibleWhen`].
///
/// # Wire forward-compatibility
///
/// `Other(String)` preserves unknown condition types from newer peers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowCondition {
    /// The field must be present and non-null (i.e., not `null`, not absent).
    Present,
    /// The field must be absent or `null`.
    Absent,
    /// An unknown condition type received from a newer peer.
    Other(String),
}

impl RowCondition {
    /// Returns the snake_case wire string for this condition.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for RowCondition {
    fn from(s: String) -> Self {
        match s.as_str() {
            "present" => Self::Present,
            "absent" => Self::Absent,
            _ => {
                tracing::debug!(value = s, "received unknown RowCondition from peer");
                Self::Other(s)
            }
        }
    }
}

impl Serialize for RowCondition {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RowCondition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(RowCondition::from)
    }
}

/// A single option in a `Select` field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    /// Value submitted when this option is selected.
    pub value: String,
    /// Human-readable label displayed in the dropdown.
    pub label: String,
}

impl SelectOption {
    /// Create a new select option.
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

// ── Wire payloads ───────────────────────────────────────────────────────────

/// Payload for `ServiceMessage::ExtensionRegister`: a service declares its UI extensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionRegisterPayload {
    /// Extension manifests provided by this service.
    pub manifests: Vec<ExtensionManifest>,
    /// Base64-encoded uncompressed P-256 public key (65 bytes) for ECIES encryption.
    ///
    /// When present, clients encrypt sensitive extension parameters with this key
    /// using the ECIES sealed-box scheme. The controller passes the ciphertext
    /// through without decryption — only this service instance can decrypt.
    ///
    /// This is per-service-instance (each instance has its own mTLS keypair).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_public_key: Option<String>,
}

impl ExtensionRegisterPayload {
    /// Create a new register payload with the given manifests.
    pub fn new(manifests: Vec<ExtensionManifest>) -> Self {
        Self {
            manifests,
            encryption_public_key: None,
        }
    }

    /// Set the ECIES encryption public key.
    pub fn with_encryption_public_key(mut self, key: impl Into<String>) -> Self {
        self.encryption_public_key = Some(key.into());
        self
    }
}

/// Payload for `ServiceMessage::ExtensionActionsRegister`: a service declares
/// its action library — a flat catalogue of [`ActionDef`] structs that can be
/// referenced by `action_id` from any extension manifest of the same source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionActionsPayload {
    /// Action definitions provided by this service.
    pub actions: Vec<ActionDef>,
}

impl ExtensionActionsPayload {
    /// Create a new actions payload.
    pub fn new(actions: Vec<ActionDef>) -> Self {
        Self { actions }
    }
}

/// Payload for `ControllerMessage::ExtensionRequest`: the controller proxies
/// an action invocation to a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionRequestPayload {
    /// Correlation ID (UUID v7) for matching request to response.
    pub request_id: String,
    /// Extension ID the action belongs to.
    pub extension_id: String,
    /// Action ID to invoke.
    pub action_id: String,
    /// Action parameters as JSON (non-sensitive fields only).
    #[serde(default)]
    pub params: serde_json::Value,
    /// ECIES sealed-box ciphertext (base64) containing sensitive parameters.
    ///
    /// Encrypted by the client using the target service's P-256 public key.
    /// The controller passes this through opaquely — it cannot decrypt.
    /// The service decrypts using its mTLS private key.
    ///
    /// Uses [`SecretString`] so the ciphertext is redacted in logs and
    /// zero-filled on drop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive_params: Option<SecretString>,
}

/// Payload for `ServiceMessage::ExtensionResponse`: a service responds to a
/// proxied action invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionResponsePayload {
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

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::Permission;

    #[test]
    fn extension_manifest_roundtrip_page() {
        let manifest = ExtensionManifest {
            id: "ssh-agent.host-management".to_string(),
            label: "SSH Host Management".to_string(),
            priority: 250,
            placement: ExtensionPlacement::Page {
                nav_section: "management".to_string(),
                icon: Some("server".to_string()),
            },
            required_permission: Permission::UpdateHosts.into(),
            targeting: ExtensionTargeting::Targeted,
            ui: ExtensionUi::DataTable {
                columns: vec![TableColumn {
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
        let roundtripped: ExtensionManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_panel() {
        let manifest = ExtensionManifest {
            id: "proxmox.lxc-panel".to_string(),
            label: "LXC Matching".to_string(),
            priority: 0,
            placement: ExtensionPlacement::Panel {
                target_page: "hosts".to_string(),
                position: PanelPosition::Below,
                tab_group: None,
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Universal,
            ui: ExtensionUi::KeyValue {
                data_action: "get-lxc-info".to_string(),
            },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        // tab_group should be omitted when None
        assert!(
            !json.contains("tab_group"),
            "tab_group should be omitted when None"
        );
        let roundtripped: ExtensionManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_panel_with_tab_group() {
        let manifest = ExtensionManifest {
            id: "notifications.webhook".to_string(),
            label: "Webhook Channels".to_string(),
            priority: 500,
            placement: ExtensionPlacement::Panel {
                target_page: "settings".to_string(),
                position: PanelPosition::Tab,
                tab_group: Some("Notification Channels".to_string()),
            },
            required_permission: "view_notifications".to_string(),
            targeting: ExtensionTargeting::Universal,
            ui: ExtensionUi::Actions {
                actions: vec!["list".to_string()],
            },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        assert!(
            json.contains(r#""tab_group":"Notification Channels"#),
            "tab_group should be present when Some"
        );
        let roundtripped: ExtensionManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_context_menu() {
        let manifest = ExtensionManifest {
            id: "ssh-agent.host-actions".to_string(),
            label: "SSH Actions".to_string(),
            priority: 100,
            placement: ExtensionPlacement::ContextMenuGroup {
                target_entity: "host".to_string(),
                group_label: "SSH Agent".to_string(),
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Targeted,
            ui: ExtensionUi::Actions {
                actions: vec!["bootstrap".to_string()],
            },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        let roundtripped: ExtensionManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_table_columns() {
        let manifest = ExtensionManifest {
            id: "ssh-agent.host-columns".to_string(),
            label: "SSH Status".to_string(),
            priority: 50,
            placement: ExtensionPlacement::TableColumns {
                target_table: "hosts".to_string(),
                columns: vec![ExtensionColumn {
                    key: "ssh_status".to_string(),
                    label: "SSH Status".to_string(),
                    data_action: "get-ssh-status".to_string(),
                }],
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Universal,
            ui: ExtensionUi::Actions { actions: vec![] },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        let roundtripped: ExtensionManifest =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn panel_position_default_is_tab() {
        assert_eq!(PanelPosition::default(), PanelPosition::Tab);
    }

    #[test]
    fn panel_position_tab_serializes_as_object() {
        let pos = PanelPosition::Tab;
        let json = serde_json::to_string(&pos).expect("serialize should succeed");
        assert_eq!(json, r#"{"type":"tab"}"#);
        let roundtripped: PanelPosition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(pos, roundtripped);
    }

    #[test]
    fn panel_position_below_serializes_as_object() {
        let pos = PanelPosition::Below;
        let json = serde_json::to_string(&pos).expect("serialize should succeed");
        assert_eq!(json, r#"{"type":"below"}"#);
        let roundtripped: PanelPosition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(pos, roundtripped);
    }

    #[test]
    fn panel_position_other_roundtrip() {
        // Other(s) serializes as {"type": s} — not {"type":"other","value":s}
        let pos = PanelPosition::Other("sidebar".to_string());
        let json = serde_json::to_string(&pos).expect("serialize should succeed");
        assert_eq!(json, r#"{"type":"sidebar"}"#);
        let roundtripped: PanelPosition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(pos, roundtripped);
    }

    #[test]
    fn panel_position_unknown_type_deserializes_to_other() {
        // A newer peer sending an unknown position type should be tolerated
        let json = r#"{"type":"floating"}"#;
        let pos: PanelPosition = serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(pos, PanelPosition::Other("floating".to_string()));
    }

    #[test]
    fn field_type_default_is_text() {
        assert_eq!(FieldType::default(), FieldType::Text);
    }

    #[test]
    fn field_type_known_variants_serialize_as_plain_string() {
        assert_eq!(
            serde_json::to_string(&FieldType::Text).unwrap(),
            r#""text""#
        );
        assert_eq!(
            serde_json::to_string(&FieldType::MultiSelect).unwrap(),
            r#""multi_select""#
        );
        assert_eq!(
            serde_json::to_string(&FieldType::Toggle).unwrap(),
            r#""toggle""#
        );
    }

    #[test]
    fn field_type_other_roundtrip() {
        let ft = FieldType::Other("color_picker".to_string());
        let json = serde_json::to_string(&ft).expect("serialize should succeed");
        assert_eq!(json, r#""color_picker""#);
        let roundtripped: FieldType =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(ft, roundtripped);
    }

    #[test]
    fn field_type_unknown_deserializes_to_other() {
        let json = r#""date_picker""#;
        let ft: FieldType = serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(ft, FieldType::Other("date_picker".to_string()));
    }

    #[test]
    fn extension_targeting_default_is_universal() {
        assert_eq!(ExtensionTargeting::default(), ExtensionTargeting::Universal);
    }

    #[test]
    fn extension_targeting_known_variants_roundtrip() {
        let variants = [ExtensionTargeting::Universal, ExtensionTargeting::Targeted];
        for v in &variants {
            let json = serde_json::to_string(v).expect("serialize should succeed");
            let roundtripped: ExtensionTargeting =
                serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(v, &roundtripped);
        }
    }

    #[test]
    fn extension_targeting_other_roundtrip() {
        let t = ExtensionTargeting::Other("scoped".to_string());
        let json = serde_json::to_string(&t).expect("serialize should succeed");
        assert_eq!(json, r#""scoped""#);
        let roundtripped: ExtensionTargeting =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(t, roundtripped);
    }

    #[test]
    fn extension_targeting_unknown_deserializes_to_other() {
        let json = r#""tenant_specific""#;
        let t: ExtensionTargeting = serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(t, ExtensionTargeting::Other("tenant_specific".to_string()));
    }

    #[test]
    fn row_condition_known_variants_roundtrip() {
        let variants = [RowCondition::Present, RowCondition::Absent];
        for v in &variants {
            let json = serde_json::to_string(v).expect("serialize should succeed");
            let roundtripped: RowCondition =
                serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(v, &roundtripped);
        }
    }

    #[test]
    fn row_condition_other_roundtrip() {
        let c = RowCondition::Other("non_empty".to_string());
        let json = serde_json::to_string(&c).expect("serialize should succeed");
        assert_eq!(json, r#""non_empty""#);
        let roundtripped: RowCondition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(c, roundtripped);
    }

    #[test]
    fn row_condition_unknown_deserializes_to_other() {
        let json = r#""matches_regex""#;
        let c: RowCondition = serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(c, RowCondition::Other("matches_regex".to_string()));
    }

    #[test]
    fn wizard_step_roundtrip() {
        let step = WizardStep {
            step_id: "step1".to_string(),
            label: "Enter Details".to_string(),
            form: FormDef {
                fields: vec![FieldDef {
                    key: "host".to_string(),
                    label: "Host".to_string(),
                    field_type: FieldType::Text,
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
        };

        let json = serde_json::to_string(&step).expect("serialize should succeed");
        let roundtripped: WizardStep =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(step, roundtripped);
    }

    #[test]
    fn action_ui_wizard_roundtrip() {
        let ui = ActionUi::Wizard {
            steps: vec![WizardStep {
                step_id: "s1".to_string(),
                label: "Step 1".to_string(),
                form: FormDef {
                    fields: vec![],
                    pre_load_action: None,
                    footer_actions: vec![],
                },
                submit_action: None,
            }],
        };

        let json = serde_json::to_string(&ui).expect("serialize should succeed");
        let roundtripped: ActionUi =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(ui, roundtripped);
    }

    #[test]
    fn select_option_roundtrip() {
        let opt = SelectOption {
            value: "opt1".to_string(),
            label: "Option 1".to_string(),
        };
        let json = serde_json::to_string(&opt).expect("serialize should succeed");
        let roundtripped: SelectOption =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(opt, roundtripped);
    }

    #[test]
    fn extension_register_payload_roundtrip() {
        let payload = ExtensionRegisterPayload {
            manifests: vec![ExtensionManifest {
                id: "test.ext".to_string(),
                label: "Test Extension".to_string(),
                priority: 500,
                placement: ExtensionPlacement::Page {
                    nav_section: "test".to_string(),
                    icon: None,
                },
                required_permission: String::new(),
                targeting: ExtensionTargeting::Universal,
                ui: ExtensionUi::KeyValue {
                    data_action: "get-data".to_string(),
                },
            }],
            encryption_public_key: None,
        };

        let json = serde_json::to_string(&payload).expect("serialize should succeed");
        let roundtripped: ExtensionRegisterPayload =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(payload, roundtripped);
    }

    #[test]
    fn extension_actions_payload_roundtrip() {
        let payload = ExtensionActionsPayload {
            actions: vec![
                ActionDef::new("list-hosts", "List Hosts"),
                ActionDef::new("bootstrap", "Bootstrap Host")
                    .with_permission(Permission::UpdateHosts)
                    .with_timeout(120),
            ],
        };

        let json = serde_json::to_string(&payload).expect("serialize should succeed");
        let roundtripped: ExtensionActionsPayload =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(payload, roundtripped);
    }

    #[test]
    fn extension_request_payload_roundtrip() {
        let payload = ExtensionRequestPayload {
            request_id: "req-123".to_string(),
            extension_id: "test.ext".to_string(),
            action_id: "do-thing".to_string(),
            params: serde_json::json!({"key": "value"}),
            sensitive_params: None,
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
        let action = ActionDef::new("delete-all", "Delete All")
            .with_permission(Permission::UpdateHosts)
            .destructive();

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(json.contains(r#""destructive":true"#));
        let roundtripped: ActionDef =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(action, roundtripped);
    }

    #[test]
    fn optional_fields_omitted_when_default() {
        let manifest = ExtensionManifest {
            id: "test".to_string(),
            label: "Test".to_string(),
            priority: 0,
            placement: ExtensionPlacement::Page {
                nav_section: "test".to_string(),
                icon: None,
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Universal,
            ui: ExtensionUi::Actions { actions: vec![] },
        };

        let json = serde_json::to_string(&manifest).expect("serialize should succeed");
        // required_permission should be omitted when empty
        assert!(!json.contains("required_permission"));
        // icon should be omitted when None
        assert!(!json.contains("icon"));
    }

    #[test]
    fn select_source_rest_api_roundtrip() {
        let source = SelectSource::RestApi {
            path: "/api/v1/hosts".to_string(),
            value_field: "id".to_string(),
            label_field: "friendly_name".to_string(),
        };
        let json = serde_json::to_string(&source).expect("serialize should succeed");
        assert!(json.contains(r#""type":"rest_api""#));
        let roundtripped: SelectSource =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(source, roundtripped);
    }

    #[test]
    fn field_def_with_select_source_roundtrip() {
        let field = FieldDef::new("host_id", "Host")
            .with_type(FieldType::Select)
            .required()
            .with_select_source(SelectSource::RestApi {
                path: "/api/v1/hosts".to_string(),
                value_field: "id".to_string(),
                label_field: "friendly_name".to_string(),
            });
        let json = serde_json::to_string(&field).expect("serialize should succeed");
        assert!(json.contains("select_source"));
        assert!(!json.contains("\"options\"")); // empty options omitted
        let roundtripped: FieldDef =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(field, roundtripped);
    }

    #[test]
    fn form_with_select_field() {
        let form = FormDef {
            fields: vec![FieldDef {
                key: "region".to_string(),
                label: "Region".to_string(),
                field_type: FieldType::Select,
                required: true,
                placeholder: None,
                help_text: None,
                default_value: None,
                options: vec![
                    SelectOption {
                        value: "us-east".to_string(),
                        label: "US East".to_string(),
                    },
                    SelectOption {
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
        let roundtripped: FormDef =
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
        let ui = ExtensionUi::DataTable {
            columns: vec![],
            data_action: "list".to_string(),
            row_actions: vec!["edit".to_string(), "delete".to_string()],
            primary_actions: vec!["create".to_string()],
            context_selector: None,
            default_per_page: None,
        };

        let json = serde_json::to_string(&ui).expect("serialize should succeed");
        assert!(json.contains(r#""row_actions":["edit","delete"]"#));
        let roundtripped: ExtensionUi =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(ui, roundtripped);
    }

    #[test]
    fn context_selector_add_action_is_string_ref() {
        let cs = ContextSelectorDef::new(
            "config_id",
            "Configuration",
            ContextSelectorSource::PluginConfigs {
                plugin_type: "infrastructure_proxmox".to_string(),
            },
        )
        .with_add_action("add-config")
        .with_empty_message("No configurations found.");

        let json = serde_json::to_string(&cs).expect("serialize should succeed");
        assert!(json.contains(r#""add_action":"add-config""#));
        let roundtripped: ContextSelectorDef =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(cs, roundtripped);
    }

    #[test]
    fn select_option_new() {
        let opt = SelectOption::new("val", "Label");
        assert_eq!(opt.value, "val");
        assert_eq!(opt.label, "Label");
    }

    #[test]
    fn wizard_step_new_with_submit_action() {
        let step =
            WizardStep::new("s1", "Step 1", FormDef::new(vec![])).with_submit_action("validate");
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
        let field = FieldDef::new("name", "Name").with_default_value("default");
        assert_eq!(
            field.default_value,
            Some(serde_json::Value::String("default".to_string()))
        );
    }

    #[test]
    fn field_def_with_options() {
        let field = FieldDef::new("region", "Region")
            .with_type(FieldType::Select)
            .with_options(vec![
                SelectOption::new("us", "US"),
                SelectOption::new("eu", "EU"),
            ]);
        assert_eq!(field.options.len(), 2);
        assert_eq!(field.options[0].value, "us");
    }

    #[test]
    fn select_source_action_roundtrip() {
        let source = SelectSource::Action {
            action_id: "list-discovered-guests".to_string(),
        };
        let json = serde_json::to_string(&source).expect("serialize should succeed");
        assert!(json.contains(r#""type":"action""#));
        let roundtripped: SelectSource =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(source, roundtripped);
    }

    #[test]
    fn register_payload_builder() {
        let payload = ExtensionRegisterPayload::new(vec![]).with_encryption_public_key("key123");
        assert!(payload.manifests.is_empty());
        assert_eq!(payload.encryption_public_key.as_deref(), Some("key123"));
    }

    #[test]
    fn actions_payload_builder() {
        let payload = ExtensionActionsPayload::new(vec![ActionDef::new("list", "List")]);
        assert_eq!(payload.actions.len(), 1);
        assert_eq!(payload.actions[0].action_id, "list");
    }

    #[test]
    fn visible_when_roundtrip() {
        let field = FieldDef::new("auth_username", "Username")
            .with_visible_when("auth_type", vec!["basic".to_string()]);
        let json = serde_json::to_string(&field).expect("serialize should succeed");
        assert!(json.contains("visible_when"));
        let roundtripped: FieldDef =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(field, roundtripped);
        let vw = roundtripped.visible_when.expect("should have visible_when");
        assert_eq!(vw.field, "auth_type");
        assert_eq!(vw.values, vec!["basic"]);
    }

    #[test]
    fn visible_when_omitted_when_none() {
        let field = FieldDef::new("name", "Name");
        let json = serde_json::to_string(&field).expect("serialize should succeed");
        assert!(!json.contains("visible_when"));
    }

    #[test]
    fn row_visible_when_roundtrip() {
        let action = ActionDef::new("approve-match", "Approve Match")
            .with_row_visible_when("suggested_host_id", RowCondition::Present);

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        let roundtripped: ActionDef =
            serde_json::from_str(&json).expect("deserialize should succeed");

        let rvw = roundtripped.row_visible_when.expect("should be set");
        assert_eq!(rvw.field, "suggested_host_id");
        assert_eq!(rvw.condition, RowCondition::Present);
    }

    #[test]
    fn row_visible_when_absent_condition() {
        let action = ActionDef::new("unmatch", "Remove Match")
            .with_row_visible_when("matched_host", RowCondition::Absent);

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        let roundtripped: ActionDef =
            serde_json::from_str(&json).expect("deserialize should succeed");

        let rvw = roundtripped.row_visible_when.expect("should be set");
        assert_eq!(rvw.field, "matched_host");
        assert_eq!(rvw.condition, RowCondition::Absent);
    }

    #[test]
    fn row_visible_when_omitted_when_none() {
        let action = ActionDef::new("match", "Manual Match");
        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(!json.contains("row_visible_when"));
    }

    #[test]
    fn confirm_entity_field_roundtrip() {
        let action = ActionDef::new("remove-host", "Remove Host")
            .destructive()
            .with_confirm_entity_field("name");

        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(json.contains(r#""confirm_entity_field":"name""#));
        let roundtripped: ActionDef =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(roundtripped.confirm_entity_field.as_deref(), Some("name"));
    }

    #[test]
    fn confirm_entity_field_omitted_when_none() {
        let action = ActionDef::new("list", "List Items");
        let json = serde_json::to_string(&action).expect("serialize should succeed");
        assert!(!json.contains("confirm_entity_field"));
    }
}
