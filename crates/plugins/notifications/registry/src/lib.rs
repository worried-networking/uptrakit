//! Notification plugin registry for Uptrakit.
//!
//! Provides [`NotificationPluginRegistry`] that manages compiled-in notification
//! plugins (webhook, Telegram, email) and [`NotificationOps`], the trait used
//! by the web API to interact with notification plugins without depending on
//! concrete implementations.

use std::collections::HashMap;
use std::sync::Arc;

pub use uptrakit_notification_plugin_core::{
    DeliveryMessage, MessageAction, NotificationPlugin, NotificationPluginError, escape_html,
};

#[cfg(feature = "email")]
pub use uptrakit_notification_plugin_email::{SmtpSettingsSnapshot, merge_smtp_into_config};

/// Configuration for the [`NotificationPluginRegistry`].
///
/// Carries deployment-level settings that affect plugin behaviour.
#[derive(Clone, Debug, Default)]
pub struct NotificationRegistryConfig {
    /// When `true`, the webhook plugin allows URLs pointing to private /
    /// loopback / link-local addresses. Intended for single-tenant or
    /// self-hosted deployments where internal webhook targets are legitimate.
    ///
    /// Default: `false` (private URLs are blocked).
    pub allow_private_urls: bool,
}

/// Registry of compiled-in notification plugins.
///
/// Plugins are registered by type name at construction time.
/// The dispatcher looks up plugins by their type string.
pub struct NotificationPluginRegistry {
    plugins: HashMap<&'static str, Arc<dyn NotificationPlugin>>,
}

impl NotificationPluginRegistry {
    /// Create a new registry with all compiled-in plugins.
    ///
    /// Which plugins are available depends on the enabled feature flags
    /// (`webhook`, `telegram`, `email`).
    ///
    /// # Errors
    ///
    /// Returns an error if any compiled-in plugin fails to initialise
    /// (e.g. the HTTP client cannot be built).
    pub fn new(
        config: NotificationRegistryConfig,
    ) -> uptrakit_notification_plugin_core::Result<Self> {
        let mut plugins: HashMap<&'static str, Arc<dyn NotificationPlugin>> = HashMap::new();

        #[cfg(feature = "webhook")]
        {
            plugins.insert(
                "webhook",
                Arc::new(uptrakit_notification_plugin_webhook::WebhookPlugin::new(
                    config.allow_private_urls,
                )?),
            );
        }

        #[cfg(feature = "telegram")]
        {
            plugins.insert(
                "telegram",
                Arc::new(uptrakit_notification_plugin_telegram::TelegramPlugin::new()?),
            );
        }

        #[cfg(feature = "email")]
        {
            plugins.insert(
                "email",
                Arc::new(uptrakit_notification_plugin_email::EmailPlugin),
            );
        }

        Ok(Self { plugins })
    }

    /// Look up a plugin by channel type name.
    #[must_use]
    pub fn get(&self, channel_type: &str) -> Option<Arc<dyn NotificationPlugin>> {
        self.plugins.get(channel_type).cloned()
    }

    /// Return the list of supported channel type names.
    #[must_use]
    pub fn supported_types(&self) -> Vec<&'static str> {
        self.plugins.keys().copied().collect()
    }

    /// Return all registered notification plugin instances.
    pub fn plugins(&self) -> impl Iterator<Item = &Arc<dyn NotificationPlugin>> {
        self.plugins.values()
    }
}

/// Abstraction over the notification plugin registry operations needed by
/// the web API.
///
/// Storing this trait in `AppState` as `Arc<dyn NotificationOps>` decouples
/// route handlers and query helpers from the concrete registry, making them
/// testable in isolation.
pub trait NotificationOps: Send + Sync + 'static {
    /// Look up a plugin by channel type name.
    fn get(&self, channel_type: &str) -> Option<Arc<dyn NotificationPlugin>>;

    /// Return the list of supported channel type names.
    fn supported_types(&self) -> Vec<&'static str>;

    /// Return UI extension manifests for each enabled notification plugin.
    fn extension_manifests(&self) -> Vec<uptrakit_extension_framework::ExtensionManifest>;

    /// Return action definitions for notification extension manifests.
    fn extension_actions(&self) -> Vec<uptrakit_extension_framework::ActionDef>;

    /// Validate channel-specific config JSON.
    fn validate_config(
        &self,
        channel_type: &str,
        config: &serde_json::Value,
    ) -> uptrakit_notification_plugin_core::Result<()>;

    /// Return a copy of the config with secrets replaced by `"***"`.
    fn mask_config_secrets(
        &self,
        channel_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value;

    /// Restore secrets from `stored` into `incoming` wherever `incoming`
    /// contains `"***"` placeholders.
    ///
    /// Returns `incoming` unchanged if the channel type is not recognized.
    fn restore_config_secrets(
        &self,
        channel_type: &str,
        incoming: &serde_json::Value,
        stored: &serde_json::Value,
    ) -> serde_json::Value;
}

impl NotificationOps for NotificationPluginRegistry {
    fn get(&self, channel_type: &str) -> Option<Arc<dyn NotificationPlugin>> {
        self.get(channel_type)
    }

    fn supported_types(&self) -> Vec<&'static str> {
        self.supported_types()
    }

    fn extension_manifests(&self) -> Vec<uptrakit_extension_framework::ExtensionManifest> {
        self.plugins
            .values()
            .flat_map(|p| p.extension_manifests())
            .collect()
    }

    fn extension_actions(&self) -> Vec<uptrakit_extension_framework::ActionDef> {
        self.plugins
            .values()
            .flat_map(|p| p.extension_actions())
            .collect()
    }

    fn validate_config(
        &self,
        channel_type: &str,
        config: &serde_json::Value,
    ) -> uptrakit_notification_plugin_core::Result<()> {
        match self.get(channel_type) {
            Some(plugin) => plugin.validate_config(config),
            None => {
                use rootcause::prelude::*;
                bail!(NotificationPluginError::InvalidConfig(format!(
                    "unsupported channel type: {channel_type}"
                )));
            }
        }
    }

    fn mask_config_secrets(
        &self,
        channel_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        match self.get(channel_type) {
            Some(plugin) => plugin.mask_config_secrets(config),
            None => {
                tracing::warn!(
                    channel_type,
                    "unknown channel type for secret masking — returning empty object"
                );
                serde_json::json!({})
            }
        }
    }

    fn restore_config_secrets(
        &self,
        channel_type: &str,
        incoming: &serde_json::Value,
        stored: &serde_json::Value,
    ) -> serde_json::Value {
        match self.get(channel_type) {
            Some(plugin) => plugin.restore_config_secrets(incoming, stored),
            None => {
                tracing::warn!(
                    channel_type,
                    "unknown channel type for secret restore — returning incoming unchanged"
                );
                incoming.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_creates_successfully() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        let types = registry.supported_types();

        #[cfg(feature = "webhook")]
        assert!(
            types.contains(&"webhook"),
            "webhook should be registered when feature is enabled"
        );

        #[cfg(feature = "telegram")]
        assert!(
            types.contains(&"telegram"),
            "telegram should be registered when feature is enabled"
        );

        #[cfg(feature = "email")]
        assert!(
            types.contains(&"email"),
            "email should be registered when feature is enabled"
        );
    }

    #[test]
    fn registry_get_returns_none_for_unknown() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        assert!(registry.get("nonexistent").is_none());
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn registry_get_returns_webhook_plugin() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        assert!(registry.get("webhook").is_some());
    }

    #[cfg(feature = "telegram")]
    #[test]
    fn registry_get_returns_telegram_plugin() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        assert!(registry.get("telegram").is_some());
    }

    #[cfg(feature = "email")]
    #[test]
    fn registry_get_returns_email_plugin() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        assert!(registry.get("email").is_some());
    }

    #[test]
    fn notification_ops_validate_config_rejects_unknown_type() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        let ops: &dyn NotificationOps = &registry;
        let result = ops.validate_config("nonexistent", &serde_json::json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn notification_ops_mask_returns_empty_for_unknown_type() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        let ops: &dyn NotificationOps = &registry;
        let masked = ops.mask_config_secrets("nonexistent", &serde_json::json!({"key": "val"}));
        assert_eq!(masked, serde_json::json!({}));
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn notification_ops_validate_config_delegates_to_plugin() {
        let registry = NotificationPluginRegistry::new(NotificationRegistryConfig::default())
            .expect("registry should build");
        let ops: &dyn NotificationOps = &registry;

        // Valid webhook config
        let valid = serde_json::json!({"url": "https://example.com/hook"});
        assert!(ops.validate_config("webhook", &valid).is_ok());

        // Invalid webhook config (missing url)
        let invalid = serde_json::json!({});
        assert!(ops.validate_config("webhook", &invalid).is_err());
    }
}
