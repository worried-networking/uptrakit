//! Webhook notification channel.
//!
//! POSTs a JSON payload to the configured URL. Optionally signs the payload
//! with HMAC-SHA256 and includes the signature in the `X-Uptrakit-Signature`
//! header.

use async_trait::async_trait;
use hmac::Mac as _;
use rootcause::prelude::*;
use sha2::Sha256;

use crate::channel::{DeliveryMessage, NotificationChannel};
use crate::error::{self, ChannelError};

type HmacSha256 = hmac::Hmac<Sha256>;

/// Webhook notification channel.
///
/// Sends a JSON POST request to the URL specified in the channel config.
/// When a `secret` field is present in the config, the request body is signed
/// with HMAC-SHA256 and the signature is included in the `X-Uptrakit-Signature`
/// header as `sha256=<hex>`.
pub struct WebhookChannel {
    http: reqwest::Client,
}

impl WebhookChannel {
    /// Create a new webhook channel with a pre-configured HTTP client.
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
impl NotificationChannel for WebhookChannel {
    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> error::Result<()> {
        let url = config["url"]
            .as_str()
            .ok_or_else(|| report!(ChannelError::InvalidConfig("missing 'url'".to_string())))?;

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
            .map_err(|e| report!(ChannelError::Serialization(e.to_string())))?;

        let mut req = self
            .http
            .post(url)
            .header("Content-Type", "application/json");

        // Add custom headers from config.
        if let Some(headers) = config.get("headers").and_then(|h| h.as_object()) {
            for (key, value) in headers {
                if let Some(v) = value.as_str() {
                    req = req.header(key.as_str(), v);
                }
            }
        }

        // HMAC-SHA256 signature if a secret is configured.
        if let Some(secret) = config.get("secret").and_then(|s| s.as_str()) {
            let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
                .map_err(|e| report!(ChannelError::HmacKey(e.to_string())))?;
            mac.update(&body_bytes);
            let signature = uptrakit_shared_types::hex::encode(mac.finalize().into_bytes());
            req = req.header("X-Uptrakit-Signature", format!("sha256={signature}"));
        }

        let resp = req
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| report!(ChannelError::HttpRequest(e.to_string())))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                %status,
                response_body = %body_text,
                "webhook delivery returned non-success status"
            );
            bail!(ChannelError::DeliveryFailed(format!(
                "webhook returned {status}: {body_text}"
            )));
        }

        tracing::debug!(url, "webhook notification delivered");
        Ok(())
    }

    fn validate_config(&self, config: &serde_json::Value) -> error::Result<()> {
        let url = config
            .get("url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| report!(ChannelError::InvalidConfig("'url' is required".to_string())))?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            bail!(ChannelError::InvalidConfig(
                "'url' must start with http:// or https://".to_string()
            ));
        }

        if let Some(headers) = config.get("headers")
            && !headers.is_object()
        {
            bail!(ChannelError::InvalidConfig(
                "'headers' must be an object".to_string()
            ));
        }

        Ok(())
    }

    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        let mut masked = config.clone();
        if let Some(obj) = masked.as_object_mut()
            && obj.contains_key("secret")
        {
            obj.insert("secret".to_string(), serde_json::json!("***"));
        }
        masked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_config_requires_url() {
        let channel = WebhookChannel::new().expect("client builds");
        let config = serde_json::json!({});
        let result = channel.validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("'url' is required"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_non_http_url() {
        let channel = WebhookChannel::new().expect("client builds");
        let config = serde_json::json!({"url": "ftp://example.com"});
        let result = channel.validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("http:// or https://"),
            "got: {msg}"
        );
    }

    #[test]
    fn validate_config_accepts_https_url() {
        let channel = WebhookChannel::new().expect("client builds");
        let config = serde_json::json!({"url": "https://example.com/hook"});
        assert!(channel.validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_accepts_http_url() {
        let channel = WebhookChannel::new().expect("client builds");
        let config = serde_json::json!({"url": "http://localhost:8080/hook"});
        assert!(channel.validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_rejects_non_object_headers() {
        let channel = WebhookChannel::new().expect("client builds");
        let config = serde_json::json!({"url": "https://example.com", "headers": "bad"});
        let result = channel.validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_accepts_object_headers() {
        let channel = WebhookChannel::new().expect("client builds");
        let config =
            serde_json::json!({"url": "https://example.com", "headers": {"X-Custom": "val"}});
        assert!(channel.validate_config(&config).is_ok());
    }

    #[test]
    fn mask_config_secrets_replaces_secret() {
        let channel = WebhookChannel::new().expect("client builds");
        let config = serde_json::json!({
            "url": "https://example.com",
            "secret": "super-secret-key"
        });
        let masked = channel.mask_config_secrets(&config);
        assert_eq!(masked["url"], "https://example.com");
        assert_eq!(masked["secret"], "***");
    }

    #[test]
    fn mask_config_secrets_preserves_config_without_secret() {
        let channel = WebhookChannel::new().expect("client builds");
        let config = serde_json::json!({"url": "https://example.com"});
        let masked = channel.mask_config_secrets(&config);
        assert_eq!(masked, config);
    }
}
