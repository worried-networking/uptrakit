#![expect(
    clippy::expect_used,
    reason = "infallible literal surface ID and value constructions; panic would indicate a programming error in the surface manifest"
)]
//! Webhook notification plugin implementation and `declare_plugin!` invocation.

use std::sync::Arc;

use async_trait::async_trait;
use hmac::Mac as _;
use rootcause::prelude::*;
use sha2::Sha256;

use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError, Result};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, InteractionDelivery, PluginFamily, PluginHttpClientConfig, PluginSurface,
    PluginSurfaceRegistration, RegisteredInteraction, SsrfMode, build_plugin_http_client,
    declare_plugin, surfaces,
};
use uptrakit_shared_types::access::actions;

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
        let http = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-notification-webhook/",
                env!("CARGO_PKG_VERSION")
            ),
            ssrf_mode: if allow_private_urls {
                SsrfMode::Permissive
            } else {
                SsrfMode::Strict
            },
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

        let resp = req.body(body_bytes).send().await.map_err(map_send_error)?;

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

fn webhook_plugin_surfaces() -> Vec<PluginSurfaceRegistration> {
    let data_source_id =
        surfaces::DataSourceId::new("channels").expect("literal data source id is valid");
    let webhook_surface = PluginSurface {
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
            .required_action(actions::NOTIFICATIONS_READ)
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
                            surfaces::SurfaceTableColumn::new("url", "URL"),
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
                InteractionDelivery::PluginHandled(crate::surfaces::webhook_list_handler),
            ),
            RegisteredInteraction::new(
                {
                    let mut i = surfaces::InteractionDescriptor::new(
                        surfaces::InteractionId::new("channels")
                            .expect("literal interaction id is valid"),
                        surfaces::InteractionKind::FormSubmit,
                        "Add Webhook",
                        surfaces::InteractionTransport::ControllerLocal,
                    );
                    i.required_action = Some(actions::NOTIFICATIONS_MANAGE_STR.to_string());
                    i.input_schema = Some(surfaces::SchemaContract::Object);
                    i.result_schema = Some(surfaces::SchemaContract::Any);
                    i.sensitive_fields = vec!["secret".to_string()];
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
                                help_text: Some(
                                    "Optional HMAC secret for request signing".to_string(),
                                ),
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
                    i.required_action = Some(actions::NOTIFICATIONS_MANAGE_STR.to_string());
                    i.input_schema = Some(surfaces::SchemaContract::Object);
                    i.result_schema = Some(surfaces::SchemaContract::Any);
                    i.sensitive_fields = vec!["secret".to_string()];
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
                                help_text: Some(
                                    "Leave unchanged to keep current secret".to_string(),
                                ),
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
                    i.required_action = Some(actions::NOTIFICATIONS_MANAGE_STR.to_string());
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
                    i.required_action = Some(actions::NOTIFICATIONS_MANAGE_STR.to_string());
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
    };

    let surfaces = vec![webhook_surface];
    vec![PluginSurfaceRegistration { surfaces }]
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(WebhookPlugin, WebhookChannelConfig, "notifications.webhook", {
    display_name: "Webhook",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    sensitive_paths: ["secret"],
    roles: [NotificationTransport],
    notification_transport: create_webhook_transport,
    raw_settings_keys: &[],
    surfaces: {
        registrations: webhook_plugin_surfaces,
    },
});

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta, surfaces};

    #[test]
    fn plugin_type_id() {
        let plugin = WebhookPlugin::new(false).expect("client builds");
        assert_eq!(plugin.plugin_type_id().as_str(), "notifications.webhook");
    }

    #[test]
    fn both_ssrf_modes_build_via_shared_builder() {
        // Conformance: `new()` routes through `build_plugin_http_client` for both
        // the Strict (`false`) and Permissive (`true`) branches. reqwest::Client
        // is not introspectable post-build, so the redirect-none/User-Agent
        // guarantee comes from the builder; here we assert only that both call
        // paths construct successfully (the Permissive branch is otherwise
        // untested by construction elsewhere).
        assert!(
            WebhookPlugin::new(false).is_ok(),
            "Strict client must build"
        );
        assert!(
            WebhookPlugin::new(true).is_ok(),
            "Permissive client must build"
        );
    }

    // ── Descriptor tests ─────────────────────────────────────────────────

    #[test]
    fn descriptor_type_id() {
        assert_eq!(DESCRIPTOR.type_id, "notifications.webhook");
        assert_eq!(DESCRIPTOR.display_name, "Webhook");
    }

    #[test]
    fn descriptor_declares_sensitive_paths() {
        assert_eq!(DESCRIPTOR.sensitive_paths, ["secret"]);
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
    fn unified_registrations_pair_every_interaction_with_expected_delivery() {
        use uptrakit_plugin_infrastructure_core::InteractionDeliveryKind;
        let registrations = webhook_plugin_surfaces();
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
                "notifications.webhook",
                "channels",
                "get",
                InteractionDeliveryKind::PluginHandled,
            ),
            (
                "notifications.webhook",
                "channels",
                "post",
                InteractionDeliveryKind::ControllerExecutor,
            ),
            (
                "notifications.webhook",
                "channels",
                "put",
                InteractionDeliveryKind::ControllerExecutor,
            ),
            (
                "notifications.webhook",
                "test",
                "post",
                InteractionDeliveryKind::ControllerExecutor,
            ),
            (
                "notifications.webhook",
                "channels",
                "delete",
                InteractionDeliveryKind::ControllerExecutor,
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
            "unexpected total interaction count across webhook_plugin_surfaces()"
        );
    }

    #[test]
    fn descriptor_has_plugin_surface_registrations() {
        let registrations = webhook_plugin_surfaces()
            .iter()
            .map(|r| r.to_wire("notifications.webhook"))
            .collect::<Vec<_>>();
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
        let registrations = webhook_plugin_surfaces()
            .iter()
            .map(|r| r.to_wire("notifications.webhook"))
            .collect::<Vec<_>>();
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
            surfaces::DataSourceKind::ProviderQuery { operation_id } if operation_id == "channels"
        ));

        let find_interaction = |id: &str, method: surfaces::InteractionHttpMethod| {
            webhook_surface
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
                .any(|field| field == "secret")
        );
        assert!(
            find_interaction("channels", surfaces::InteractionHttpMethod::Put)
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

    #[tokio::test]
    async fn map_send_error_strips_url_bearing_secret() {
        let secret = "SENTINEL-webhook-secret-7c1d";
        // Secret embedded in BOTH basic-auth userinfo and the query string: the
        // query copy guarantees the fixture precondition (reqwest always renders
        // it), while userinfo exercises the exact basic-auth leak the spec names.
        // `without_url()` nulls the whole URL, so both positions are covered.
        let err = reqwest::Client::new()
            .get(format!(
                "http://user:{secret}@127.0.0.1:1/hook?token={secret}"
            ))
            .send()
            .await
            .expect_err("connection to 127.0.0.1:1 must be refused");
        assert!(
            format!("{err:?}").contains(secret),
            "fixture precondition: raw reqwest error must carry the secret"
        );
        let mapped = map_send_error(err);
        assert!(
            !format!("{mapped}").contains(secret),
            "Display must not leak the secret"
        );
        assert!(
            !format!("{mapped:?}").contains(secret),
            "Debug must not leak the secret"
        );
    }
}
