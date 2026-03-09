//! Core trait and types for notification plugins.
//!
//! This crate defines the [`NotificationPlugin`] trait implemented by each
//! notification channel (webhook, Telegram, email, etc.) and the
//! [`DeliveryMessage`] struct used to pass channel-agnostic notification
//! content to plugins.

mod error;
mod traits;

pub use error::{NotificationPluginError, Result};
pub use traits::{DeliveryMessage, MessageAction, NotificationPlugin};

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
