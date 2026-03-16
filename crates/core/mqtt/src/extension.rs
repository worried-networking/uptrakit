//! UI extension manifest and action library for the MQTT service.
//!
//! The MQTT service exposes a settings page that lets tenants create, edit,
//! and delete MQTT client connections without accessing the database or CLI.
//! Configurations are stored via the Service Config Store using
//! `"clients.{uuid}"` keys and delivered back through `ServiceConfigDelivery`
//! and `ServiceConfigUpdated` messages.
//!
//! ## Action routing
//!
//! All actions use the default `ExtensionTargeting::Universal` — the
//! controller picks any running MQTT service instance to handle the request.
//!
//! ## Permission
//!
//! All write actions require `UpdateSystemServices`.

use uptrakit_internal_wire::{
    ServiceMessage,
    extension::{
        ActionDef, ActionUi, ExtensionManifest, ExtensionPlacement, ExtensionRegisterPayload,
        ExtensionRequestPayload, ExtensionResponsePayload, ExtensionUi, FieldDef, FieldType,
        FormDef, SelectOption, TableColumn,
    },
};
use uptrakit_service_sdk::{ControllerConnection, LoopError, LoopResult};

/// Extension and action IDs — kept as constants to avoid magic strings.
pub(crate) const EXT_ID: &str = "mqtt.clients";
pub(crate) const ACTION_LIST: &str = "mqtt.list-clients";
pub(crate) const ACTION_CREATE: &str = "mqtt.create-client";
pub(crate) const ACTION_EDIT: &str = "mqtt.edit-client";
pub(crate) const ACTION_DELETE: &str = "mqtt.delete-client";

/// Build the `ExtensionRegister` payload for the MQTT clients settings page.
pub(crate) fn build_register_payload() -> ExtensionRegisterPayload {
    let manifest = ExtensionManifest::new(
        EXT_ID,
        "MQTT Clients",
        100,
        ExtensionPlacement::Page {
            nav_section: "system".to_string(),
            icon: Some("wifi".to_string()),
        },
        ExtensionUi::DataTable {
            columns: vec![
                TableColumn::new("client_id", "Client ID").sortable(),
                TableColumn::new("host", "Broker Host").sortable(),
                TableColumn::new("transport", "Transport").sortable(),
                TableColumn::new("enabled", "Enabled"),
                TableColumn::new("ha_discovery", "HA Discovery"),
            ],
            data_action: ACTION_LIST.to_string(),
            row_actions: vec![ACTION_EDIT.to_string(), ACTION_DELETE.to_string()],
            primary_actions: vec![ACTION_CREATE.to_string()],
            context_selector: None,
            default_per_page: None,
        },
    )
    .with_permission("update_system_services");

    ExtensionRegisterPayload::new(vec![manifest])
}

/// Build the action library for `ExtensionActionsRegister`.
pub(crate) fn build_actions() -> Vec<ActionDef> {
    let perm = "update_system_services";

    vec![
        ActionDef::new(ACTION_LIST, "List MQTT Clients"),
        ActionDef::new(ACTION_CREATE, "Add MQTT Client")
            .with_permission(perm)
            .with_ui(ActionUi::Form(client_form(false))),
        ActionDef::new(ACTION_EDIT, "Edit MQTT Client")
            .with_permission(perm)
            .with_ui(ActionUi::Form(client_form(true))),
        ActionDef::new(ACTION_DELETE, "Delete MQTT Client")
            .with_permission(perm)
            .destructive()
            .with_confirm_entity_field("client_id"),
    ]
}

/// Build the form definition for creating or editing an MQTT client.
///
/// When `pre_load` is `true`, a `pre_load_action` pointing to `ACTION_EDIT`
/// is set so the form opens pre-populated with the existing client data.
fn client_form(pre_load: bool) -> FormDef {
    let fields = vec![
        FieldDef::new("client_id", "MQTT Client ID")
            .required()
            .with_placeholder("my-uptrakit-client")
            .with_help_text("Unique identifier sent to the MQTT broker."),
        FieldDef::new("host", "Broker Host")
            .required()
            .with_placeholder("mqtt.example.com"),
        FieldDef::new("port", "Broker Port")
            .with_type(FieldType::Number)
            .with_placeholder("0")
            .with_help_text("0 = use the default port for the selected transport."),
        FieldDef::new("transport", "Transport")
            .with_type(FieldType::Select)
            .with_options(vec![
                SelectOption::new("tcp", "TCP (plain)"),
                SelectOption::new("tls", "TLS"),
            ])
            .required(),
        FieldDef::new("topic_prefix", "Topic Prefix")
            .required()
            .with_placeholder("uptrakit")
            .with_help_text("Base topic path for all published messages."),
        FieldDef::new("username", "Username"),
        FieldDef::new("password", "Password")
            .with_type(FieldType::Password)
            .sensitive(),
        FieldDef::new("ca_pem", "Custom CA Certificate (PEM)")
            .with_type(FieldType::Textarea)
            .sensitive()
            .with_help_text("Optional PEM-encoded CA certificate for broker TLS verification."),
        FieldDef::new("ha_discovery", "Enable HA Discovery")
            .with_type(FieldType::Toggle)
            .with_help_text("Publish Home Assistant MQTT discovery topics."),
        FieldDef::new("ha_discovery_prefix", "HA Discovery Prefix")
            .with_placeholder("homeassistant")
            .with_help_text("Topic prefix for HA discovery messages."),
        FieldDef::new("enabled", "Enabled")
            .with_type(FieldType::Toggle)
            .with_default_value(true),
    ];

    let mut form = FormDef::new(fields);
    if pre_load {
        form = form.with_pre_load_action(ACTION_EDIT);
    }
    form
}

/// Handle a list action request.
///
/// Builds a JSON summary of all current MQTT client configurations and returns
/// a success response. Returns `None` if the action ID does not match.
pub(crate) fn handle_list_action(
    request: &ExtensionRequestPayload,
    configs: &[crate::client_manager::ParsedMqttClientConfig],
) -> Option<ExtensionResponsePayload> {
    if request.action_id != ACTION_LIST {
        return None;
    }

    let items: Vec<serde_json::Value> = configs
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.mqtt_client_id.to_string(),
                "client_id": c.client_id,
                "host": c.host,
                "transport": c.transport.as_str(),
                "enabled": c.enabled,
                "ha_discovery": c.ha_discovery,
                "topic_prefix": c.topic_prefix,
            })
        })
        .collect();

    Some(ExtensionResponsePayload {
        request_id: request.request_id.clone(),
        success: true,
        data: serde_json::json!({ "items": items }),
        error: None,
    })
}

/// Send an error response back to the controller for an unhandled or failed action.
pub(crate) async fn send_error_response(
    conn: &mut ControllerConnection,
    request_id: String,
    message: impl Into<String>,
) -> LoopResult<()> {
    use rootcause::prelude::*;
    let response = ExtensionResponsePayload {
        request_id,
        success: false,
        data: serde_json::Value::Null,
        error: Some(message.into()),
    };
    conn.send(ServiceMessage::ExtensionResponse(response))
        .await
        .map_err(|e| {
            report!(LoopError::Other(format!(
                "failed to send extension error response: {e}"
            )))
        })
}
