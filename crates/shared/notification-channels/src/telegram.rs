//! Telegram notification channel.
//!
//! Sends messages to a configured Telegram chat via the Bot API. Renders
//! action buttons as inline keyboard buttons.

use async_trait::async_trait;
use rootcause::prelude::*;

use crate::channel::{DeliveryMessage, NotificationChannel};
use crate::error::{self, ChannelError};

/// Telegram notification channel using the Bot API.
///
/// Sends messages to a configured chat via `sendMessage`. When the message
/// includes actions, they are rendered as inline keyboard buttons.
pub struct TelegramChannel {
    http: reqwest::Client,
}

impl TelegramChannel {
    /// Create a new Telegram channel with a pre-configured HTTP client.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::HttpClientBuild`] if the HTTP client cannot be
    /// constructed.
    pub fn new() -> error::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| report!(ChannelError::HttpClientBuild(e.to_string())))?;
        Ok(Self { http })
    }
}

#[async_trait]
impl NotificationChannel for TelegramChannel {
    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> error::Result<()> {
        let bot_token = config["bot_token"].as_str().ok_or_else(|| {
            report!(ChannelError::InvalidConfig(
                "missing 'bot_token'".to_string()
            ))
        })?;
        let chat_id = config["chat_id"]
            .as_str()
            .ok_or_else(|| report!(ChannelError::InvalidConfig("missing 'chat_id'".to_string())))?;

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
            .map_err(|e| report!(ChannelError::HttpRequest(e.to_string())))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                %status,
                response_body = %body_text,
                "telegram delivery returned non-success status"
            );
            bail!(ChannelError::DeliveryFailed(format!(
                "telegram API returned {status}: {body_text}"
            )));
        }

        tracing::debug!(chat_id, "telegram notification delivered");
        Ok(())
    }

    fn validate_config(&self, config: &serde_json::Value) -> error::Result<()> {
        if config
            .get("bot_token")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
        {
            bail!(ChannelError::InvalidConfig(
                "'bot_token' is required".to_string()
            ));
        }

        if config
            .get("chat_id")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
        {
            bail!(ChannelError::InvalidConfig(
                "'chat_id' is required".to_string()
            ));
        }

        Ok(())
    }

    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        let mut masked = config.clone();
        if let Some(obj) = masked.as_object_mut() {
            if obj.contains_key("bot_token") {
                obj.insert("bot_token".to_string(), serde_json::json!("***"));
            }
            if obj.contains_key("webhook_secret") {
                obj.insert("webhook_secret".to_string(), serde_json::json!("***"));
            }
        }
        masked
    }
}

/// Re-export the shared HTML escaping function for use within this module.
use crate::escape_html;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_config_requires_bot_token() {
        let channel = TelegramChannel::new().expect("client builds");
        let config = serde_json::json!({"chat_id": "12345"});
        let result = channel.validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("'bot_token'"), "got: {msg}");
    }

    #[test]
    fn validate_config_requires_chat_id() {
        let channel = TelegramChannel::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "123:ABC"});
        let result = channel.validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("'chat_id'"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_empty_bot_token() {
        let channel = TelegramChannel::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "", "chat_id": "12345"});
        let result = channel.validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_rejects_empty_chat_id() {
        let channel = TelegramChannel::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "123:ABC", "chat_id": ""});
        let result = channel.validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_accepts_valid_config() {
        let channel = TelegramChannel::new().expect("client builds");
        let config = serde_json::json!({"bot_token": "123:ABC", "chat_id": "-100123"});
        assert!(channel.validate_config(&config).is_ok());
    }

    #[test]
    fn mask_config_secrets_replaces_bot_token() {
        let channel = TelegramChannel::new().expect("client builds");
        let config = serde_json::json!({
            "bot_token": "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11",
            "chat_id": "-100123456"
        });
        let masked = channel.mask_config_secrets(&config);
        assert_eq!(masked["bot_token"], "***");
        assert_eq!(masked["chat_id"], "-100123456");
    }

    #[test]
    fn mask_config_secrets_replaces_webhook_secret() {
        let channel = TelegramChannel::new().expect("client builds");
        let config = serde_json::json!({
            "bot_token": "tok",
            "chat_id": "id",
            "webhook_secret": "s3cret"
        });
        let masked = channel.mask_config_secrets(&config);
        assert_eq!(masked["bot_token"], "***");
        assert_eq!(masked["webhook_secret"], "***");
    }

    #[test]
    fn escape_html_escapes_special_chars() {
        assert_eq!(escape_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn escape_html_preserves_plain_text() {
        assert_eq!(escape_html("hello world"), "hello world");
    }
}
