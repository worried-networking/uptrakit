use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

/// Per-channel config for Telegram notification channels.
///
/// The `bot_token` is optional at the channel level — when absent, the global
/// bot token from `global_settings` is used at delivery time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramChannelConfig {
    /// Bot API token for this channel. Falls back to the global token when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    /// Telegram chat ID to send messages to (e.g., `-1001234567890`).
    #[serde(default)]
    pub chat_id: String,
    /// Secret token for verifying Telegram Bot API webhook callbacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
}

impl PluginConfig for TelegramChannelConfig {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if self.chat_id.is_empty() {
            return Err(PluginConfigValidationError::invalid_field(
                "chat_id",
                "is required",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;

    #[test]
    fn validate_accepts_config_without_bot_token() {
        let config = TelegramChannelConfig {
            chat_id: "-100123".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_requires_chat_id() {
        let config = TelegramChannelConfig {
            bot_token: Some("123:ABC".to_string()),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert_eq!(msg.field(), Some("chat_id"));
        assert!(msg.to_string().contains("is required"), "got: {msg}");
    }

    #[test]
    fn validate_rejects_empty_chat_id() {
        let config = TelegramChannelConfig {
            bot_token: Some("123:ABC".to_string()),
            chat_id: String::new(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_config() {
        let config = TelegramChannelConfig {
            bot_token: Some("123:ABC".to_string()),
            chat_id: "-100123".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn deserialize_minimal_config() {
        let config: TelegramChannelConfig =
            serde_json::from_str(r#"{"chat_id": "-100123"}"#).expect("deserialize");
        assert_eq!(config.chat_id, "-100123");
        assert!(config.bot_token.is_none());
        assert!(config.webhook_secret.is_none());
    }

    #[test]
    fn deserialize_full_config() {
        let config: TelegramChannelConfig = serde_json::from_str(
            r#"{"bot_token": "123:ABC", "chat_id": "-100123", "webhook_secret": "s3cret"}"#,
        )
        .expect("deserialize");
        assert_eq!(config.bot_token.as_deref(), Some("123:ABC"));
        assert_eq!(config.chat_id, "-100123");
        assert_eq!(config.webhook_secret.as_deref(), Some("s3cret"));
    }

    #[test]
    fn serialize_skips_none_fields() {
        let config = TelegramChannelConfig {
            chat_id: "-100123".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert!(json.get("bot_token").is_none());
        assert!(json.get("webhook_secret").is_none());
        assert_eq!(json["chat_id"], "-100123");
    }
}
