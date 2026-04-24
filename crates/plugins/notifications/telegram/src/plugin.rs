//! Telegram notification plugin — `declare_plugin!` descriptor and role impl.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_shared_types::ssrf::{SsrfSafeResolver, webpki_client_config};

use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPluginError, Result, escape_html,
};
use uptrakit_plugin_infrastructure_core::{
    ApiSubmitDescriptor, ConfigModel, FormFieldDescriptor, FormFieldType, PluginFamily,
    SurfaceActionDescriptor, SurfaceActionUi, SurfaceFormDescriptor, declare_plugin, surfaces,
};

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
                .and_then(|g| g.get(crate::surfaces::KEY_GLOBAL_TELEGRAM_BOT_TOKEN))
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

fn telegram_surface_actions() -> Vec<SurfaceActionDescriptor> {
    vec![
        SurfaceActionDescriptor::new("list", "List"),
        SurfaceActionDescriptor::new("create", "Add Telegram Channel")
            .with_permission("manage_notifications")
            .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
                FormFieldDescriptor::new("name", "Name").required(),
                FormFieldDescriptor::new("bot_token", "Bot Token")
                    .required()
                    .with_type(FormFieldType::Password)
                    .sensitive()
                    .with_placeholder("123456:ABC-DEF..."),
                FormFieldDescriptor::new("chat_id", "Chat ID")
                    .required()
                    .with_placeholder("-1001234567890"),
                FormFieldDescriptor::new("enabled", "Enabled")
                    .with_type(FormFieldType::Toggle)
                    .with_default_value(serde_json::json!("true")),
            ])))
            .with_api_submit(
                ApiSubmitDescriptor::new(
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
        SurfaceActionDescriptor::new("edit", "Edit")
            .with_permission("manage_notifications")
            .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
                FormFieldDescriptor::new("id", "ID").with_type(FormFieldType::Hidden),
                FormFieldDescriptor::new("name", "Name").required(),
                FormFieldDescriptor::new("bot_token", "Bot Token")
                    .with_type(FormFieldType::Password)
                    .sensitive()
                    .with_help_text("Leave unchanged to keep current token"),
                FormFieldDescriptor::new("chat_id", "Chat ID")
                    .required()
                    .with_placeholder("-1001234567890"),
                FormFieldDescriptor::new("enabled", "Enabled")
                    .with_type(FormFieldType::Toggle)
                    .with_default_value(serde_json::json!("true")),
            ])))
            .with_api_submit(ApiSubmitDescriptor::new(
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
        SurfaceActionDescriptor::new("test", "Test")
            .with_permission("manage_notifications")
            .with_api_submit(ApiSubmitDescriptor::new(
                "POST",
                "/api/v1/notifications/channels/{{id}}/test",
                serde_json::json!({}),
            )),
        SurfaceActionDescriptor::new("delete", "Delete")
            .with_permission("manage_notifications")
            .destructive()
            .with_confirm_entity_field("name")
            .with_api_submit(ApiSubmitDescriptor::new(
                "DELETE",
                "/api/v1/notifications/channels/{{id}}",
                serde_json::json!({}),
            )),
        SurfaceActionDescriptor::new("get_global_telegram", "Get Global Telegram Settings"),
        SurfaceActionDescriptor::new("save_global_telegram", "Save Global Telegram Settings")
            .with_permission("manage_global_settings"),
    ]
}

/// Surface action handler wrapper that bridges the shared `SurfaceActionContext`
/// receiver to `surfaces::handle_surface_action`.
fn telegram_handle_surface_action<'a>(
    ctx: &'a uptrakit_plugin_infrastructure_core::SurfaceActionContext<'a>,
    surface_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = std::result::Result<
                    serde_json::Value,
                    uptrakit_plugin_infrastructure_core::SurfaceActionError,
                >,
            > + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        crate::surfaces::handle_surface_action(ctx, surface_id, action_id, params).await
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

fn collect_registration_capabilities(
    surfaces: &[surfaces::RegisteredSurface],
) -> surfaces::CapabilitySet {
    let mut caps = BTreeSet::new();
    for surface in surfaces {
        caps.extend(surface.descriptor.required_capabilities.0.iter().cloned());
    }
    surfaces::CapabilitySet(caps)
}

fn telegram_surface_registrations() -> Vec<surfaces::SurfaceRegistration> {
    let channels_surface = {
        let data_source_id =
            surfaces::DataSourceId::new("data.primary").expect("literal data source id is valid");
        surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("notifications.telegram")
                    .expect("literal surface id is valid"),
                label: "Telegram Channels".to_string(),
                priority: 501,
                slot: surfaces::SLOT_SETTINGS_TABS.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: Some("view_notifications".to_string()),
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::SectionNode,
                    surfaces::Capability::ActionBarNode,
                    surfaces::Capability::TableNode,
                    surfaces::Capability::DataLoad,
                    surfaces::Capability::FormSubmit,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::ConfirmableAction,
                    surfaces::Capability::ProviderQueryDataSource,
                    surfaces::Capability::UniversalTargeting,
                    surfaces::Capability::SensitiveFields,
                ]),
                root_node: surfaces::SurfaceNode::Section {
                    title: None,
                    children: vec![
                        surfaces::SurfaceNode::ActionBar {
                            action_ids: vec![
                                surfaces::InteractionId::new("create")
                                    .expect("literal interaction id is valid"),
                            ],
                        },
                        surfaces::SurfaceNode::Table {
                            data_source_id: data_source_id.clone(),
                            columns: vec![
                                surfaces::SurfaceTableColumn {
                                    key: "name".to_string(),
                                    label: "Name".to_string(),
                                },
                                surfaces::SurfaceTableColumn {
                                    key: "chat_id".to_string(),
                                    label: "Chat ID".to_string(),
                                },
                                surfaces::SurfaceTableColumn {
                                    key: "enabled".to_string(),
                                    label: "Enabled".to_string(),
                                },
                                surfaces::SurfaceTableColumn {
                                    key: "created_at".to_string(),
                                    label: "Created".to_string(),
                                },
                            ],
                            row_actions: vec![
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("edit")
                                        .expect("literal interaction id is valid"),
                                    visible_when: None,
                                },
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("test")
                                        .expect("literal interaction id is valid"),
                                    visible_when: None,
                                },
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("delete")
                                        .expect("literal interaction id is valid"),
                                    visible_when: None,
                                },
                            ],
                        },
                    ],
                },
            },
            interactions: vec![
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("list")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::DataLoad,
                    label: "List".to_string(),
                    required_permission: None,
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("create")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: "Add Telegram Channel".to_string(),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec!["bot_token".to_string()],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: Some(surfaces::FormUiDescriptor {
                        fields: vec![
                            surfaces::FormFieldDescriptor {
                                key: "name".to_string(),
                                label: "Name".to_string(),
                                field_type: "text".to_string(),
                                required: true,
                                placeholder: None,
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "bot_token".to_string(),
                                label: "Bot Token".to_string(),
                                field_type: "password".to_string(),
                                required: true,
                                placeholder: Some("123456:ABC-DEF...".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: true,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "chat_id".to_string(),
                                label: "Chat ID".to_string(),
                                field_type: "text".to_string(),
                                required: true,
                                placeholder: Some("-1001234567890".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "enabled".to_string(),
                                label: "Enabled".to_string(),
                                field_type: "toggle".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("true".to_string()),
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                        ],
                        pre_load_interaction_id: None,
                    }),
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("edit")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: "Edit".to_string(),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec!["bot_token".to_string()],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: Some(surfaces::FormUiDescriptor {
                        fields: vec![
                            surfaces::FormFieldDescriptor {
                                key: "id".to_string(),
                                label: "ID".to_string(),
                                field_type: "hidden".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "name".to_string(),
                                label: "Name".to_string(),
                                field_type: "text".to_string(),
                                required: true,
                                placeholder: None,
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "bot_token".to_string(),
                                label: "Bot Token".to_string(),
                                field_type: "password".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: Some(
                                    "Leave unchanged to keep current token".to_string(),
                                ),
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: true,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "chat_id".to_string(),
                                label: "Chat ID".to_string(),
                                field_type: "text".to_string(),
                                required: true,
                                placeholder: Some("-1001234567890".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "enabled".to_string(),
                                label: "Enabled".to_string(),
                                field_type: "toggle".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("true".to_string()),
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                        ],
                        pre_load_interaction_id: None,
                    }),
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("test")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: "Test".to_string(),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("delete")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::ConfirmableAction,
                    label: "Delete".to_string(),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: Some(surfaces::InteractionConfirmation {
                        title: "Confirm Delete".to_string(),
                        message: "This action may modify existing data.".to_string(),
                        confirm_label: None,
                        cancel_label: None,
                        severity: surfaces::ConfirmationSeverity::Danger,
                    }),
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
            ],
            data_sources: vec![surfaces::DataSourceDescriptor {
                data_source_id,
                kind: surfaces::DataSourceKind::ProviderQuery {
                    operation_id: "list".to_string(),
                },
                result_schema: surfaces::SchemaContract::Array,
                pagination: Some(surfaces::DataSourcePagination {
                    default_page_size: 20,
                    max_page_size: 200,
                }),
                sorting: None,
                filtering: None,
                refresh_policy: surfaces::RefreshPolicy::Manual,
                empty_state: None,
            }],
        }
    };

    let global_settings_surface = {
        let save_global_interaction = surfaces::InteractionId::new("save_global_telegram")
            .expect("literal interaction id is valid");
        surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("notifications.telegram.global_settings")
                    .expect("literal surface id is valid"),
                label: "Telegram Defaults".to_string(),
                priority: 601,
                slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: Some("manage_global_settings".to_string()),
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::FormNode,
                    surfaces::Capability::DataLoad,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                    surfaces::Capability::SensitiveFields,
                ]),
                root_node: surfaces::SurfaceNode::Form {
                    interaction_id: save_global_interaction.clone(),
                },
            },
            interactions: vec![
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("get_global_telegram")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::DataLoad,
                    label: "Get Global Telegram Settings".to_string(),
                    required_permission: None,
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: save_global_interaction,
                    kind: surfaces::InteractionKind::MutationAction,
                    label: "Save Global Telegram Settings".to_string(),
                    required_permission: Some("manage_global_settings".to_string()),
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec!["bot_token".to_string()],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: Some(surfaces::FormUiDescriptor {
                        fields: vec![surfaces::FormFieldDescriptor {
                            key: "bot_token".to_string(),
                            label: "Global Bot Token".to_string(),
                            field_type: "password".to_string(),
                            required: false,
                            placeholder: Some("123456:ABC-DEF...".to_string()),
                            help_text: Some(
                                "Shared bot token used as a fallback for all Telegram channels that do not have their own token configured.".to_string(),
                            ),
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: true,
                            list: false,
                            visible_when: None,
                        }],
                        pre_load_interaction_id: Some(
                            surfaces::InteractionId::new("get_global_telegram")
                                .expect("literal interaction id is valid"),
                        ),
                    }),
                },
            ],
            data_sources: vec![],
        }
    };

    let surfaces = vec![channels_surface, global_settings_surface];
    vec![surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "plugin.telegram".to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: collect_registration_capabilities(&surfaces),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces,
        encryption_metadata: None,
    }]
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(TelegramPlugin, TelegramChannelConfig, "telegram", {
    display_name: "Telegram",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_telegram_transport,
    owned_surface_ids: &["notifications.telegram", "notifications.telegram.global_settings"],
    raw_settings_keys: &["global_telegram.bot_token"],
    surface_actions: {
        actions: telegram_surface_actions,
        handle_action: telegram_handle_surface_action,
    },
    surfaces: {
        registrations: telegram_surface_registrations,
    },
});

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{PluginMeta, surfaces};

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
    fn descriptor_has_surface_actions() {
        assert!(DESCRIPTOR.surface_actions.is_some());
        let ext = DESCRIPTOR.surface_actions.unwrap();
        assert!(ext.owned_surface_ids().contains(&"notifications.telegram"));
        assert!(
            ext.owned_surface_ids()
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
    fn telegram_channel_surface_keeps_table_and_sensitive_action_contract() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let channel_surface = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| surface.descriptor.surface_id.as_str() == "notifications.telegram")
            .expect("notifications.telegram surface should be present");

        assert_eq!(
            channel_surface.descriptor.slot,
            surfaces::SLOT_SETTINGS_TABS
        );
        assert_eq!(channel_surface.data_sources.len(), 1);
        assert!(matches!(
            &channel_surface.data_sources[0].kind,
            surfaces::DataSourceKind::ProviderQuery { operation_id } if operation_id == "list"
        ));

        let find_interaction = |id: &str| {
            channel_surface
                .interactions
                .iter()
                .find(|interaction| interaction.interaction_id.as_str() == id)
                .unwrap_or_else(|| panic!("interaction `{id}` should exist"))
        };
        assert_eq!(
            find_interaction("list").kind,
            surfaces::InteractionKind::DataLoad
        );
        assert_eq!(
            find_interaction("create").kind,
            surfaces::InteractionKind::FormSubmit
        );
        assert_eq!(
            find_interaction("edit").kind,
            surfaces::InteractionKind::FormSubmit
        );
        assert_eq!(
            find_interaction("test").kind,
            surfaces::InteractionKind::MutationAction
        );
        assert_eq!(
            find_interaction("delete").kind,
            surfaces::InteractionKind::ConfirmableAction
        );
        assert!(find_interaction("delete").confirmation.is_some());
        assert!(
            find_interaction("create")
                .sensitive_fields
                .iter()
                .any(|field| field == "bot_token")
        );
        assert!(
            find_interaction("edit")
                .sensitive_fields
                .iter()
                .any(|field| field == "bot_token")
        );
    }

    #[test]
    fn telegram_global_settings_surface_keeps_preload_form_behavior() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let settings_surface = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| {
                surface.descriptor.surface_id.as_str() == "notifications.telegram.global_settings"
            })
            .expect("notifications.telegram.global_settings surface should be present");

        assert_eq!(
            settings_surface.descriptor.slot,
            surfaces::SLOT_SETTINGS_BELOW_GLOBAL
        );
        assert!(matches!(
            settings_surface.descriptor.root_node,
            surfaces::SurfaceNode::Form { ref interaction_id }
                if interaction_id.as_str() == "save_global_telegram"
        ));

        let save = settings_surface
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "save_global_telegram")
            .expect("save_global_telegram interaction should exist");
        assert_eq!(save.kind, surfaces::InteractionKind::MutationAction);
        assert!(
            save.sensitive_fields
                .iter()
                .any(|field| field == "bot_token")
        );
        assert_eq!(
            save.form_ui
                .as_ref()
                .and_then(|form_ui| form_ui.pre_load_interaction_id.as_ref())
                .map(|interaction_id| interaction_id.as_str()),
            Some("get_global_telegram")
        );
        assert!(settings_surface.interactions.iter().any(|interaction| {
            interaction.interaction_id.as_str() == "get_global_telegram"
                && interaction.kind == surfaces::InteractionKind::DataLoad
        }));
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
        assert!(msg.to_string().contains("chat_id"), "got: {msg}");
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
