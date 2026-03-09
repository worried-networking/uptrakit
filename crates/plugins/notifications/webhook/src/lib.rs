//! Webhook notification plugin.
//!
//! POSTs a JSON payload to the configured URL. Optionally signs the payload
//! with HMAC-SHA256 and includes the signature in the `X-Uptrakit-Signature`
//! header.

use std::sync::Arc;

use async_trait::async_trait;
use hmac::Mac as _;
use rootcause::prelude::*;
use sha2::Sha256;
use uptrakit_shared_types::network::is_private_host;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPlugin, NotificationPluginError, Result,
};

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

/// Returns an error if `key` matches any entry in [`BLOCKED_HEADERS`].
///
/// Called from both [`WebhookPlugin::deliver`] (defence-in-depth) and
/// [`WebhookPlugin::validate_config`] (pre-save validation).
fn check_header_allowed(key: &str) -> Result<()> {
    let lower = key.to_lowercase();
    if BLOCKED_HEADERS.contains(&lower.as_str()) {
        bail!(NotificationPluginError::InvalidConfig(format!(
            "header '{key}' is not allowed in webhook custom headers"
        )));
    }
    Ok(())
}

/// Webhook notification plugin.
///
/// Sends a JSON POST request to the URL specified in the channel config.
/// When a `secret` field is present in the config, the request body is signed
/// with HMAC-SHA256 and the signature is included in the `X-Uptrakit-Signature`
/// header as `sha256=<hex>`.
pub struct WebhookPlugin {
    http: reqwest::Client,
    allow_private_urls: bool,
}

impl WebhookPlugin {
    /// Create a new webhook plugin with a pre-configured HTTP client.
    ///
    /// When `allow_private_urls` is `true`, the private-host check in
    /// [`validate_config`](NotificationPlugin::validate_config) is skipped.
    /// This is intended for single-tenant / self-hosted deployments where
    /// internal webhook URLs (e.g. a Mattermost on the LAN) are legitimate.
    /// The header blocklist is always enforced regardless of this flag.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationPluginError::HttpClientBuild`] if the HTTP client
    /// cannot be constructed.
    pub fn new(allow_private_urls: bool) -> Result<Self> {
        let resolver = if allow_private_urls {
            SsrfSafeResolver::permissive()
        } else {
            SsrfSafeResolver::new()
        };
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(resolver))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| report!(NotificationPluginError::HttpClientBuild(e.to_string())))?;
        Ok(Self {
            http,
            allow_private_urls,
        })
    }
}

#[async_trait]
impl NotificationPlugin for WebhookPlugin {
    fn channel_type(&self) -> &'static str {
        "webhook"
    }

    async fn deliver(&self, config: &serde_json::Value, message: &DeliveryMessage) -> Result<()> {
        let url = config["url"].as_str().ok_or_else(|| {
            report!(NotificationPluginError::InvalidConfig(
                "missing 'url'".to_string()
            ))
        })?;

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
            .map_err(|e| report!(NotificationPluginError::Serialization(e.to_string())))?;

        let mut req = self
            .http
            .post(url)
            .header("Content-Type", "application/json");

        // Add custom headers from config.  Defence-in-depth: enforce the
        // blocked-header list at delivery time even if validation was skipped
        // (e.g. config written directly to the DB before the blocklist existed).
        if let Some(headers) = config.get("headers").and_then(|h| h.as_object()) {
            for (key, value) in headers {
                check_header_allowed(key)?;
                if let Some(v) = value.as_str() {
                    req = req.header(key.as_str(), v);
                }
            }
        }

        // HMAC-SHA256 signature if a secret is configured.
        if let Some(secret) = config.get("secret").and_then(|s| s.as_str()) {
            let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
                .map_err(|e| report!(NotificationPluginError::HmacKey(e.to_string())))?;
            mac.update(&body_bytes);
            let signature = uptrakit_shared_types::hex::encode(mac.finalize().into_bytes());
            req = req.header("X-Uptrakit-Signature", format!("sha256={signature}"));
        }

        let resp = req
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| report!(NotificationPluginError::HttpRequest(e.to_string())))?;

        // Reject redirect responses explicitly. Redirect following is disabled
        // to prevent SSRF via attacker-controlled redirect targets.
        if resp.status().is_redirection() {
            let status = resp.status();
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<missing>");
            tracing::warn!(
                %status,
                redirect_target = %location,
                "webhook delivery returned redirect — not following (SSRF protection)"
            );
            bail!(NotificationPluginError::DeliveryFailed(format!(
                "webhook returned redirect {status} to {location} — redirects are not followed"
            )));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                %status,
                response_body = %body_text,
                "webhook delivery returned non-success status"
            );
            bail!(NotificationPluginError::DeliveryFailed(format!(
                "webhook returned {status}: {body_text}"
            )));
        }

        tracing::debug!(url, "webhook notification delivered");
        Ok(())
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<()> {
        let url = config.get("url").and_then(|u| u.as_str()).ok_or_else(|| {
            report!(NotificationPluginError::InvalidConfig(
                "'url' is required".to_string()
            ))
        })?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            bail!(NotificationPluginError::InvalidConfig(
                "'url' must start with http:// or https://".to_string()
            ));
        }

        // Private-host SSRF check (skipped when allow_private_urls is true).
        if !self.allow_private_urls
            && let Ok(parsed) = url::Url::parse(url)
            && let Some(host) = parsed.host_str()
            && is_private_host(host)
        {
            bail!(NotificationPluginError::InvalidConfig(
                "'url' must not point to private/loopback addresses".to_string()
            ));
        }

        // Validate headers structure and enforce blocked-header list.
        if let Some(headers) = config.get("headers") {
            if !headers.is_object() {
                bail!(NotificationPluginError::InvalidConfig(
                    "'headers' must be an object".to_string()
                ));
            }
            if let Some(obj) = headers.as_object() {
                for key in obj.keys() {
                    check_header_allowed(key)?;
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

    /// Helper: create a plugin with private URLs blocked (the default).
    fn plugin() -> WebhookPlugin {
        WebhookPlugin::new(false).expect("client builds")
    }

    /// Helper: create a plugin with private URLs allowed.
    fn plugin_allow_private() -> WebhookPlugin {
        WebhookPlugin::new(true).expect("client builds")
    }

    #[test]
    fn validate_config_requires_url() {
        let config = serde_json::json!({});
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("'url' is required"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_non_http_url() {
        let config = serde_json::json!({"url": "ftp://example.com"});
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("http:// or https://"), "got: {msg}");
    }

    #[test]
    fn validate_config_accepts_https_url() {
        let config = serde_json::json!({"url": "https://example.com/hook"});
        assert!(plugin().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_rejects_private_url() {
        let config = serde_json::json!({"url": "http://192.168.1.1:8080/hook"});
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("private"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_localhost_url() {
        let config = serde_json::json!({"url": "http://localhost:8080/hook"});
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(msg.contains("private"), "got: {msg}");
    }

    #[test]
    fn validate_config_rejects_loopback_url() {
        let config = serde_json::json!({"url": "http://127.0.0.1/hook"});
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_allows_private_url_when_flag_set() {
        let config = serde_json::json!({"url": "http://192.168.1.1:8080/hook"});
        assert!(plugin_allow_private().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_allows_localhost_when_flag_set() {
        let config = serde_json::json!({"url": "http://localhost:8080/hook"});
        assert!(plugin_allow_private().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_rejects_non_object_headers() {
        let config = serde_json::json!({"url": "https://example.com", "headers": "bad"});
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_accepts_object_headers() {
        let config =
            serde_json::json!({"url": "https://example.com", "headers": {"X-Custom": "val"}});
        assert!(plugin().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_rejects_authorization_header() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Authorization": "Bearer token"}
        });
        let result = plugin().validate_config(&config);
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
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_rejects_host_header() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Host": "evil.com"}
        });
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_blocked_header_case_insensitive() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "Bearer token"}
        });
        let result = plugin().validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn validate_config_blocked_header_enforced_even_with_private_urls() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "headers": {"Authorization": "Bearer token"}
        });
        let result = plugin_allow_private().validate_config(&config);
        assert!(result.is_err());
    }

    // ── check_header_allowed unit tests ──────────────────────────────────

    #[test]
    fn check_header_allowed_rejects_blocked_headers() {
        for blocked in BLOCKED_HEADERS {
            let result = check_header_allowed(blocked);
            assert!(result.is_err(), "should reject '{blocked}'");
            let msg = result.unwrap_err().current_context().to_string();
            assert!(
                msg.contains(blocked),
                "error should mention header name; got: {msg}"
            );
        }
    }

    #[test]
    fn check_header_allowed_case_insensitive() {
        assert!(check_header_allowed("Authorization").is_err());
        assert!(check_header_allowed("AUTHORIZATION").is_err());
        assert!(check_header_allowed("AuThOrIzAtIoN").is_err());
    }

    #[test]
    fn check_header_allowed_permits_custom_headers() {
        assert!(check_header_allowed("X-Custom-Header").is_ok());
        assert!(check_header_allowed("X-Api-Key").is_ok());
        assert!(check_header_allowed("Accept").is_ok());
    }

    #[test]
    fn mask_config_secrets_replaces_secret() {
        let config = serde_json::json!({
            "url": "https://example.com",
            "secret": "super-secret-key"
        });
        let masked = plugin().mask_config_secrets(&config);
        assert_eq!(masked["url"], "https://example.com");
        assert_eq!(masked["secret"], "***");
    }

    #[test]
    fn mask_config_secrets_preserves_config_without_secret() {
        let config = serde_json::json!({"url": "https://example.com"});
        let masked = plugin().mask_config_secrets(&config);
        assert_eq!(masked, config);
    }
}
