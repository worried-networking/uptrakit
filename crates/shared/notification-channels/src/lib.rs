//! Pluggable notification channel implementations for Uptrakit.
//!
//! This crate provides the [`NotificationChannel`] trait and concrete
//! implementations for delivering notifications via different transports
//! (webhook, Telegram, email). Channels are registered in a
//! [`ChannelRegistry`] and looked up by type name at dispatch time.

mod channel;
#[cfg(feature = "email")]
mod email;
mod error;
mod registry;
#[cfg(feature = "telegram")]
mod telegram;
#[cfg(feature = "webhook")]
mod webhook;

pub use channel::{DeliveryMessage, MessageAction, NotificationChannel};
pub use error::ChannelError;
pub use registry::{ChannelRegistry, ChannelRegistryConfig};

#[cfg(feature = "email")]
pub use email::EmailChannel;
#[cfg(feature = "telegram")]
pub use telegram::TelegramChannel;
#[cfg(feature = "webhook")]
pub use webhook::WebhookChannel;

/// Escape HTML-significant characters for safe interpolation into HTML bodies.
///
/// Replaces the five characters that can break HTML structure or enable
/// injection: `& < > " '`. Use this on any user-controlled string before
/// embedding it in `body_html` templates.
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_html_all_five_chars() {
        assert_eq!(
            escape_html("<script>alert('xss' & \"evil\")</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27; &amp; &quot;evil&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn escape_html_preserves_plain_text() {
        assert_eq!(escape_html("hello world 123"), "hello world 123");
    }

    #[test]
    fn escape_html_empty_string() {
        assert_eq!(escape_html(""), "");
    }
}
