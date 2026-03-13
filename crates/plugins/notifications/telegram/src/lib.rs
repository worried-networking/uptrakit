//! Telegram notification plugin.
//!
//! Sends messages to a configured Telegram chat via the Bot API. Renders
//! action buttons as inline keyboard buttons.

use async_trait::async_trait;
use rootcause::prelude::*;

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionPlacement, ExtensionUi,
    FieldDef, FieldType, FormDef, PanelPosition, TableColumn,
};
use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPlugin, NotificationPluginError, Result, escape_html,
};

/// Telegram notification plugin using the Bot API.
///
/// Sends messages to a configured chat via `sendMessage`. When the message
/// includes actions, they are rendered as inline keyboard buttons.
pub struct TelegramPlugin {
    http: reqwest::Client,
}

impl TelegramPlugin {
    /// Create a new Telegram plugin with a pre-configured HTTP client.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationPluginError::HttpClientBuild`] if the HTTP client
    /// cannot be constructed.
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| report!(NotificationPluginError::HttpClientBuild(e.to_string())))?;
        Ok(Self { http })
    }
}

#[async_trait]
impl NotificationPlugin for TelegramPlugin {
    fn channel_type(&self) -> &'static str {
        "telegram"
    }

    async fn deliver(&self, config: &serde_json::Value, message: &DeliveryMessage) -> Result<()> {
        let bot_token = config["bot_token"].as_str().ok_or_else(|| {
            report!(NotificationPluginError::InvalidConfig(
                "missing 'bot_token'".to_string()
            ))
        })?;
        let chat_id = config["chat_id"].as_str().ok_or_else(|| {
            report!(NotificationPluginError::InvalidConfig(
                "missing 'chat_id'".to_string()
            ))
        })?;

        let text = message.body_html.as_deref().unwrap_or(&message.body);

        // Build the full message with a bold title.
        let full_text = format!("<b>{}</b>\n\n{}", escape_html(&message.title), text);

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": full_text,
            "parse_mode": "HTML",
            "disable_web_page_preview": true,
        });

        // Add inline keyboard if there are actions.
        if !message.actions.is_empty() {
            let buttons: Vec<serde_json::Value> = message
                .actions
                .iter()
                .map(|a| {
                    serde_json::json!([{
                        "text": a.label,
                        "callback_data": a.token,
                    }])
                })
                .collect();

            body["reply_markup"] = serde_json::json!({
                "inline_keyboard": buttons,
            });
        }

        let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| report!(NotificationPluginError::HttpRequest(e.to_string())))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                %status,
                response_body = %body_text,
                "telegram delivery returned non-success status"
            );
            bail!(NotificationPluginError::DeliveryFailed(format!(
                "telegram API returned {status}: {body_text}"
            )));
        }

        tracing::debug!(chat_id, "telegram notification delivered");
        Ok(())
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<()> {
        if config
            .get("bot_token")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
        {
            bail!(NotificationPluginError::InvalidConfig(
                "'bot_token' is required".to_string()
            ));
        }

        if config
            .get("chat_id")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
        {
            bail!(NotificationPluginError::InvalidConfig(
                "'chat_id' is required".to_string()
            ));
        }

        Ok(())
    }

    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        let mut masked = config.clone();
        if let Some(obj) = masked.as_object_mut() {
            if obj.contains_key("bot_token") {
                obj.insert("bot_token".to_string(), serde_json::json!("***"));
            }
            if obj.contains_key("webhook_secret") {
                obj.insert("webhook_secret".to_string(), serde_json::json!("***"));
            }
        }
        masked
    }

    fn extension_manifests(&self) -> Vec<ExtensionManifest> {
        vec![
            ExtensionManifest::new(
                "notifications.telegram",
                "Telegram Channels",
                501,
                ExtensionPlacement::Panel {
                    target_page: "settings".to_string(),
                    position: PanelPosition::Tab,
                    tab_group: Some("Notification Channels".to_string()),
                },
                ExtensionUi::DataTable {
                    columns: vec![
                        TableColumn::new("name", "Name"),
                        TableColumn::new("chat_id", "Chat ID"),
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
            .with_permission("view_notifications"),
        ]
    }

    fn extension_actions(&self) -> Vec<ActionDef> {
        vec![
            ActionDef::new("list", "List"),
            ActionDef::new("create", "Add Telegram Channel")
                .with_permission("manage_notifications")
                .with_ui(ActionUi::Form(FormDef::new(vec![
                    FieldDef::new("name", "Name").required(),
                    FieldDef::new("bot_token", "Bot Token")
                        .required()
                        .with_type(FieldType::Password)
                        .sensitive()
                        .with_placeholder("123456:ABC-DEF..."),
                    FieldDef::new("chat_id", "Chat ID")
                        .required()
                        .with_placeholder("-1001234567890"),
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
                            "channel_type": "telegram",
                            "config": {
                                "bot_token": "{{bot_token}}",
                                "chat_id": "{{chat_id}}"
                            },
                            "enabled": "{{enabled:bool}}"
                        }),
                    )
                    .with_response_id_field("id"),
                ),
            ActionDef::new("edit", "Edit")
                .with_permission("manage_notifications")
                .with_ui(ActionUi::Form(FormDef::new(vec![
                    FieldDef::new("id", "ID").with_type(FieldType::Hidden),
                    FieldDef::new("name", "Name").required(),
                    FieldDef::new("bot_token", "Bot Token")
                        .with_type(FieldType::Password)
                        .sensitive()
                        .with_help_text("Leave unchanged to keep current token"),
                    FieldDef::new("chat_id", "Chat ID")
                        .required()
                        .with_placeholder("-1001234567890"),
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
                            "bot_token": "{{bot_token}}",
                            "chat_id": "{{chat_id}}"
                        },
                        "enabled": "{{enabled:bool}}"
                    }),
                )),
            ActionDef::new("test", "Test")
                .with_permission("manage_notifications")
                .with_api_submit(ApiSubmitDef::new(
                    "POST",
                    "/api/v1/notifications/channels/{{id}}/test",
                    serde_json::json!({}),
                )),
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
}

// ── PluginBase + NotificationTransportPlugin ────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PluginBase for TelegramPlugin {
    fn plugin_type_id(&self) -> &str {
        "telegram"
    }

    fn capabilities(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
        vec![uptrakit_plugin_infrastructure_core::PluginCapability::NotificationDelivery]
    }

    fn validate_config(&self, config: &serde_json::Value) -> std::result::Result<(), String> {
        NotificationPlugin::validate_config(self, config).map_err(|e| e.to_string())
    }

    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        NotificationPlugin::mask_config_secrets(self, config)
    }

    fn restore_config_secrets(
        &self,
        incoming: &serde_json::Value,
        stored: &serde_json::Value,
    ) -> serde_json::Value {
        NotificationPlugin::restore_config_secrets(self, incoming, stored)
    }

    fn extension_manifests(&self) -> Vec<uptrakit_extension_framework::ExtensionManifest> {
        NotificationPlugin::extension_manifests(self)
    }

    fn extension_actions(&self) -> Vec<uptrakit_extension_framework::ActionDef> {
        NotificationPlugin::extension_actions(self)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::NotificationTransportPlugin for TelegramPlugin {
    fn channel_type(&self) -> &'static str {
        NotificationPlugin::channel_type(self)
    }

    async fn deliver(&self, config: &serde_json::Value, message: &DeliveryMessage) -> Result<()> {
        NotificationPlugin::deliver(self, config, message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_config_requires_bot_token() {
        let plugin = TelegramPlugin::new().expect("client builds");
        let config = serde_json::json!({"chat_id": "12345"});
        let result = plugin.validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("'bot_token'"), "got: {msg}");
    }

    #[test]
    fn validate_config_requires_chat_id() {
        let plugin = TelegramPlugin::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "123:ABC"});
        let result = plugin.validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("'chat_id'"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_empty_bot_token() {
        let plugin = TelegramPlugin::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "", "chat_id": "12345"});
        let result = plugin.validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_rejects_empty_chat_id() {
        let plugin = TelegramPlugin::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "123:ABC", "chat_id": ""});
        let result = plugin.validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_accepts_valid_config() {
        let plugin = TelegramPlugin::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "123:ABC", "chat_id": "-100123"});
        assert!(plugin.validate_config(&config).is_ok());
    }

    #[test]
    fn mask_config_secrets_replaces_bot_token() {
        let plugin = TelegramPlugin::new().expect("client builds");
        let config = serde_json::json!({
            "bot_token": "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11",
            "chat_id": "-100123456"
        });
        let masked = plugin.mask_config_secrets(&config);
        assert_eq!(masked["bot_token"], "***");
        assert_eq!(masked["chat_id"], "-100123456");
    }

    #[test]
    fn mask_config_secrets_replaces_webhook_secret() {
        let plugin = TelegramPlugin::new().expect("client builds");
        let config = serde_json::json!({
            "bot_token": "tok",
            "chat_id": "id",
            "webhook_secret": "s3cret"
        });
        let masked = plugin.mask_config_secrets(&config);
        assert_eq!(masked["bot_token"], "***");
        assert_eq!(masked["webhook_secret"], "***");
    }

    #[test]
    fn escape_html_escapes_special_chars() {
        assert_eq!(escape_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn escape_html_preserves_plain_text() {
        assert_eq!(escape_html("hello world"), "hello world");
    }
}
