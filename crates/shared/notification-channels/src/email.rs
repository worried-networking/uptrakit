//! SMTP email notification channel.
//!
//! Delivers notifications via SMTP using `lettre`. Per-channel configuration
//! contains only the recipient addresses; SMTP server credentials and sender
//! identity are supplied at delivery time from the merged global SMTP settings.

use std::time::Duration;

use async_trait::async_trait;
use lettre::message::header::ContentType;
use lettre::message::{MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use rootcause::prelude::*;
use serde::Deserialize;

use crate::channel::{DeliveryMessage, NotificationChannel};
use crate::error::{self, ChannelError};

/// Minimum config required on the per-channel DB row (recipients only).
///
/// Validated at channel create/update time via
/// [`EmailChannel::validate_config`].
#[derive(Debug, Deserialize)]
struct EmailChannelConfig {
    to_addresses: Vec<String>,
}

/// Full merged config passed to [`EmailChannel::deliver`] at dispatch time.
///
/// SMTP credentials come from the global per-tenant SMTP settings and are
/// merged with the per-channel `to_addresses` by the dispatcher before the
/// channel's `deliver` method is called.
#[derive(Debug, Deserialize)]
struct EmailConfig {
    smtp_host: String,
    smtp_port: u16,
    smtp_username: Option<String>,
    smtp_password: Option<String>,
    from_address: String,
    from_name: Option<String>,
    to_addresses: Vec<String>,
    #[serde(default = "default_tls_mode")]
    tls_mode: String,
}

fn default_tls_mode() -> String {
    "starttls".to_string()
}

/// Minimal email format validation: must contain exactly one `@` with
/// non-empty local and domain parts and at least one `.` in the domain.
fn is_valid_email(addr: &str) -> bool {
    let mut parts = addr.splitn(2, '@');
    let local = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

/// Wrap an HTML snippet in a minimal HTML5 document shell.
fn wrap_html(html_body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>{html_body}</body></html>"
    )
}

/// HTML-escape special characters for safe inclusion in an HTML email body.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Email notification channel via SMTP.
///
/// Per-channel config stores only recipient addresses (`to_addresses`). SMTP
/// server credentials and sender identity are merged into the config JSON by
/// the dispatcher from the global per-tenant SMTP settings before
/// [`deliver`](EmailChannel::deliver) is called.
///
/// Enabled by the `email` feature flag.
pub struct EmailChannel;

#[async_trait]
impl NotificationChannel for EmailChannel {
    /// Deliver a notification to all configured recipients.
    ///
    /// The `config` argument must be the *merged* config containing both the
    /// global SMTP settings and the per-channel `to_addresses`.
    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> error::Result<()> {
        let cfg: EmailConfig = serde_json::from_value(config.clone()).map_err(|e| {
            report!(ChannelError::InvalidConfig(format!(
                "failed to deserialize email config: {e}"
            )))
        })?;

        if cfg.to_addresses.is_empty() {
            bail!(ChannelError::InvalidConfig(
                "'to_addresses' must not be empty".to_string()
            ));
        }
        if cfg.smtp_host.is_empty() {
            bail!(ChannelError::InvalidConfig(
                "'smtp_host' must not be empty".to_string()
            ));
        }
        if cfg.from_address.is_empty() {
            bail!(ChannelError::InvalidConfig(
                "'from_address' must not be empty".to_string()
            ));
        }

        let mailer = build_mailer(&cfg)?;

        // Build From header
        let from_header = if let Some(ref name) = cfg.from_name {
            format!("{name} <{}>", cfg.from_address)
        } else {
            cfg.from_address.clone()
        };
        let from_mailbox: lettre::message::Mailbox = from_header.parse().map_err(|e| {
            report!(ChannelError::InvalidConfig(format!(
                "invalid from address: {e}"
            )))
        })?;

        // Build the HTML body — use existing HTML if provided, otherwise escape plain text.
        let html_body = if let Some(ref html) = message.body_html {
            wrap_html(html)
        } else {
            wrap_html(&escape_html(&message.body))
        };

        // Send one message per recipient.
        for to_addr in &cfg.to_addresses {
            let to_mailbox: lettre::message::Mailbox = to_addr.parse().map_err(|e| {
                report!(ChannelError::InvalidConfig(format!(
                    "invalid to address '{to_addr}': {e}"
                )))
            })?;

            let email = Message::builder()
                .from(from_mailbox.clone())
                .to(to_mailbox)
                .subject(&message.title)
                .multipart(
                    MultiPart::alternative()
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_PLAIN)
                                .body(message.body.clone()),
                        )
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_HTML)
                                .body(html_body.clone()),
                        ),
                )
                .map_err(|e| {
                    report!(ChannelError::DeliveryFailed(format!(
                        "failed to build email message: {e}"
                    )))
                })?;

            mailer.send(email).await.map_err(|e| {
                report!(ChannelError::DeliveryFailed(format!(
                    "SMTP delivery failed to {to_addr}: {e}"
                )))
            })?;

            tracing::debug!(to = %to_addr, "email notification delivered");
        }

        Ok(())
    }

    /// Validate per-channel config.
    ///
    /// Only `to_addresses` is stored in the per-channel config. This method
    /// verifies the array is non-empty and each entry is a plausible email
    /// address. SMTP server settings are validated separately when they are
    /// configured via `PUT /api/v1/settings/smtp`.
    fn validate_config(&self, config: &serde_json::Value) -> error::Result<()> {
        let cfg: EmailChannelConfig = serde_json::from_value(config.clone()).map_err(|e| {
            report!(ChannelError::InvalidConfig(format!(
                "failed to deserialize email channel config: {e}"
            )))
        })?;

        if cfg.to_addresses.is_empty() {
            bail!(ChannelError::InvalidConfig(
                "'to_addresses' must not be empty".to_string()
            ));
        }

        for addr in &cfg.to_addresses {
            if !is_valid_email(addr) {
                bail!(ChannelError::InvalidConfig(format!(
                    "invalid email address: '{addr}'"
                )));
            }
        }

        Ok(())
    }

    /// Return config unchanged — per-channel config contains no secrets.
    ///
    /// SMTP credentials are stored in the global per-tenant SMTP settings, not
    /// in the per-channel config.
    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        config.clone()
    }
}

/// Connection timeout applied to every SMTP transport variant.
///
/// Prevents the mailer from hanging indefinitely when the server is
/// unreachable or a firewall silently drops SYN packets.
const SMTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Build an async SMTP mailer from the merged [`EmailConfig`].
///
/// Selects the transport variant based on `tls_mode`:
/// - `"tls"` — SMTPS (implicit TLS, typically port 465)
/// - `"starttls"` — SMTP with STARTTLS upgrade (typically port 587, **default**)
/// - `"none"` — plaintext SMTP (not recommended for production)
fn build_mailer(cfg: &EmailConfig) -> error::Result<AsyncSmtpTransport<Tokio1Executor>> {
    let creds = match (&cfg.smtp_username, &cfg.smtp_password) {
        (Some(user), Some(pass)) if !user.is_empty() => {
            Some(Credentials::new(user.clone(), pass.clone()))
        }
        _ => None,
    };

    let transport = match cfg.tls_mode.as_str() {
        "tls" => {
            let mut builder =
                AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.smtp_host).map_err(|e| {
                    report!(ChannelError::DeliveryFailed(format!(
                        "failed to build TLS SMTP transport: {e}"
                    )))
                })?;
            builder = builder.port(cfg.smtp_port);
            builder = builder.timeout(Some(SMTP_CONNECT_TIMEOUT));
            if let Some(c) = creds {
                builder = builder.credentials(c);
            }
            builder.build()
        }
        "none" => {
            let mut builder =
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host);
            builder = builder.port(cfg.smtp_port);
            builder = builder.timeout(Some(SMTP_CONNECT_TIMEOUT));
            if let Some(c) = creds {
                builder = builder.credentials(c);
            }
            builder.build()
        }
        _ => {
            // Default: "starttls"
            let mut builder =
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
                    .map_err(|e| {
                        report!(ChannelError::DeliveryFailed(format!(
                            "failed to build STARTTLS SMTP transport: {e}"
                        )))
                    })?;
            builder = builder.port(cfg.smtp_port);
            builder = builder.timeout(Some(SMTP_CONNECT_TIMEOUT));
            if let Some(c) = creds {
                builder = builder.credentials(c);
            }
            builder.build()
        }
    };

    Ok(transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> EmailChannel {
        EmailChannel
    }

    // ── validate_config ──────────────────────────────────────────────────

    #[test]
    fn validate_config_rejects_empty_to_addresses() {
        let config = serde_json::json!({"to_addresses": []});
        let err = channel().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("to_addresses"),
            "expected to_addresses mention, got: {msg}"
        );
    }

    #[test]
    fn validate_config_rejects_missing_to_addresses() {
        let config = serde_json::json!({});
        let err = channel().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(!msg.is_empty(), "should produce an error for missing field");
    }

    #[test]
    fn validate_config_rejects_invalid_email_format() {
        let config = serde_json::json!({"to_addresses": ["not-an-email"]});
        let err = channel().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("invalid email address"),
            "expected invalid email error, got: {msg}"
        );
    }

    #[test]
    fn validate_config_rejects_email_without_dot_in_domain() {
        let config = serde_json::json!({"to_addresses": ["user@nodomain"]});
        let err = channel().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("invalid email address"),
            "expected invalid email error, got: {msg}"
        );
    }

    #[test]
    fn validate_config_accepts_valid_config() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        assert!(channel().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_accepts_multiple_valid_addresses() {
        let config = serde_json::json!({
            "to_addresses": ["alice@example.com", "bob@example.org"]
        });
        assert!(channel().validate_config(&config).is_ok());
    }

    // ── mask_config_secrets ──────────────────────────────────────────────

    #[test]
    fn mask_config_secrets_returns_config_unchanged() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let masked = channel().mask_config_secrets(&config);
        assert_eq!(masked, config, "per-channel config has no secrets to mask");
    }

    // ── deliver ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn deliver_returns_error_on_missing_required_fields() {
        // Config missing smtp_host and from_address should fail deserialization or validation.
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let msg = DeliveryMessage {
            title: "Test".to_string(),
            body: "Body".to_string(),
            body_html: None,
            event_payload: serde_json::json!({}),
            actions: vec![],
        };
        let result = channel().deliver(&config, &msg).await;
        assert!(result.is_err(), "missing smtp_host should produce an error");
    }

    #[tokio::test]
    async fn deliver_returns_error_on_unreachable_smtp_host() {
        // Bind on a loopback port then immediately release it so that the
        // connection attempt gets an instant ECONNREFUSED rather than waiting
        // for an OS-level TCP timeout (which can exceed 75 seconds for
        // non-routable addresses such as the TEST-NET-1 range 192.0.2.0/24).
        let free_addr = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let config = serde_json::json!({
            "smtp_host": free_addr.ip().to_string(),
            "smtp_port": free_addr.port(),
            "from_address": "sender@example.com",
            "to_addresses": ["user@example.com"],
            "tls_mode": "none"
        });
        let msg = DeliveryMessage {
            title: "Test".to_string(),
            body: "Body".to_string(),
            body_html: None,
            event_payload: serde_json::json!({}),
            actions: vec![],
        };
        let result = channel().deliver(&config, &msg).await;
        assert!(
            result.is_err(),
            "delivery to a refused connection should fail"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(
                err.current_context(),
                ChannelError::DeliveryFailed(_) | ChannelError::InvalidConfig(_)
            ),
            "expected DeliveryFailed or InvalidConfig, got: {err}"
        );
    }

    // ── helpers ──────────────────────────────────────────────────────────

    #[test]
    fn is_valid_email_accepts_standard_addresses() {
        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("user+tag@sub.domain.org"));
        assert!(is_valid_email("a@b.io"));
    }

    #[test]
    fn is_valid_email_rejects_no_at_sign() {
        assert!(!is_valid_email("notanemail"));
        assert!(!is_valid_email("no-at-sign.com"));
    }

    #[test]
    fn is_valid_email_rejects_empty_local_or_domain() {
        assert!(!is_valid_email("@domain.com"));
        assert!(!is_valid_email("local@"));
    }

    #[test]
    fn is_valid_email_rejects_domain_without_dot() {
        assert!(!is_valid_email("user@nodomain"));
    }

    #[test]
    fn escape_html_escapes_special_chars() {
        assert_eq!(
            escape_html("<b>hello & \"world\"</b>"),
            "&lt;b&gt;hello &amp; &quot;world&quot;&lt;/b&gt;"
        );
    }

    #[test]
    fn escape_html_preserves_plain_text() {
        assert_eq!(escape_html("hello world"), "hello world");
    }

    #[test]
    fn wrap_html_produces_valid_structure() {
        let result = wrap_html("<p>Test</p>");
        assert!(result.starts_with("<!DOCTYPE html>"));
        assert!(result.contains("<body><p>Test</p></body>"));
    }
}
