use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::PluginConfig;

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
    fn validate(&self) -> Result<(), String> {
        if self.chat_id.is_empty() {
            return Err("'chat_id' is required".to_string());
        }
        Ok(())
    }

    fn with_secrets_masked(mut self) -> Self {
        if self.bot_token.is_some() {
            self.bot_token = Some("***".to_string());
        }
        if self.webhook_secret.is_some() {
            self.webhook_secret = Some("***".to_string());
        }
        self
    }

    fn restore_secrets_from(&mut self, existing: &Self) {
        if self.bot_token.as_deref() == Some("***") {
            self.bot_token = existing.bot_token.clone();
        }
        if self.webhook_secret.as_deref() == Some("***") {
            self.webhook_secret = existing.webhook_secret.clone();
        }
    }
}

#[cfg(test)]
mod tests {
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
        assert!(msg.contains("'chat_id'"), "got: {msg}");
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
    fn mask_secrets_replaces_bot_token() {
        let config = TelegramChannelConfig {
            bot_token: Some("123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11".to_string()),
            chat_id: "-100123456".to_string(),
            ..Default::default()
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.bot_token.as_deref(), Some("***"));
        assert_eq!(masked.chat_id, "-100123456");
    }

    #[test]
    fn mask_secrets_replaces_webhook_secret() {
        let config = TelegramChannelConfig {
            bot_token: Some("tok".to_string()),
            chat_id: "id".to_string(),
            webhook_secret: Some("s3cret".to_string()),
        };
        let masked = config.with_secrets_masked();
        assert_eq!(masked.bot_token.as_deref(), Some("***"));
        assert_eq!(masked.webhook_secret.as_deref(), Some("***"));
    }

    #[test]
    fn mask_secrets_leaves_none_fields_as_none() {
        let config = TelegramChannelConfig {
            chat_id: "-100123".to_string(),
            ..Default::default()
        };
        let masked = config.with_secrets_masked();
        assert!(masked.bot_token.is_none());
        assert!(masked.webhook_secret.is_none());
    }

    #[test]
    fn restore_secrets_from_existing() {
        let existing = TelegramChannelConfig {
            bot_token: Some("real-token".to_string()),
            chat_id: "-100123".to_string(),
            webhook_secret: Some("real-secret".to_string()),
        };
        let mut incoming = TelegramChannelConfig {
            bot_token: Some("***".to_string()),
            chat_id: "-100123".to_string(),
            webhook_secret: Some("***".to_string()),
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.bot_token.as_deref(), Some("real-token"));
        assert_eq!(incoming.webhook_secret.as_deref(), Some("real-secret"));
    }

    #[test]
    fn restore_secrets_preserves_new_values() {
        let existing = TelegramChannelConfig {
            bot_token: Some("old-token".to_string()),
            chat_id: "-100123".to_string(),
            ..Default::default()
        };
        let mut incoming = TelegramChannelConfig {
            bot_token: Some("new-token".to_string()),
            chat_id: "-100123".to_string(),
            ..Default::default()
        };
        incoming.restore_secrets_from(&existing);
        assert_eq!(incoming.bot_token.as_deref(), Some("new-token"));
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
