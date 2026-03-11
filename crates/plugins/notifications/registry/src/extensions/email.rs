//! Email channel extension manifest and actions.

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionPlacement, ExtensionUi,
    FieldDef, FieldType, FormDef, PanelPosition, SelectOption, TableColumn,
};

/// Extension manifest for the email notification channel tab.
pub fn manifest() -> ExtensionManifest {
    ExtensionManifest::new(
        "notifications.email",
        "Email Channels",
        502,
        ExtensionPlacement::Panel {
            target_page: "settings".to_string(),
            position: PanelPosition::Tab,
            tab_group: Some("Notification Channels".to_string()),
        },
        ExtensionUi::DataTable {
            columns: vec![
                TableColumn::new("name", "Name"),
                TableColumn::new("to_addresses", "Recipients"),
                TableColumn::new("enabled", "Enabled"),
                TableColumn::new("created_at", "Created"),
            ],
            data_action: "list".to_string(),
            row_actions: vec!["edit".to_string(), "test".to_string(), "delete".to_string()],
            primary_actions: vec!["create".to_string(), "configure_smtp".to_string()],
            context_selector: None,
            default_per_page: Some(20),
        },
    )
    .with_permission("view_notifications")
}

/// Action definitions for email channels.
pub fn actions() -> Vec<ActionDef> {
    vec![
        // List — data-only, no UI
        ActionDef::new("list", "List"),
        // Create
        ActionDef::new("create", "Add Email Channel")
            .with_permission("manage_notifications")
            .with_ui(ActionUi::Form(FormDef::new(vec![
                FieldDef::new("name", "Name").required(),
                FieldDef::new("to_addresses", "Recipients")
                    .required()
                    .with_type(FieldType::Textarea)
                    .with_placeholder("user@example.com\nadmin@example.com")
                    .with_help_text("One email address per line"),
                FieldDef::new("enabled", "Enabled")
                    .with_type(FieldType::Toggle)
                    .with_default_value(serde_json::json!("true")),
            ])))
            .with_api_submit(
                ApiSubmitDef::new(
                    "POST",
                    "/api/v1/notifications/channels",
                    serde_json::json!({
                        "name": "{{name}}",
                        "channel_type": "email",
                        "config": {
                            "to_addresses": "{{to_addresses}}"
                        },
                        "enabled": "{{enabled:bool}}"
                    }),
                )
                .with_response_id_field("id"),
            ),
        // Edit
        ActionDef::new("edit", "Edit")
            .with_permission("manage_notifications")
            .with_ui(ActionUi::Form(FormDef::new(vec![
                FieldDef::new("id", "ID").with_type(FieldType::Hidden),
                FieldDef::new("name", "Name").required(),
                FieldDef::new("to_addresses", "Recipients")
                    .required()
                    .with_type(FieldType::Textarea)
                    .with_placeholder("user@example.com\nadmin@example.com")
                    .with_help_text("One email address per line"),
                FieldDef::new("enabled", "Enabled")
                    .with_type(FieldType::Toggle)
                    .with_default_value(serde_json::json!("true")),
            ])))
            .with_api_submit(ApiSubmitDef::new(
                "PUT",
                "/api/v1/notifications/channels/{{id}}",
                serde_json::json!({
                    "name": "{{name}}",
                    "config": {
                        "to_addresses": "{{to_addresses}}"
                    },
                    "enabled": "{{enabled:bool}}"
                }),
            )),
        // Test
        ActionDef::new("test", "Test")
            .with_permission("manage_notifications")
            .with_api_submit(ApiSubmitDef::new(
                "POST",
                "/api/v1/notifications/channels/{{id}}/test",
                serde_json::json!({}),
            )),
        // Delete
        ActionDef::new("delete", "Delete")
            .with_permission("manage_notifications")
            .destructive()
            .with_confirm_entity_field("name")
            .with_api_submit(ApiSubmitDef::new(
                "DELETE",
                "/api/v1/notifications/channels/{{id}}",
                serde_json::json!({}),
            )),
        // Configure SMTP — form submitted through extension invoke
        ActionDef::new("configure_smtp", "Configure SMTP")
            .with_permission("manage_notifications")
            .with_ui(ActionUi::Form(
                FormDef::new(vec![
                    FieldDef::new("host", "SMTP Host").with_placeholder("smtp.example.com"),
                    FieldDef::new("port", "Port")
                        .with_type(FieldType::Number)
                        .with_default_value(serde_json::json!("587")),
                    FieldDef::new("tls_mode", "TLS Mode")
                        .with_type(FieldType::Select)
                        .with_options(vec![
                            SelectOption::new("starttls", "STARTTLS (port 587)"),
                            SelectOption::new("tls", "TLS (port 465)"),
                            SelectOption::new("none", "None (port 25)"),
                        ])
                        .with_default_value(serde_json::json!("starttls")),
                    FieldDef::new("from_address", "From Address")
                        .with_placeholder("noreply@example.com"),
                    FieldDef::new("from_name", "From Name")
                        .with_placeholder("Uptrakit Notifications"),
                    FieldDef::new("username", "Username").with_placeholder("SMTP username"),
                    FieldDef::new("password", "Password")
                        .with_type(FieldType::Password)
                        .with_help_text("Leave empty to keep current password"),
                ])
                .with_pre_load_action("get_smtp"),
            )),
        // get_smtp — data-only action for pre-loading SMTP settings
        ActionDef::new("get_smtp", "Get SMTP Settings"),
        // save_smtp — invoked by the configure_smtp form submit
        ActionDef::new("save_smtp", "Save SMTP Settings").with_permission("manage_notifications"),
    ]
}
