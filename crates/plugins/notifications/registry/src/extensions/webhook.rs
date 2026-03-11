//! Webhook channel extension manifest and actions.

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionPlacement, ExtensionUi,
    FieldDef, FieldType, FormDef, PanelPosition, TableColumn,
};

/// Extension manifest for the webhook notification channel tab.
pub fn manifest() -> ExtensionManifest {
    ExtensionManifest::new(
        "notifications.webhook",
        "Webhook Channels",
        500,
        ExtensionPlacement::Panel {
            target_page: "settings".to_string(),
            position: PanelPosition::Tab,
            tab_group: Some("Notification Channels".to_string()),
        },
        ExtensionUi::DataTable {
            columns: vec![
                TableColumn::new("name", "Name"),
                TableColumn::new("url", "URL"),
                TableColumn::new("enabled", "Enabled"),
                TableColumn::new("created_at", "Created"),
            ],
            data_action: "list".to_string(),
            row_actions: vec!["edit".to_string(), "test".to_string(), "delete".to_string()],
            primary_actions: vec!["create".to_string()],
            context_selector: None,
            default_per_page: Some(20),
        },
    )
    .with_permission("view_notifications")
}

/// Action definitions for webhook channels.
pub fn actions() -> Vec<ActionDef> {
    vec![
        // List — data-only, no UI
        ActionDef::new("list", "List"),
        // Create
        ActionDef::new("create", "Add Webhook")
            .with_permission("manage_notifications")
            .with_ui(ActionUi::Form(FormDef::new(vec![
                FieldDef::new("name", "Name").required(),
                FieldDef::new("url", "URL")
                    .required()
                    .with_placeholder("https://example.com/webhook"),
                FieldDef::new("secret", "Secret")
                    .with_type(FieldType::Password)
                    .sensitive()
                    .with_help_text("Optional HMAC secret for request signing"),
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
                        "channel_type": "webhook",
                        "config": {
                            "url": "{{url}}",
                            "secret": "{{secret}}"
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
                FieldDef::new("url", "URL")
                    .required()
                    .with_placeholder("https://example.com/webhook"),
                FieldDef::new("secret", "Secret")
                    .with_type(FieldType::Password)
                    .sensitive()
                    .with_help_text("Leave unchanged to keep current secret"),
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
                        "url": "{{url}}",
                        "secret": "{{secret}}"
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
    ]
}
