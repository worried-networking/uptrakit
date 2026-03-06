//! UI extension manifest types for the wire protocol.
//!
//! Extensions allow plugins and connected services to dynamically contribute
//! UI elements (pages, panels, context menu groups, table columns) and expose
//! actions that the controller proxies to the appropriate service instance.
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
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionManifest {
    /// Unique extension identifier (e.g., `"ssh-agent.host-management"`).
    pub id: String,
    /// Human-readable name displayed in the UI.
    pub label: String,
    /// Where this extension appears in the UI.
    pub placement: ExtensionPlacement,
    /// Permission required to see and use this extension (e.g., `"manage_hosts"`).
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
        placement: ExtensionPlacement,
        ui: ExtensionUi,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
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
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionTargeting {
    /// Any connected instance of the source service type can handle actions.
    /// The controller picks one automatically.
    #[default]
    Universal,
    /// Actions must be routed to a specific service instance selected by the user.
    Targeted,
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
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

// ── UI definitions ──────────────────────────────────────────────────────────

/// Schema-driven UI definition for an extension.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtensionUi {
    /// Data table with columns, row actions, and primary actions.
    DataTable {
        /// Column definitions.
        columns: Vec<TableColumn>,
        /// Action to invoke to fetch table data.
        data_action: String,
        /// Actions available for each row (context menu).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        row_actions: Vec<ActionDef>,
        /// Primary actions (buttons above the table).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        primary_actions: Vec<ActionDef>,
    },
    /// Input form.
    Form(FormDef),
    /// Read-only key-value display.
    KeyValue {
        /// Action to invoke to fetch data.
        data_action: String,
    },
    /// List of actions (used for `ContextMenuGroup` placement).
    Actions {
        /// Action definitions.
        actions: Vec<ActionDef>,
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
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDef {
    /// Action identifier (unique within the extension).
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

/// A form definition with typed fields.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDef {
    /// Ordered list of form fields.
    pub fields: Vec<FieldDef>,
}

impl FormDef {
    /// Create a new form definition.
    pub fn new(fields: Vec<FieldDef>) -> Self {
        Self { fields }
    }
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
    /// Options for `Select` field type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    /// Whether this field contains sensitive data (e.g., passwords, private keys).
    ///
    /// Sensitive fields are encrypted client-side using ECIES before submission
    /// and transmitted in `ExtensionRequestPayload::sensitive_params` instead of
    /// `params`. The controller never sees their plaintext.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
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
            sensitive: false,
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
}

/// Input field type.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Multi-line text input.
    Textarea,
    /// Boolean toggle.
    Toggle,
    /// Hidden field (not displayed, included in submission).
    Hidden,
    /// Forward-compatible catch-all for unknown field types.
    Other(String),
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

    #[test]
    fn extension_manifest_roundtrip_page() {
        let manifest = ExtensionManifest {
            id: "ssh-agent.host-management".to_string(),
            label: "SSH Host Management".to_string(),
            placement: ExtensionPlacement::Page {
                nav_section: "management".to_string(),
                icon: Some("server".to_string()),
            },
            required_permission: "manage_hosts".to_string(),
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
            },
        };

        let json = serde_json::to_string(&manifest).unwrap();
        let roundtripped: ExtensionManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_panel() {
        let manifest = ExtensionManifest {
            id: "proxmox.lxc-panel".to_string(),
            label: "LXC Matching".to_string(),
            placement: ExtensionPlacement::Panel {
                target_page: "hosts".to_string(),
                position: PanelPosition::Below,
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Universal,
            ui: ExtensionUi::KeyValue {
                data_action: "get-lxc-info".to_string(),
            },
        };

        let json = serde_json::to_string(&manifest).unwrap();
        let roundtripped: ExtensionManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_context_menu() {
        let manifest = ExtensionManifest {
            id: "ssh-agent.host-actions".to_string(),
            label: "SSH Actions".to_string(),
            placement: ExtensionPlacement::ContextMenuGroup {
                target_entity: "host".to_string(),
                group_label: "SSH Agent".to_string(),
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Targeted,
            ui: ExtensionUi::Actions {
                actions: vec![ActionDef {
                    action_id: "bootstrap".to_string(),
                    label: "Bootstrap Host".to_string(),
                    ui: Some(ActionUi::Form(FormDef {
                        fields: vec![FieldDef {
                            key: "username".to_string(),
                            label: "Username".to_string(),
                            field_type: FieldType::Text,
                            required: true,
                            placeholder: Some("root".to_string()),
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            sensitive: false,
                        }],
                    })),
                    permission: String::new(),
                    destructive: false,
                    timeout_seconds: Some(60),
                }],
            },
        };

        let json = serde_json::to_string(&manifest).unwrap();
        let roundtripped: ExtensionManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn extension_manifest_roundtrip_table_columns() {
        let manifest = ExtensionManifest {
            id: "ssh-agent.host-columns".to_string(),
            label: "SSH Status".to_string(),
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

        let json = serde_json::to_string(&manifest).unwrap();
        let roundtripped: ExtensionManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, roundtripped);
    }

    #[test]
    fn panel_position_default_is_tab() {
        assert_eq!(PanelPosition::default(), PanelPosition::Tab);
    }

    #[test]
    fn panel_position_other_roundtrip() {
        let pos = PanelPosition::Other("sidebar".to_string());
        let json = serde_json::to_string(&pos).unwrap();
        let roundtripped: PanelPosition = serde_json::from_str(&json).unwrap();
        assert_eq!(pos, roundtripped);
    }

    #[test]
    fn field_type_default_is_text() {
        assert_eq!(FieldType::default(), FieldType::Text);
    }

    #[test]
    fn field_type_other_roundtrip() {
        let ft = FieldType::Other("color_picker".to_string());
        let json = serde_json::to_string(&ft).unwrap();
        let roundtripped: FieldType = serde_json::from_str(&json).unwrap();
        assert_eq!(ft, roundtripped);
    }

    #[test]
    fn extension_targeting_default_is_universal() {
        assert_eq!(ExtensionTargeting::default(), ExtensionTargeting::Universal);
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
                    sensitive: false,
                }],
            },
            submit_action: Some("validate-host".to_string()),
        };

        let json = serde_json::to_string(&step).unwrap();
        let roundtripped: WizardStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, roundtripped);
    }

    #[test]
    fn action_ui_wizard_roundtrip() {
        let ui = ActionUi::Wizard {
            steps: vec![WizardStep {
                step_id: "s1".to_string(),
                label: "Step 1".to_string(),
                form: FormDef { fields: vec![] },
                submit_action: None,
            }],
        };

        let json = serde_json::to_string(&ui).unwrap();
        let roundtripped: ActionUi = serde_json::from_str(&json).unwrap();
        assert_eq!(ui, roundtripped);
    }

    #[test]
    fn select_option_roundtrip() {
        let opt = SelectOption {
            value: "opt1".to_string(),
            label: "Option 1".to_string(),
        };
        let json = serde_json::to_string(&opt).unwrap();
        let roundtripped: SelectOption = serde_json::from_str(&json).unwrap();
        assert_eq!(opt, roundtripped);
    }

    #[test]
    fn extension_register_payload_roundtrip() {
        let payload = ExtensionRegisterPayload {
            manifests: vec![ExtensionManifest {
                id: "test.ext".to_string(),
                label: "Test Extension".to_string(),
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

        let json = serde_json::to_string(&payload).unwrap();
        let roundtripped: ExtensionRegisterPayload = serde_json::from_str(&json).unwrap();
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

        let json = serde_json::to_string(&payload).unwrap();
        let roundtripped: ExtensionRequestPayload = serde_json::from_str(&json).unwrap();
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

        let json = serde_json::to_string(&payload).unwrap();
        assert!(!json.contains("error"));
        let roundtripped: ExtensionResponsePayload = serde_json::from_str(&json).unwrap();
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

        let json = serde_json::to_string(&payload).unwrap();
        let roundtripped: ExtensionResponsePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, roundtripped);
    }

    #[test]
    fn destructive_action_serialization() {
        let action = ActionDef {
            action_id: "delete-all".to_string(),
            label: "Delete All".to_string(),
            ui: None,
            permission: "manage_hosts".to_string(),
            destructive: true,
            timeout_seconds: None,
        };

        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""destructive":true"#));
        let roundtripped: ActionDef = serde_json::from_str(&json).unwrap();
        assert_eq!(action, roundtripped);
    }

    #[test]
    fn optional_fields_omitted_when_default() {
        let manifest = ExtensionManifest {
            id: "test".to_string(),
            label: "Test".to_string(),
            placement: ExtensionPlacement::Page {
                nav_section: "test".to_string(),
                icon: None,
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Universal,
            ui: ExtensionUi::Actions { actions: vec![] },
        };

        let json = serde_json::to_string(&manifest).unwrap();
        // required_permission should be omitted when empty
        assert!(!json.contains("required_permission"));
        // icon should be omitted when None
        assert!(!json.contains("icon"));
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
                sensitive: false,
            }],
        };

        let json = serde_json::to_string(&form).unwrap();
        let roundtripped: FormDef = serde_json::from_str(&json).unwrap();
        assert_eq!(form, roundtripped);
    }
}
