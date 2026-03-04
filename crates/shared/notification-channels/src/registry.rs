//! Registry of compiled-in notification channels.
//!
//! Channels are registered by type name at construction time. The notification
//! dispatcher looks up channels by their type string when delivering a message.

use std::collections::HashMap;
use std::sync::Arc;

use crate::channel::NotificationChannel;
use crate::error;

/// Configuration for the [`ChannelRegistry`].
///
/// Carries deployment-level settings that affect channel behaviour.
#[derive(Clone, Debug, Default)]
pub struct ChannelRegistryConfig {
    /// When `true`, the webhook channel allows URLs pointing to private /
    /// loopback / link-local addresses. Intended for single-tenant or
    /// self-hosted deployments where internal webhook targets are legitimate.
    ///
    /// Default: `false` (private URLs are blocked).
    pub allow_private_urls: bool,
}

/// Registry of compiled-in notification channels.
///
/// Channels are registered by type name at construction time.
/// The dispatcher looks up channels by their type string.
pub struct ChannelRegistry {
    channels: HashMap<String, Arc<dyn NotificationChannel>>,
}

impl ChannelRegistry {
    /// Create a new registry with all compiled-in channels.
    ///
    /// Which channels are available depends on the enabled feature flags
    /// (`webhook`, `telegram`, `email`).
    ///
    /// # Errors
    ///
    /// Returns an error if any compiled-in channel fails to initialise
    /// (e.g. the HTTP client cannot be built).
    pub fn new(config: ChannelRegistryConfig) -> error::Result<Self> {
        let mut channels: HashMap<String, Arc<dyn NotificationChannel>> = HashMap::new();

        #[cfg(feature = "webhook")]
        {
            channels.insert(
                "webhook".to_string(),
                Arc::new(crate::webhook::WebhookChannel::new(
                    config.allow_private_urls,
                )?),
            );
        }

        #[cfg(feature = "telegram")]
        {
            channels.insert(
                "telegram".to_string(),
                Arc::new(crate::telegram::TelegramChannel::new()?),
            );
        }

        #[cfg(feature = "email")]
        {
            channels.insert("email".to_string(), Arc::new(crate::email::EmailChannel));
        }

        Ok(Self { channels })
    }

    /// Look up a channel by type name.
    #[must_use]
    pub fn get(&self, channel_type: &str) -> Option<Arc<dyn NotificationChannel>> {
        self.channels.get(channel_type).cloned()
    }

    /// Return the list of supported channel type names.
    #[must_use]
    pub fn supported_types(&self) -> Vec<&str> {
        self.channels.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_creates_successfully() {
        let registry =
            ChannelRegistry::new(ChannelRegistryConfig::default()).expect("registry should build");
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
    }

    #[test]
    fn registry_get_returns_none_for_unknown() {
        let registry =
            ChannelRegistry::new(ChannelRegistryConfig::default()).expect("registry should build");
        assert!(registry.get("nonexistent").is_none());
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn registry_get_returns_webhook_channel() {
        let registry =
            ChannelRegistry::new(ChannelRegistryConfig::default()).expect("registry should build");
        assert!(registry.get("webhook").is_some());
    }

    #[cfg(feature = "telegram")]
    #[test]
    fn registry_get_returns_telegram_channel() {
        let registry =
            ChannelRegistry::new(ChannelRegistryConfig::default()).expect("registry should build");
        assert!(registry.get("telegram").is_some());
    }

    #[cfg(feature = "email")]
    #[test]
    fn registry_get_returns_email_channel() {
        let registry =
            ChannelRegistry::new(ChannelRegistryConfig::default()).expect("registry should build");
        assert!(registry.get("email").is_some());
    }
}
