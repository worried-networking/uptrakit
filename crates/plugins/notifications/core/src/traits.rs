//! Core notification message types.

/// Channel-agnostic notification message built by the dispatcher.
///
/// Channel implementations render this into their native format
/// (JSON payload for webhooks, HTML message for Telegram, etc.).
#[non_exhaustive]
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
#[non_exhaustive]
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

impl MessageAction {
    /// Create a new [`MessageAction`].
    pub fn new(
        label: impl Into<String>,
        callback_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            callback_url: callback_url.into(),
            token: token.into(),
        }
    }
}

impl DeliveryMessage {
    /// Create a new [`DeliveryMessage`].
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        body_html: Option<String>,
        event_payload: serde_json::Value,
        actions: Vec<MessageAction>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            body_html,
            event_payload,
            actions,
        }
    }
}
