//! Webhook notification plugin implementation and `declare_plugin!` invocation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use hmac::Mac as _;
use rootcause::prelude::*;
use sha2::Sha256;
use uptrakit_shared_types::ssrf::{SsrfSafeResolver, webpki_client_config};

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionPlacement, ExtensionUi,
    FieldDef, FieldType, FormDef, PanelPosition, TableColumn,
};
use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError, Result};
use uptrakit_plugin_infrastructure_core::{ConfigModel, PluginFamily, declare_plugin};

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

// ── Extension functions ────────────────────────────────────────────────────

/// Return extension manifests for the webhook plugin.
fn webhook_extension_manifests() -> Vec<ExtensionManifest> {
    vec![
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
        .with_permission("view_notifications"),
    ]
}

/// Return extension action definitions for the webhook plugin.
fn webhook_extension_actions() -> Vec<ActionDef> {
    vec![
        ActionDef::new("list", "List"),
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

/// Extension action handler wrapper for the `declare_plugin!` macro.
///
/// Matches the `ExtensionActionHandler` type signature which receives
/// `descriptor::ExtensionActionContext` (with `db: &dyn Any`). Downcasts
/// the database connection and delegates to `extensions::handle_action`.
fn webhook_handle_extension_action<'a>(
    ctx: &'a uptrakit_plugin_infrastructure_core::descriptor::ExtensionActionContext<'a>,
    extension_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>> {
    Box::pin(async move {
        let db = ctx
            .db
            .downcast_ref::<sea_orm::DatabaseConnection>()
            .ok_or_else(|| "internal error: expected DatabaseConnection".to_string())?;

        // Build the plugin_ops::ExtensionActionContext that the existing handler expects.
        let inner_ctx = uptrakit_plugin_infrastructure_core::ExtensionActionContext {
            db,
            tenant_id: ctx.tenant_id,
            caller_user_id: ctx.caller_user_id,
        };

        crate::extensions::handle_action(&inner_ctx, extension_id, action_id, params).await
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

fn webhook_surface_registrations()
-> Vec<uptrakit_plugin_infrastructure_core::surfaces::SurfaceRegistration> {
    uptrakit_plugin_infrastructure_core::build_plugin_surface_registrations_from_extensions(
        "webhook",
        webhook_extension_manifests(),
        webhook_extension_actions(),
    )
}

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(WebhookPlugin, WebhookChannelConfig, "webhook", {
    display_name: "Webhook",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_webhook_transport,
    owned_extension_ids: &["notifications.webhook"],
    raw_settings_keys: &[],
    extensions: {
        manifests: webhook_extension_manifests,
        actions: webhook_extension_actions,
        handle_action: webhook_handle_extension_action,
    },
    surfaces: {
        registrations: webhook_surface_registrations,
    },
});

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta};

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
    fn descriptor_has_extensions() {
        assert!(DESCRIPTOR.extensions.is_some());
        let ext = DESCRIPTOR.extensions.unwrap();
        assert_eq!(ext.owned_ids, &["notifications.webhook"]);
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
                .any(|id| id == "notifications.webhook")
        );
    }

    // ── Config operations via descriptor ──────────────────────────────────

    #[test]
    fn descriptor_validate_config_requires_url() {
        let config = serde_json::json!({});
        let result = (DESCRIPTOR.config.validate)(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("url"), "got: {msg}");
    }

    #[test]
    fn descriptor_validate_config_rejects_non_http_url() {
        let config = serde_json::json!({"url": "ftp://example.com"});
        let result = (DESCRIPTOR.config.validate)(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("http:// or https://"), "got: {msg}");
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
        assert!(msg.contains("Authorization"), "got: {msg}");
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

    // ── Extension manifests and actions ───────────────────────────────────

    #[test]
    fn extension_manifests_not_empty() {
        let manifests = webhook_extension_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].id, "notifications.webhook");
    }

    #[test]
    fn extension_actions_not_empty() {
        let actions = webhook_extension_actions();
        assert!(!actions.is_empty());
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert!(ids.contains(&"list"));
        assert!(ids.contains(&"create"));
        assert!(ids.contains(&"edit"));
        assert!(ids.contains(&"test"));
        assert!(ids.contains(&"delete"));
    }
}
