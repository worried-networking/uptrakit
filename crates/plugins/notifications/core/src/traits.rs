//! Core notification plugin trait and message types.

use async_trait::async_trait;

use crate::error;

/// Channel-agnostic notification message built by the dispatcher.
///
/// Channel implementations render this into their native format
/// (JSON payload for webhooks, HTML message for Telegram, etc.).
#[derive(Clone, Debug)]
pub struct DeliveryMessage {
    /// One-line human-readable title (e.g. "Update Available: nginx").
    pub title: String,
    /// Multi-line plain-text body.
    pub body: String,
    /// Optional HTML-formatted body for channels that support rich text.
    pub body_html: Option<String>,
    /// Machine-readable event payload (for webhook JSON bodies).
    pub event_payload: serde_json::Value,
    /// Optional action buttons. Channels that do not support interactive
    /// elements silently ignore this field.
    pub actions: Vec<MessageAction>,
}

/// A single actionable button attached to a notification.
#[derive(Clone, Debug)]
pub struct MessageAction {
    /// Button label (e.g. "Install Update").
    pub label: String,
    /// Callback URL the channel should use when the button is pressed.
    pub callback_url: String,
    /// Opaque token identifying this action. Channels embed it in their
    /// native callback mechanism.
    pub token: String,
}

/// Trait implemented by each notification plugin (webhook, Telegram, email, etc.).
///
/// The dispatcher calls [`deliver`](NotificationPlugin::deliver) with a
/// fully-built [`DeliveryMessage`]. Plugins render it into their native format
/// and send it. Unsupported features (e.g. action buttons for email) are
/// silently ignored.
#[async_trait]
pub trait NotificationPlugin: Send + Sync {
    /// Returns the channel type identifier (e.g. `"webhook"`, `"telegram"`, `"email"`).
    fn channel_type(&self) -> &'static str;

    /// Deliver a pre-built message using the given channel-specific config.
    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> error::Result<()>;

    /// Validate channel-specific config JSON at create/update time.
    fn validate_config(&self, config: &serde_json::Value) -> error::Result<()>;

    /// Return a copy of the config with secrets replaced by `"***"`.
    #[must_use]
    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value;
}
