//! Telegram notification plugin — `declare_plugin!` descriptor and role impl.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_shared_types::ssrf::{SsrfSafeResolver, webpki_client_config};

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionPlacement, ExtensionUi,
    FieldDef, FieldType, FormDef, PanelPosition, TableColumn,
};
use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPluginError, Result, escape_html,
};
use uptrakit_plugin_infrastructure_core::{ConfigModel, PluginFamily, declare_plugin};

use crate::config::TelegramChannelConfig;

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
            .use_preconfigured_tls(webpki_client_config())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .dns_resolver(Arc::new(SsrfSafeResolver::new()))
            .build()
            .map_err(|e| report!(NotificationPluginError::HttpClientBuild(e.to_string())))?;
        Ok(Self { http })
    }
}

// ── NotificationTransport role implementation ────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::NotificationTransport for TelegramPlugin {
    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()> {
        // If per-channel config has no bot_token, try the global settings bag.
        let effective_config;
        let config = if config
            .get("bot_token")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
        {
            if let Some(global_token) = settings
                .get("global")
                .and_then(|g| g.get(crate::extensions::KEY_GLOBAL_TELEGRAM_BOT_TOKEN))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                effective_config = {
                    let mut c = config.clone();
                    c.as_object_mut()
                        .expect("config is always an object")
                        .insert("bot_token".to_string(), serde_json::json!(global_token));
                    c
                };
                &effective_config
            } else {
                config
            }
        } else {
            config
        };

        // bot_token must be present after merging (extracted from settings bag
        // when absent in per-channel config).
        let bot_token = config["bot_token"].as_str().ok_or_else(|| {
            report!(NotificationPluginError::InvalidConfig(
                "missing 'bot_token' (no per-channel token and no global token configured)"
                    .to_string()
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
}

// ── Extension functions ──────────────────────────────────────────────────────

fn telegram_extension_manifests() -> Vec<ExtensionManifest> {
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
        // Global Telegram defaults panel (below global settings)
        ExtensionManifest::new(
            "notifications.telegram.global_settings",
            "Telegram Defaults",
            601,
            ExtensionPlacement::Panel {
                target_page: "global-settings".to_string(),
                position: PanelPosition::Below,
                tab_group: None,
            },
            ExtensionUi::Form(
                FormDef::new(vec![
                    FieldDef::new("bot_token", "Global Bot Token")
                        .with_type(FieldType::Password)
                        .sensitive()
                        .with_placeholder("123456:ABC-DEF...")
                        .with_help_text(
                            "Shared bot token used as a fallback for all Telegram channels \
                             that do not have their own token configured.",
                        ),
                ])
                .with_pre_load_action("get_global_telegram"),
            ),
        )
        .with_permission("manage_global_settings"),
    ]
}

fn telegram_extension_actions() -> Vec<ActionDef> {
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
        ActionDef::new("get_global_telegram", "Get Global Telegram Settings"),
        ActionDef::new("save_global_telegram", "Save Global Telegram Settings")
            .with_permission("manage_global_settings"),
    ]
}

/// Extension action handler wrapper that bridges the `descriptor::ExtensionActionContext`
/// (with `dyn Any` db field) to the concrete `plugin_ops::ExtensionActionContext`
/// (with `&DatabaseConnection` db field) used by `extensions::handle_action`.
fn telegram_handle_extension_action<'a>(
    ctx: &'a uptrakit_plugin_infrastructure_core::descriptor::ExtensionActionContext<'a>,
    extension_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<Output = std::result::Result<serde_json::Value, String>>
            + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        let db = ctx
            .db
            .downcast_ref::<sea_orm::DatabaseConnection>()
            .ok_or_else(|| "internal error: expected DatabaseConnection".to_string())?;
        let inner_ctx = uptrakit_plugin_infrastructure_core::ExtensionActionContext {
            db,
            tenant_id: ctx.tenant_id,
            caller_user_id: ctx.caller_user_id,
        };
        crate::extensions::handle_action(&inner_ctx, extension_id, action_id, params).await
    })
}

/// Singleton factory for the notification transport.
fn create_telegram_transport(
    _config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::error::Result<
    Arc<dyn uptrakit_plugin_infrastructure_core::NotificationTransport>,
> {
    let plugin = TelegramPlugin::new().map_err(|e| {
        rootcause::report!(
            uptrakit_plugin_infrastructure_core::PluginError::Configuration(e.to_string())
        )
    })?;
    Ok(Arc::new(plugin))
}

fn telegram_surface_registrations()
-> Vec<uptrakit_plugin_infrastructure_core::surfaces::SurfaceRegistration> {
    uptrakit_plugin_infrastructure_core::build_plugin_surface_registrations_from_extensions(
        "telegram",
        telegram_extension_manifests(),
        telegram_extension_actions(),
    )
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(TelegramPlugin, TelegramChannelConfig, "telegram", {
    display_name: "Telegram",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_telegram_transport,
    owned_extension_ids: &["notifications.telegram", "notifications.telegram.global_settings"],
    raw_settings_keys: &["global_telegram.bot_token"],
    extensions: {
        manifests: telegram_extension_manifests,
        actions: telegram_extension_actions,
        handle_action: telegram_handle_extension_action,
    },
    surfaces: {
        registrations: telegram_surface_registrations,
    },
});

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::PluginMeta;

    #[test]
    fn plugin_type_id_is_telegram() {
        let plugin = TelegramPlugin::new().expect("client builds");
        assert_eq!(plugin.plugin_type_id().as_str(), "telegram");
    }

    #[test]
    fn descriptor_type_id() {
        assert_eq!(DESCRIPTOR.type_id, "telegram");
    }

    #[test]
    fn descriptor_family_is_notification() {
        assert_eq!(DESCRIPTOR.family, PluginFamily::Notification);
    }

    #[test]
    fn descriptor_config_model_is_notification_channel() {
        assert_eq!(DESCRIPTOR.config_model, ConfigModel::NotificationChannel);
    }

    #[test]
    fn descriptor_has_notification_transport() {
        assert!(DESCRIPTOR.roles.notification_transport.is_some());
    }

    #[test]
    fn descriptor_has_extensions() {
        assert!(DESCRIPTOR.extensions.is_some());
        let ext = DESCRIPTOR.extensions.unwrap();
        assert!(ext.owned_ids.contains(&"notifications.telegram"));
        assert!(
            ext.owned_ids
                .contains(&"notifications.telegram.global_settings")
        );
    }

    #[test]
    fn descriptor_has_plugin_surface_registrations() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        assert!(!registrations.is_empty());
        assert!(registrations.iter().all(|registration| {
            registration.provider.provider_kind
                == uptrakit_plugin_infrastructure_core::surfaces::ProviderKind::Plugin
        }));
        let all_surface_ids: Vec<String> = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .map(|surface| surface.descriptor.surface_id.to_string())
            .collect();
        assert!(
            all_surface_ids
                .iter()
                .any(|id| id == "notifications.telegram.global_settings")
        );
        assert!(
            all_surface_ids
                .iter()
                .any(|id| id == "notifications.telegram"),
            "notification channel data-table should be registered as an action-driven shared surface"
        );
    }

    #[test]
    fn descriptor_has_raw_settings_keys() {
        assert!(
            DESCRIPTOR
                .raw_settings_keys
                .contains(&"global_telegram.bot_token")
        );
    }

    #[test]
    fn descriptor_capabilities_include_notification_delivery() {
        assert!(DESCRIPTOR.capabilities.contains(
            &uptrakit_plugin_infrastructure_core::PluginCapability::NotificationDelivery
        ));
    }

    // ── Config validation via descriptor ─────────────────────────────────

    #[test]
    fn descriptor_validate_accepts_config_without_bot_token() {
        let config = serde_json::json!({"chat_id": "-100123"});
        assert!((DESCRIPTOR.config.validate)(&config).is_ok());
    }

    #[test]
    fn descriptor_validate_requires_chat_id() {
        let config = serde_json::json!({"bot_token": "123:ABC"});
        let result = (DESCRIPTOR.config.validate)(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("'chat_id'"), "got: {msg}");
    }

    #[test]
    fn descriptor_validate_rejects_empty_chat_id() {
        let config = serde_json::json!({"bot_token": "123:ABC", "chat_id": ""});
        assert!((DESCRIPTOR.config.validate)(&config).is_err());
    }

    #[test]
    fn descriptor_validate_accepts_valid_config() {
        let config = serde_json::json!({"bot_token": "123:ABC", "chat_id": "-100123"});
        assert!((DESCRIPTOR.config.validate)(&config).is_ok());
    }

    // ── Secret masking via descriptor ────────────────────────────────────

    #[test]
    fn descriptor_mask_secrets_replaces_bot_token() {
        let config = serde_json::json!({
            "bot_token": "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11",
            "chat_id": "-100123456"
        });
        let masked = (DESCRIPTOR.config.mask_secrets)(&config);
        assert_eq!(masked["bot_token"], "***");
        assert_eq!(masked["chat_id"], "-100123456");
    }

    #[test]
    fn descriptor_mask_secrets_replaces_webhook_secret() {
        let config = serde_json::json!({
            "bot_token": "tok",
            "chat_id": "id",
            "webhook_secret": "s3cret"
        });
        let masked = (DESCRIPTOR.config.mask_secrets)(&config);
        assert_eq!(masked["bot_token"], "***");
        assert_eq!(masked["webhook_secret"], "***");
    }

    // ── Secret restoration via descriptor ────────────────────────────────

    #[test]
    fn descriptor_restore_secrets_from_stored() {
        let stored = serde_json::json!({
            "bot_token": "real-token",
            "chat_id": "-100123",
            "webhook_secret": "real-secret"
        });
        let mut incoming = serde_json::json!({
            "bot_token": "***",
            "chat_id": "-100123",
            "webhook_secret": "***"
        });
        (DESCRIPTOR.config.restore_secrets)(&mut incoming, &stored);
        assert_eq!(incoming["bot_token"], "real-token");
        assert_eq!(incoming["webhook_secret"], "real-secret");
    }

    // ── escape_html (shared helper) ──────────────────────────────────────

    #[test]
    fn escape_html_escapes_special_chars() {
        assert_eq!(escape_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn escape_html_preserves_plain_text() {
        assert_eq!(escape_html("hello world"), "hello world");
    }
}
