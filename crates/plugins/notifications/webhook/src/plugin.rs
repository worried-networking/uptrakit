//! Webhook notification plugin implementation and `declare_plugin!` invocation.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use hmac::Mac as _;
use rootcause::prelude::*;
use sha2::Sha256;
use uptrakit_shared_types::ssrf::{SsrfSafeResolver, webpki_client_config};

use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError, Result};
use uptrakit_plugin_infrastructure_core::{
    ApiSubmitDescriptor, ConfigModel, FormFieldDescriptor, FormFieldType, PluginFamily,
    SurfaceActionDescriptor, SurfaceActionUi, SurfaceFormDescriptor, declare_plugin, surfaces,
};

use crate::config::{BLOCKED_HEADERS, WebhookChannelConfig};

type HmacSha256 = hmac::Hmac<Sha256>;

/// Returns an error if `key` matches any entry in [`BLOCKED_HEADERS`].
///
/// This version returns `notification_plugin_core::Result` for use in
/// the delivery path where `NotificationPluginError` is the error type.
fn check_header_allowed_delivery(key: &str) -> Result<()> {
    let lower = key.to_lowercase();
    if BLOCKED_HEADERS.contains(&lower.as_str()) {
        bail!(NotificationPluginError::InvalidConfig(format!(
            "header '{key}' is not allowed in webhook custom headers"
        )));
    }
    Ok(())
}

/// Webhook notification plugin.
///
/// Sends a JSON POST request to the URL specified in the channel config.
/// When a `secret` field is present in the config, the request body is signed
/// with HMAC-SHA256 and the signature is included in the `X-Uptrakit-Signature`
/// header as `sha256=<hex>`.
pub struct WebhookPlugin {
    http: reqwest::Client,
    #[allow(dead_code)]
    allow_private_urls: bool,
}

impl WebhookPlugin {
    /// Create a new webhook plugin with a pre-configured HTTP client.
    ///
    /// When `allow_private_urls` is `true`, the SSRF-safe DNS resolver is
    /// replaced with a permissive one that allows private/loopback addresses.
    /// This is intended for single-tenant / self-hosted deployments where
    /// internal webhook URLs (e.g. a Mattermost on the LAN) are legitimate.
    /// The header blocklist is always enforced regardless of this flag.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationPluginError::HttpClientBuild`] if the HTTP client
    /// cannot be constructed.
    pub fn new(allow_private_urls: bool) -> Result<Self> {
        let resolver = if allow_private_urls {
            SsrfSafeResolver::permissive()
        } else {
            SsrfSafeResolver::new()
        };
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .use_preconfigured_tls(webpki_client_config())
            .dns_resolver(Arc::new(resolver))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| report!(NotificationPluginError::HttpClientBuild(e.to_string())))?;
        Ok(Self {
            http,
            allow_private_urls,
        })
    }
}

// ── NotificationTransport ──────────────────────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::NotificationTransport for WebhookPlugin {
    async fn deliver(
        &self,
        config: &serde_json::Value,
        _settings: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()> {
        let url = config["url"].as_str().ok_or_else(|| {
            report!(NotificationPluginError::InvalidConfig(
                "missing 'url'".to_string()
            ))
        })?;

        let payload = serde_json::json!({
            "title": message.title,
            "body": message.body,
            "event": message.event_payload,
            "actions": message.actions.iter().map(|a| {
                serde_json::json!({
                    "label": a.label,
                    "callback_url": a.callback_url,
                    "token": a.token,
                })
            }).collect::<Vec<_>>(),
        });

        let body_bytes = serde_json::to_vec(&payload)
            .map_err(|e| report!(NotificationPluginError::Serialization(e.to_string())))?;

        let mut req = self
            .http
            .post(url)
            .header("Content-Type", "application/json");

        // Add custom headers from config.  Defence-in-depth: enforce the
        // blocked-header list at delivery time even if validation was skipped
        // (e.g. config written directly to the DB before the blocklist existed).
        if let Some(headers) = config.get("headers").and_then(|h| h.as_object()) {
            for (key, value) in headers {
                check_header_allowed_delivery(key)?;
                if let Some(v) = value.as_str() {
                    req = req.header(key.as_str(), v);
                }
            }
        }

        // HMAC-SHA256 signature if a secret is configured.
        if let Some(secret) = config.get("secret").and_then(|s| s.as_str()) {
            let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
                .map_err(|e| report!(NotificationPluginError::HmacKey(e.to_string())))?;
            mac.update(&body_bytes);
            let signature = uptrakit_shared_types::hex::encode(mac.finalize().into_bytes());
            req = req.header("X-Uptrakit-Signature", format!("sha256={signature}"));
        }

        let resp = req
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| report!(NotificationPluginError::HttpRequest(e.to_string())))?;

        // Reject redirect responses explicitly. Redirect following is disabled
        // to prevent SSRF via attacker-controlled redirect targets.
        if resp.status().is_redirection() {
            let status = resp.status();
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<missing>");
            tracing::warn!(
                %status,
                redirect_target = %location,
                "webhook delivery returned redirect — not following (SSRF protection)"
            );
            bail!(NotificationPluginError::DeliveryFailed(format!(
                "webhook returned redirect {status} to {location} — redirects are not followed"
            )));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                %status,
                response_body = %body_text,
                "webhook delivery returned non-success status"
            );
            bail!(NotificationPluginError::DeliveryFailed(format!(
                "webhook returned {status}: {body_text}"
            )));
        }

        tracing::debug!(url, "webhook notification delivered");
        Ok(())
    }
}

/// Return surface action definitions for the webhook plugin.
fn webhook_surface_actions() -> Vec<SurfaceActionDescriptor> {
    vec![
        SurfaceActionDescriptor::new("list", "List"),
        SurfaceActionDescriptor::new("create", "Add Webhook")
            .with_permission("manage_notifications")
            .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
                FormFieldDescriptor::new("name", "Name").required(),
                FormFieldDescriptor::new("url", "URL")
                    .required()
                    .with_placeholder("https://example.com/webhook"),
                FormFieldDescriptor::new("secret", "Secret")
                    .with_type(FormFieldType::Password)
                    .sensitive()
                    .with_help_text("Optional HMAC secret for request signing"),
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
        SurfaceActionDescriptor::new("edit", "Edit")
            .with_permission("manage_notifications")
            .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
                FormFieldDescriptor::new("id", "ID").with_type(FormFieldType::Hidden),
                FormFieldDescriptor::new("name", "Name").required(),
                FormFieldDescriptor::new("url", "URL")
                    .required()
                    .with_placeholder("https://example.com/webhook"),
                FormFieldDescriptor::new("secret", "Secret")
                    .with_type(FormFieldType::Password)
                    .sensitive()
                    .with_help_text("Leave unchanged to keep current secret"),
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
                        "url": "{{url}}",
                        "secret": "{{secret}}"
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
    ]
}

/// Surface action handler wrapper for the `declare_plugin!` macro.
///
/// Matches the `SurfaceActionHandler` type signature which receives
/// `SurfaceActionContext` (with `db: &dyn Any`). Downcasts
/// the database connection and delegates to `surfaces::handle_surface_action`.
fn webhook_handle_surface_action<'a>(
    ctx: &'a uptrakit_plugin_infrastructure_core::SurfaceActionContext<'a>,
    surface_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> Pin<
    Box<
        dyn Future<
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

/// Create the webhook transport singleton from catalog config.
fn create_webhook_transport(
    config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<
    Arc<dyn uptrakit_plugin_infrastructure_core::NotificationTransport>,
> {
    Ok(Arc::new(
        WebhookPlugin::new(config.allow_private_urls).map_err(|e| {
            rootcause::report!(
                uptrakit_plugin_infrastructure_core::PluginError::Configuration(e.to_string())
            )
        })?,
    ))
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

fn webhook_surface_registrations() -> Vec<surfaces::SurfaceRegistration> {
    let data_source_id =
        surfaces::DataSourceId::new("data.primary").expect("literal data source id is valid");
    let webhook_surface = surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor::builder()
            .surface_id(
                surfaces::SurfaceId::new("notifications.webhook")
                    .expect("literal surface id is valid"),
            )
            .label("Webhook Channels")
            .priority(500)
            .slot(surfaces::SLOT_SETTINGS_TABS)
            .scope(surfaces::Scope::Global)
            .targeting(surfaces::Targeting::Universal)
            .required_permission("view_notifications")
            .provider_kind(surfaces::ProviderKind::Plugin)
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
            .root_node(surfaces::SurfaceNode::Section {
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
                            surfaces::SurfaceTableColumn::new("name", "Name"),
                            surfaces::SurfaceTableColumn::new("url", "URL"),
                            surfaces::SurfaceTableColumn::new("enabled", "Enabled"),
                            surfaces::SurfaceTableColumn::new("created_at", "Created"),
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
            })
            .build(),
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
                label: "Add Webhook".to_string(),
                required_permission: Some("manage_notifications".to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec!["secret".to_string()],
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
                            key: "url".to_string(),
                            label: "URL".to_string(),
                            field_type: "text".to_string(),
                            required: true,
                            placeholder: Some("https://example.com/webhook".to_string()),
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "secret".to_string(),
                            label: "Secret".to_string(),
                            field_type: "password".to_string(),
                            required: false,
                            placeholder: None,
                            help_text: Some("Optional HMAC secret for request signing".to_string()),
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
                sensitive_fields: vec!["secret".to_string()],
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
                            key: "url".to_string(),
                            label: "URL".to_string(),
                            field_type: "text".to_string(),
                            required: true,
                            placeholder: Some("https://example.com/webhook".to_string()),
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "secret".to_string(),
                            label: "Secret".to_string(),
                            field_type: "password".to_string(),
                            required: false,
                            placeholder: None,
                            help_text: Some("Leave unchanged to keep current secret".to_string()),
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
    };

    let surfaces = vec![webhook_surface];
    vec![surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "plugin.webhook".to_string(),
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

declare_plugin!(WebhookPlugin, WebhookChannelConfig, "webhook", {
    display_name: "Webhook",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_webhook_transport,
    owned_surface_ids: &["notifications.webhook"],
    raw_settings_keys: &[],
    surface_actions: {
        actions: webhook_surface_actions,
        handle_action: webhook_handle_surface_action,
    },
    surfaces: {
        registrations: webhook_surface_registrations,
    },
});

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta, surfaces};

    #[test]
    fn plugin_type_id() {
        let plugin = WebhookPlugin::new(false).expect("client builds");
        assert_eq!(plugin.plugin_type_id().as_str(), "webhook");
    }

    // ── Descriptor tests ─────────────────────────────────────────────────

    #[test]
    fn descriptor_type_id() {
        assert_eq!(DESCRIPTOR.type_id, "webhook");
        assert_eq!(DESCRIPTOR.display_name, "Webhook");
    }

    #[test]
    fn descriptor_family_and_config_model() {
        assert_eq!(DESCRIPTOR.family, PluginFamily::Notification);
        assert_eq!(DESCRIPTOR.config_model, ConfigModel::NotificationChannel);
    }

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::NotificationDelivery)
        );
    }

    #[test]
    fn descriptor_has_notification_transport() {
        assert!(DESCRIPTOR.roles.notification_transport.is_some());
    }

    #[test]
    fn descriptor_has_no_software_roles() {
        assert!(DESCRIPTOR.roles.discoverer.is_none());
        assert!(DESCRIPTOR.roles.version_detector.is_none());
        assert!(DESCRIPTOR.roles.release_fetcher.is_none());
        assert!(DESCRIPTOR.roles.package_indexer.is_none());
        assert!(DESCRIPTOR.roles.update_executor.is_none());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }

    #[test]
    fn descriptor_has_surface_actions() {
        assert!(DESCRIPTOR.surface_actions.is_some());
        let ext = DESCRIPTOR.surface_actions.unwrap();
        assert_eq!(ext.owned_surface_ids(), &["notifications.webhook"]);
    }

    #[test]
    fn descriptor_has_plugin_surface_registrations() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        assert!(
            !registrations.is_empty(),
            "webhook should contribute at least one shared-surface registration"
        );
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
                .any(|id| id == "notifications.webhook"),
            "notifications.webhook surface should be represented in shared surfaces"
        );
    }

    #[test]
    fn webhook_surface_keeps_table_data_source_and_action_shapes() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let webhook_surface = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| surface.descriptor.surface_id.as_str() == "notifications.webhook")
            .expect("notifications.webhook surface should be present");

        assert_eq!(
            webhook_surface.descriptor.slot,
            surfaces::SLOT_SETTINGS_TABS
        );
        assert_eq!(webhook_surface.data_sources.len(), 1);
        assert!(matches!(
            &webhook_surface.data_sources[0].kind,
            surfaces::DataSourceKind::ProviderQuery { operation_id } if operation_id == "list"
        ));

        let find_interaction = |id: &str| {
            webhook_surface
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
                .any(|field| field == "secret")
        );
        assert!(
            find_interaction("edit")
                .sensitive_fields
                .iter()
                .any(|field| field == "secret")
        );
    }

    // ── Config operations via descriptor ──────────────────────────────────

    #[test]
    fn descriptor_validate_config_requires_url() {
        let config = serde_json::json!({});
        let result = (DESCRIPTOR.config.validate)(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.to_string().contains("url"), "got: {msg}");
    }

    #[test]
    fn descriptor_validate_config_rejects_non_http_url() {
        let config = serde_json::json!({"url": "ftp://example.com"});
        let result = (DESCRIPTOR.config.validate)(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.to_string().contains("http:// or https://"),
            "got: {msg}"
        );
    }

    #[test]
    fn descriptor_validate_config_accepts_https_url() {
        let config = serde_json::json!({"url": "https://example.com/hook"});
        assert!((DESCRIPTOR.config.validate)(&config).is_ok());
    }

    #[test]
    fn descriptor_validate_config_rejects_blocked_header() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Authorization": "Bearer token"}
        });
        let result = (DESCRIPTOR.config.validate)(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.to_string().contains("Authorization"), "got: {msg}");
    }

    #[test]
    fn descriptor_validate_config_accepts_custom_headers() {
        let config =
            serde_json::json!({"url": "https://example.com", "headers": {"X-Custom": "val"}});
        assert!((DESCRIPTOR.config.validate)(&config).is_ok());
    }

    #[test]
    fn descriptor_validate_config_blocked_header_case_insensitive() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "Bearer token"}
        });
        let result = (DESCRIPTOR.config.validate)(&config);
        assert!(result.is_err());
    }

    #[test]
    fn descriptor_mask_secrets_replaces_secret() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "secret": "super-secret-key"
        });
        let masked = (DESCRIPTOR.config.mask_secrets)(&config);
        assert_eq!(masked["url"], "https://example.com");
        assert_eq!(masked["secret"], "***");
    }

    #[test]
    fn descriptor_mask_secrets_preserves_config_without_secret() {
        let config = serde_json::json!({"url": "https://example.com"});
        let masked = (DESCRIPTOR.config.mask_secrets)(&config);
        assert_eq!(masked["url"], "https://example.com");
        // "secret" should not appear
        assert!(masked.get("secret").is_none());
    }

    #[test]
    fn descriptor_restore_secrets() {
        let stored = serde_json::json!({
            "url": "https://example.com",
            "secret": "real-secret"
        });
        let mut incoming = serde_json::json!({
            "url": "https://example.com",
            "secret": "***"
        });
        (DESCRIPTOR.config.restore_secrets)(&mut incoming, &stored);
        assert_eq!(incoming["secret"], "real-secret");
    }

    #[test]
    fn descriptor_restore_secrets_keeps_new_value() {
        let stored = serde_json::json!({
            "url": "https://example.com",
            "secret": "old-secret"
        });
        let mut incoming = serde_json::json!({
            "url": "https://example.com",
            "secret": "new-secret"
        });
        (DESCRIPTOR.config.restore_secrets)(&mut incoming, &stored);
        assert_eq!(incoming["secret"], "new-secret");
    }

    #[test]
    fn descriptor_sample_config() {
        let sample = (DESCRIPTOR.config.sample)();
        assert!(sample.is_object());
        assert_eq!(sample["url"], "");
    }

    // ── Extension actions ─────────────────────────────────────────────────

    #[test]
    fn surface_actions_not_empty() {
        let actions = webhook_surface_actions();
        assert!(!actions.is_empty());
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert!(ids.contains(&"list"));
        assert!(ids.contains(&"create"));
        assert!(ids.contains(&"edit"));
        assert!(ids.contains(&"test"));
        assert!(ids.contains(&"delete"));
    }
}
