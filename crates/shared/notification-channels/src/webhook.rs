//! Webhook notification channel.
//!
//! POSTs a JSON payload to the configured URL. Optionally signs the payload
//! with HMAC-SHA256 and includes the signature in the `X-Uptrakit-Signature`
//! header.

use async_trait::async_trait;
use hmac::Mac as _;
use rootcause::prelude::*;
use sha2::Sha256;
use uptrakit_shared_types::network::is_private_host;

use crate::channel::{DeliveryMessage, NotificationChannel};
use crate::error::{self, ChannelError};

type HmacSha256 = hmac::Hmac<Sha256>;

/// Header names that are always rejected in webhook custom headers,
/// regardless of the `allow_private_urls` setting.
///
/// These headers could be used for credential injection, host header
/// poisoning, or IP spoofing if an attacker controls the header values.
const BLOCKED_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "host",
    "proxy-authorization",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
];

/// Webhook notification channel.
///
/// Sends a JSON POST request to the URL specified in the channel config.
/// When a `secret` field is present in the config, the request body is signed
/// with HMAC-SHA256 and the signature is included in the `X-Uptrakit-Signature`
/// header as `sha256=<hex>`.
pub struct WebhookChannel {
    http: reqwest::Client,
    allow_private_urls: bool,
}

impl WebhookChannel {
    /// Create a new webhook channel with a pre-configured HTTP client.
    ///
    /// When `allow_private_urls` is `true`, the private-host check in
    /// [`validate_config`](NotificationChannel::validate_config) is skipped.
    /// This is intended for single-tenant / self-hosted deployments where
    /// internal webhook URLs (e.g. a Mattermost on the LAN) are legitimate.
    /// The header blocklist is always enforced regardless of this flag.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::HttpClientBuild`] if the HTTP client cannot be
    /// constructed.
    pub fn new(allow_private_urls: bool) -> error::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| report!(ChannelError::HttpClientBuild(e.to_string())))?;
        Ok(Self {
            http,
            allow_private_urls,
        })
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

        // Private-host SSRF check (skipped when allow_private_urls is true).
        if !self.allow_private_urls {
            if let Ok(parsed) = url::Url::parse(url) {
                if let Some(host) = parsed.host_str() {
                    if is_private_host(host) {
                        bail!(ChannelError::InvalidConfig(
                            "'url' must not point to private/loopback addresses".to_string()
                        ));
                    }
                }
            }
        }

        // Validate headers structure and enforce blocked-header list.
        if let Some(headers) = config.get("headers") {
            if !headers.is_object() {
                bail!(ChannelError::InvalidConfig(
                    "'headers' must be an object".to_string()
                ));
            }
            if let Some(obj) = headers.as_object() {
                for key in obj.keys() {
                    let lower = key.to_lowercase();
                    if BLOCKED_HEADERS.contains(&lower.as_str()) {
                        bail!(ChannelError::InvalidConfig(format!(
                            "header '{key}' is not allowed in webhook custom headers"
                        )));
                    }
                }
            }
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

    /// Helper: create a channel with private URLs blocked (the default).
    fn channel() -> WebhookChannel {
        WebhookChannel::new(false).expect("client builds")
    }

    /// Helper: create a channel with private URLs allowed.
    fn channel_allow_private() -> WebhookChannel {
        WebhookChannel::new(true).expect("client builds")
    }

    #[test]
    fn validate_config_requires_url() {
        let config = serde_json::json!({});
        let result = channel().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("'url' is required"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_non_http_url() {
        let config = serde_json::json!({"url": "ftp://example.com"});
        let result = channel().validate_config(&config);
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
        let config = serde_json::json!({"url": "https://example.com/hook"});
        assert!(channel().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_rejects_private_url() {
        let config = serde_json::json!({"url": "http://192.168.1.1:8080/hook"});
        let result = channel().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("private"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_localhost_url() {
        let config = serde_json::json!({"url": "http://localhost:8080/hook"});
        let result = channel().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("private"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_loopback_url() {
        let config = serde_json::json!({"url": "http://127.0.0.1/hook"});
        let result = channel().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_allows_private_url_when_flag_set() {
        let config = serde_json::json!({"url": "http://192.168.1.1:8080/hook"});
        assert!(channel_allow_private().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_allows_localhost_when_flag_set() {
        let config = serde_json::json!({"url": "http://localhost:8080/hook"});
        assert!(channel_allow_private().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_rejects_non_object_headers() {
        let config = serde_json::json!({"url": "https://example.com", "headers": "bad"});
        let result = channel().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_accepts_object_headers() {
        let config =
            serde_json::json!({"url": "https://example.com", "headers": {"X-Custom": "val"}});
        assert!(channel().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_rejects_authorization_header() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Authorization": "Bearer token"}
        });
        let result = channel().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("Authorization"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_cookie_header() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Cookie": "session=abc"}
        });
        let result = channel().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_rejects_host_header() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Host": "evil.com"}
        });
        let result = channel().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_blocked_header_case_insensitive() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "Bearer token"}
        });
        let result = channel().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_blocked_header_enforced_even_with_private_urls() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Authorization": "Bearer token"}
        });
        let result = channel_allow_private().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn mask_config_secrets_replaces_secret() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "secret": "super-secret-key"
        });
        let masked = channel().mask_config_secrets(&config);
        assert_eq!(masked["url"], "https://example.com");
        assert_eq!(masked["secret"], "***");
    }

    #[test]
    fn mask_config_secrets_preserves_config_without_secret() {
        let config = serde_json::json!({"url": "https://example.com"});
        let masked = channel().mask_config_secrets(&config);
        assert_eq!(masked, config);
    }
}
