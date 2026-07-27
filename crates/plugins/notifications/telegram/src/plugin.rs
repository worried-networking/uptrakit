#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "infallible literal surface ID and value constructions; panic would indicate a programming error in the surface manifest; array and slice indices are bounded by construction or derived from known-valid positions"
)]
//! Telegram notification plugin — `declare_plugin!` descriptor and role impl.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;

use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPluginError, Result, escape_html,
};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, InteractionDelivery, PluginFamily, PluginHttpClientConfig, PluginSurface,
    PluginSurfaceRegistration, RegisteredInteraction, SsrfMode, build_plugin_http_client,
    declare_plugin, surfaces,
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
        let http = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-notification-telegram/",
                env!("CARGO_PKG_VERSION")
            ),
            ssrf_mode: SsrfMode::Strict,
            ..Default::default()
        })
        .map_err(|e| report!(NotificationPluginError::HttpClientBuild(e.to_string())))?;
        Ok(Self { http })
    }
}

fn map_send_error(e: reqwest::Error) -> Report<NotificationPluginError> {
    report!(NotificationPluginError::HttpRequest(
        e.without_url().to_string()
    ))
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
            .map_err(map_send_error)?;

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

    async fn handle_callback(
        &self,
        ctx: &uptrakit_plugin_infrastructure_core::SurfaceActionContext<'_>,
        params: &serde_json::Value,
    ) -> std::result::Result<
        serde_json::Value,
        uptrakit_plugin_infrastructure_core::SurfaceActionError,
    > {
        crate::surfaces::handle_callback(ctx, params).await
    }
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

fn telegram_plugin_surfaces() -> Vec<PluginSurfaceRegistration> {
    let channels_surface = {
        let data_source_id =
            surfaces::DataSourceId::new("channels").expect("literal data source id is valid");
        PluginSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(
                    surfaces::SurfaceId::new("notifications.telegram")
                        .expect("literal surface id is valid"),
                )
                .label("Telegram Channels")
                .priority(501)
                .slot(surfaces::SLOT_SETTINGS_TABS)
                .scope(surfaces::Scope::Global)
                .targeting(surfaces::Targeting::Universal)
                .required_permission("view_notifications")
                .provider_kind(surfaces::ProviderKind::Plugin)
                .tab_group("notification-channels", "Notification Channels")
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
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
                ]))
                .root_node(surfaces::SurfaceNode::section(
                    None::<String>,
                    vec![
                        surfaces::SurfaceNode::ActionBar {
                            action_ids: vec![surfaces::ActionRef::WithMethod {
                                interaction_id: surfaces::InteractionId::new("channels")
                                    .expect("literal interaction id is valid"),
                                http_method: Some(surfaces::InteractionHttpMethod::Post),
                            }],
                        },
                        surfaces::SurfaceNode::Table {
                            data_source_id: data_source_id.clone(),
                            columns: vec![
                                surfaces::SurfaceTableColumn::new("name", "Name"),
                                surfaces::SurfaceTableColumn::new("chat_id", "Chat ID"),
                                surfaces::SurfaceTableColumn::new("enabled", "Enabled"),
                                surfaces::SurfaceTableColumn::new("created_at", "Created"),
                            ],
                            row_actions: vec![
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("channels")
                                        .expect("literal interaction id is valid"),
                                    http_method: Some(surfaces::InteractionHttpMethod::Put),
                                    visible_when: None,
                                },
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("test")
                                        .expect("literal interaction id is valid"),
                                    http_method: None,
                                    visible_when: None,
                                },
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("channels")
                                        .expect("literal interaction id is valid"),
                                    http_method: Some(surfaces::InteractionHttpMethod::Delete),
                                    visible_when: None,
                                },
                            ],
                        },
                    ],
                ))
                .build(),
            interactions: vec![
                RegisteredInteraction::new(
                    {
                        let mut i = surfaces::InteractionDescriptor::new(
                            surfaces::InteractionId::new("channels")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionKind::DataLoad,
                            "List",
                            surfaces::InteractionTransport::ControllerLocal,
                        );
                        i.result_schema = Some(surfaces::SchemaContract::Any);
                        i
                    },
                    InteractionDelivery::PluginHandled(crate::surfaces::telegram_list_handler),
                ),
                RegisteredInteraction::new(
                    {
                        let mut i = surfaces::InteractionDescriptor::new(
                            surfaces::InteractionId::new("channels")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionKind::FormSubmit,
                            "Add Telegram Channel",
                            surfaces::InteractionTransport::ControllerLocal,
                        );
                        i.required_permission = Some("manage_notifications".to_string());
                        i.input_schema = Some(surfaces::SchemaContract::Object);
                        i.result_schema = Some(surfaces::SchemaContract::Any);
                        i.sensitive_fields = vec!["bot_token".to_string()];
                        i.form_ui = Some(surfaces::FormUiDescriptor {
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
                        });
                        i
                    },
                    InteractionDelivery::ControllerExecutor,
                ),
                RegisteredInteraction::new(
                    {
                        let mut i = surfaces::InteractionDescriptor::new(
                            surfaces::InteractionId::new("channels")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionKind::FormSubmit,
                            "Edit",
                            surfaces::InteractionTransport::ControllerLocal,
                        );
                        i.http_method = surfaces::InteractionHttpMethod::Put;
                        i.required_permission = Some("manage_notifications".to_string());
                        i.input_schema = Some(surfaces::SchemaContract::Object);
                        i.result_schema = Some(surfaces::SchemaContract::Any);
                        i.sensitive_fields = vec!["bot_token".to_string()];
                        i.form_ui = Some(surfaces::FormUiDescriptor {
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
                        });
                        i
                    },
                    InteractionDelivery::ControllerExecutor,
                ),
                RegisteredInteraction::new(
                    {
                        let mut i = surfaces::InteractionDescriptor::new(
                            surfaces::InteractionId::new("test")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionKind::MutationAction,
                            "Test",
                            surfaces::InteractionTransport::ControllerLocal,
                        );
                        i.required_permission = Some("manage_notifications".to_string());
                        i.input_schema = Some(surfaces::SchemaContract::Object);
                        i.result_schema = Some(surfaces::SchemaContract::Any);
                        i
                    },
                    InteractionDelivery::ControllerExecutor,
                ),
                RegisteredInteraction::new(
                    {
                        let mut i = surfaces::InteractionDescriptor::new(
                            surfaces::InteractionId::new("channels")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionKind::ConfirmableAction,
                            "Delete",
                            surfaces::InteractionTransport::ControllerLocal,
                        );
                        i.http_method = surfaces::InteractionHttpMethod::Delete;
                        i.required_permission = Some("manage_notifications".to_string());
                        i.input_schema = Some(surfaces::SchemaContract::Object);
                        i.result_schema = Some(surfaces::SchemaContract::Any);
                        i.confirmation = Some(surfaces::InteractionConfirmation {
                            title: "Confirm Delete".to_string(),
                            message: "This action may modify existing data.".to_string(),
                            confirm_label: None,
                            cancel_label: None,
                            severity: surfaces::ConfirmationSeverity::Danger,
                        });
                        i
                    },
                    InteractionDelivery::ControllerExecutor,
                ),
            ],
            data_sources: vec![surfaces::DataSourceDescriptor {
                data_source_id,
                kind: surfaces::DataSourceKind::ProviderQuery {
                    operation_id: "channels".to_string(),
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
        let save_global_interaction =
            surfaces::InteractionId::new("settings").expect("literal interaction id is valid");
        PluginSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(
                    surfaces::SurfaceId::new("notifications.telegram.global-settings")
                        .expect("literal surface id is valid"),
                )
                .label("Telegram Defaults")
                .priority(601)
                .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                .scope(surfaces::Scope::Global)
                .targeting(surfaces::Targeting::Universal)
                .required_permission("manage_global_settings")
                .provider_kind(surfaces::ProviderKind::Plugin)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::FormNode,
                    surfaces::Capability::DataLoad,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                    surfaces::Capability::SensitiveFields,
                ]))
                .root_node(surfaces::SurfaceNode::Form {
                    interaction_id: save_global_interaction.clone(),
                    http_method: Some(surfaces::InteractionHttpMethod::Put),
                })
                .build(),
            interactions: vec![
                RegisteredInteraction::new(
                    {
                        let mut i = surfaces::InteractionDescriptor::new(
                            surfaces::InteractionId::new("settings")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionKind::DataLoad,
                            "Get Global Telegram Settings",
                            surfaces::InteractionTransport::ControllerLocal,
                        );
                        i.result_schema = Some(surfaces::SchemaContract::Any);
                        i
                    },
                    InteractionDelivery::PluginHandled(
                        crate::surfaces::telegram_get_global_handler,
                    ),
                ),
                RegisteredInteraction::new(
                    {
                        let mut i = surfaces::InteractionDescriptor::new(
                            save_global_interaction,
                            surfaces::InteractionKind::MutationAction,
                            "Save Global Telegram Settings",
                            surfaces::InteractionTransport::ControllerLocal,
                        );
                        i.http_method = surfaces::InteractionHttpMethod::Put;
                        i.required_permission = Some("manage_global_settings".to_string());
                        i.result_schema = Some(surfaces::SchemaContract::Any);
                        i.sensitive_fields = vec!["bot_token".to_string()];
                        i.form_ui = Some(surfaces::FormUiDescriptor {
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
                            surfaces::InteractionId::new("settings")
                                .expect("literal interaction id is valid"),
                        ),
                    });
                        i
                    },
                    InteractionDelivery::PluginHandled(
                        crate::surfaces::telegram_save_global_handler,
                    ),
                ),
            ],
            data_sources: vec![],
        }
    };

    let surfaces = vec![channels_surface, global_settings_surface];
    vec![PluginSurfaceRegistration { surfaces }]
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(TelegramPlugin, TelegramChannelConfig, "telegram", {
    display_name: "Telegram",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_telegram_transport,
    raw_settings_keys: &["global_telegram.bot_token"],
    surfaces: {
        provider_id: "plugin.telegram",
        registrations: telegram_plugin_surfaces,
    },
});

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
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
    fn unified_registrations_pair_every_interaction_with_expected_delivery() {
        use uptrakit_plugin_infrastructure_core::InteractionDeliveryKind;
        let registrations = telegram_plugin_surfaces();
        let mut seen: Vec<(String, String, String, InteractionDeliveryKind)> = Vec::new();
        for registration in &registrations {
            for surface in &registration.surfaces {
                for interaction in &surface.interactions {
                    assert_eq!(
                        interaction.descriptor().transport,
                        surfaces::InteractionTransport::ControllerLocal
                    );
                    seen.push((
                        surface.descriptor.surface_id.as_str().to_string(),
                        interaction.descriptor().interaction_id.as_str().to_string(),
                        interaction
                            .descriptor()
                            .effective_http_method()
                            .as_str()
                            .to_string(),
                        interaction.delivery().kind(),
                    ));
                }
            }
        }
        // Expected (surface, interaction, delivery) table (spec D6); the table's
        // own length is the count source of truth below — no bare literal count
        // is asserted separately.
        let expected: Vec<(&str, &str, &str, InteractionDeliveryKind)> = vec![
            (
                "notifications.telegram",
                "channels",
                "get",
                InteractionDeliveryKind::PluginHandled,
            ),
            (
                "notifications.telegram",
                "channels",
                "post",
                InteractionDeliveryKind::ControllerExecutor,
            ),
            (
                "notifications.telegram",
                "channels",
                "put",
                InteractionDeliveryKind::ControllerExecutor,
            ),
            (
                "notifications.telegram",
                "test",
                "post",
                InteractionDeliveryKind::ControllerExecutor,
            ),
            (
                "notifications.telegram",
                "channels",
                "delete",
                InteractionDeliveryKind::ControllerExecutor,
            ),
            (
                "notifications.telegram.global-settings",
                "settings",
                "get",
                InteractionDeliveryKind::PluginHandled,
            ),
            (
                "notifications.telegram.global-settings",
                "settings",
                "put",
                InteractionDeliveryKind::PluginHandled,
            ),
        ];
        for (surface, id, method, kind) in &expected {
            assert!(
                seen.iter()
                    .any(|(s, i, m, k)| s == surface && i == id && m == method && k == kind),
                "missing ({surface}, {id}, {method}, {kind:?})"
            );
        }
        assert_eq!(
            seen.len(),
            expected.len(),
            "unexpected total interaction count across telegram_plugin_surfaces()"
        );
    }

    #[test]
    fn descriptor_has_plugin_surface_registrations() {
        let registrations = telegram_plugin_surfaces()
            .iter()
            .map(|r| r.to_wire("plugin.telegram"))
            .collect::<Vec<_>>();
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
                .any(|id| id == "notifications.telegram.global-settings")
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
        let registrations = telegram_plugin_surfaces()
            .iter()
            .map(|r| r.to_wire("plugin.telegram"))
            .collect::<Vec<_>>();
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
            surfaces::DataSourceKind::ProviderQuery { operation_id } if operation_id == "channels"
        ));

        let find_interaction = |id: &str, method: surfaces::InteractionHttpMethod| {
            channel_surface
                .interactions
                .iter()
                .find(|interaction| {
                    interaction.interaction_id.as_str() == id
                        && interaction.effective_http_method() == method
                })
                .unwrap_or_else(|| panic!("interaction `{id}` ({method}) should exist"))
        };
        assert_eq!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Get).kind,
            surfaces::InteractionKind::DataLoad
        );
        assert_eq!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Post).kind,
            surfaces::InteractionKind::FormSubmit
        );
        assert_eq!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Put).kind,
            surfaces::InteractionKind::FormSubmit
        );
        assert_eq!(
            find_interaction("test", surfaces::InteractionHttpMethod::Post).kind,
            surfaces::InteractionKind::MutationAction
        );
        assert_eq!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Delete).kind,
            surfaces::InteractionKind::ConfirmableAction
        );
        assert!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Delete)
                .confirmation
                .is_some()
        );
        assert!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Post)
                .sensitive_fields
                .iter()
                .any(|field| field == "bot_token")
        );
        assert!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Put)
                .sensitive_fields
                .iter()
                .any(|field| field == "bot_token")
        );
    }

    #[test]
    fn telegram_global_settings_surface_keeps_preload_form_behavior() {
        let registrations = telegram_plugin_surfaces()
            .iter()
            .map(|r| r.to_wire("plugin.telegram"))
            .collect::<Vec<_>>();
        let settings_surface = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| {
                surface.descriptor.surface_id.as_str() == "notifications.telegram.global-settings"
            })
            .expect("notifications.telegram.global-settings surface should be present");

        assert_eq!(
            settings_surface.descriptor.slot,
            surfaces::SLOT_SETTINGS_BELOW_GLOBAL
        );
        assert!(matches!(
            settings_surface.descriptor.root_node,
            surfaces::SurfaceNode::Form { ref interaction_id, .. }
                if interaction_id.as_str() == "settings"
        ));

        let save = settings_surface
            .interactions
            .iter()
            .find(|interaction| {
                interaction.interaction_id.as_str() == "settings"
                    && interaction.effective_http_method() == surfaces::InteractionHttpMethod::Put
            })
            .expect("save global telegram (`settings` PUT) interaction should exist");
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
            Some("settings")
        );
        assert!(settings_surface.interactions.iter().any(|interaction| {
            interaction.interaction_id.as_str() == "settings"
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

    #[tokio::test]
    async fn map_send_error_strips_url_bearing_token() {
        let token = "SENTINEL-bot-token-3f9a";
        let err = reqwest::Client::new()
            .get(format!("http://127.0.0.1:1/bot{token}/sendMessage"))
            .send()
            .await
            .expect_err("connection to 127.0.0.1:1 must be refused");
        assert!(
            format!("{err:?}").contains(token),
            "fixture precondition: raw reqwest error must carry the token"
        );
        let mapped = map_send_error(err);
        assert!(
            !format!("{mapped}").contains(token),
            "Display must not leak the token"
        );
        assert!(
            !format!("{mapped:?}").contains(token),
            "Debug must not leak the token"
        );
    }
}
