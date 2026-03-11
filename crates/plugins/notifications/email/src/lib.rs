//! SMTP email notification plugin.
//!
//! Delivers notifications via SMTP using `mail-send`. Per-channel configuration
//! contains only the recipient addresses; SMTP server credentials and sender
//! identity are supplied at delivery time from the merged global SMTP settings.

use std::time::Duration;

use async_trait::async_trait;
use mail_builder::MessageBuilder;
use mail_send::SmtpClientBuilder;
use rootcause::prelude::*;
use serde::Deserialize;

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionPlacement, ExtensionUi,
    FieldDef, FieldType, FormDef, PanelPosition, SelectOption, TableColumn,
};
use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPlugin, NotificationPluginError, Result, escape_html,
};

/// Minimum config required on the per-channel DB row (recipients only).
///
/// Validated at channel create/update time via
/// [`EmailPlugin::validate_config`].
#[derive(Debug, Deserialize)]
struct EmailChannelConfig {
    to_addresses: Vec<String>,
}

/// Full merged config passed to [`EmailPlugin::deliver`] at dispatch time.
///
/// SMTP credentials come from the global per-tenant SMTP settings and are
/// merged with the per-channel `to_addresses` by the dispatcher before the
/// plugin's `deliver` method is called.
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

/// Merge global SMTP settings into a per-channel email config object.
///
/// The per-channel email config contains only `to_addresses`. This function
/// adds the SMTP connection and auth fields from the live settings snapshot
/// so that [`EmailPlugin::deliver`] receives the full merged config.
///
/// # Arguments
///
/// * `smtp` - SMTP settings snapshot containing host, port, credentials, etc.
/// * `config` - Per-channel config JSON (must be an object).
pub fn merge_smtp_into_config(
    smtp: &SmtpSettingsSnapshot,
    mut config: serde_json::Value,
) -> serde_json::Value {
    let obj = config.as_object_mut().expect("config is always an object");
    if let Some(ref host) = smtp.host {
        obj.insert("smtp_host".to_string(), serde_json::json!(host));
    }
    obj.insert(
        "smtp_port".to_string(),
        serde_json::json!(smtp.port.unwrap_or(587)),
    );
    if let Some(ref username) = smtp.username {
        obj.insert("smtp_username".to_string(), serde_json::json!(username));
    }
    if let Some(ref password) = smtp.password {
        obj.insert("smtp_password".to_string(), serde_json::json!(password));
    }
    if let Some(ref from_address) = smtp.from_address {
        obj.insert("from_address".to_string(), serde_json::json!(from_address));
    }
    if let Some(ref from_name) = smtp.from_name {
        obj.insert("from_name".to_string(), serde_json::json!(from_name));
    }
    obj.insert(
        "tls_mode".to_string(),
        serde_json::json!(smtp.tls_mode.clone()),
    );
    config
}

/// A snapshot of SMTP settings used for merging into per-channel config.
///
/// This mirrors the fields provided by the settings store.
#[derive(Clone, Debug)]
pub struct SmtpSettingsSnapshot {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub tls_mode: String,
}

impl SmtpSettingsSnapshot {
    /// Returns `true` if the minimum required SMTP settings (host + from_address)
    /// are present.
    pub fn is_configured(&self) -> bool {
        self.host.is_some() && self.from_address.is_some()
    }
}

/// Email notification plugin via SMTP.
///
/// Per-channel config stores only recipient addresses (`to_addresses`). SMTP
/// server credentials and sender identity are merged into the config JSON by
/// the dispatcher from the global per-tenant SMTP settings before
/// [`deliver`](EmailPlugin::deliver) is called.
pub struct EmailPlugin;

/// Connection timeout applied to every SMTP connection.
///
/// Prevents the client from hanging indefinitely when the server is
/// unreachable or a firewall silently drops SYN packets.
const SMTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Connect to the SMTP server and send a single message.
///
/// `mail-send`'s `connect()` returns `SmtpClient<TlsStream<TcpStream>>` while
/// `connect_plain()` returns `SmtpClient<TcpStream>` — different concrete types.
/// This function encapsulates the TLS-mode dispatch so callers don't need to
/// deal with the type divergence.
async fn send_email(cfg: &EmailConfig, message: MessageBuilder<'_>) -> Result<()> {
    let mut builder =
        SmtpClientBuilder::new(cfg.smtp_host.as_str(), cfg.smtp_port).timeout(SMTP_CONNECT_TIMEOUT);

    if let (Some(user), Some(pass)) = (&cfg.smtp_username, &cfg.smtp_password)
        && !user.is_empty()
    {
        builder = builder.credentials((user.as_str(), pass.as_str()));
    }

    match cfg.tls_mode.as_str() {
        "tls" => {
            builder = builder.implicit_tls(true);
            let mut client = builder.connect().await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP TLS connection failed: {e}"
                )))
            })?;
            client.send(message).await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP send failed: {e}"
                )))
            })?;
        }
        "none" => {
            let mut client = builder.connect_plain().await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP plaintext connection failed: {e}"
                )))
            })?;
            client.send(message).await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP send failed: {e}"
                )))
            })?;
        }
        _ => {
            // Default: STARTTLS
            builder = builder.implicit_tls(false);
            let mut client = builder.connect().await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP STARTTLS connection failed: {e}"
                )))
            })?;
            client.send(message).await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP send failed: {e}"
                )))
            })?;
        }
    }

    Ok(())
}

#[async_trait]
impl NotificationPlugin for EmailPlugin {
    fn channel_type(&self) -> &'static str {
        "email"
    }

    /// Deliver a notification to all configured recipients.
    ///
    /// The `config` argument must be the *merged* config containing both the
    /// global SMTP settings and the per-channel `to_addresses`.
    async fn deliver(&self, config: &serde_json::Value, message: &DeliveryMessage) -> Result<()> {
        let cfg: EmailConfig = serde_json::from_value(config.clone()).map_err(|e| {
            report!(NotificationPluginError::InvalidConfig(format!(
                "failed to deserialize email config: {e}"
            )))
        })?;

        if cfg.to_addresses.is_empty() {
            bail!(NotificationPluginError::InvalidConfig(
                "'to_addresses' must not be empty".to_string()
            ));
        }
        if cfg.smtp_host.is_empty() {
            bail!(NotificationPluginError::InvalidConfig(
                "'smtp_host' must not be empty".to_string()
            ));
        }
        if cfg.from_address.is_empty() {
            bail!(NotificationPluginError::InvalidConfig(
                "'from_address' must not be empty".to_string()
            ));
        }

        // Build From header
        let from_header = if let Some(ref name) = cfg.from_name {
            format!("{name} <{}>", cfg.from_address)
        } else {
            cfg.from_address.clone()
        };

        // Build the HTML body — use existing HTML if provided, otherwise escape plain text.
        let html_body = if let Some(ref html) = message.body_html {
            wrap_html(html)
        } else {
            wrap_html(&escape_html(&message.body))
        };

        // Send one message per recipient.
        for to_addr in &cfg.to_addresses {
            let email = MessageBuilder::new()
                .from(from_header.as_str())
                .to(to_addr.as_str())
                .subject(&message.title)
                .text_body(&message.body)
                .html_body(&html_body);

            send_email(&cfg, email).await?;

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
    fn validate_config(&self, config: &serde_json::Value) -> Result<()> {
        let cfg: EmailChannelConfig = serde_json::from_value(config.clone()).map_err(|e| {
            report!(NotificationPluginError::InvalidConfig(format!(
                "failed to deserialize email channel config: {e}"
            )))
        })?;

        if cfg.to_addresses.is_empty() {
            bail!(NotificationPluginError::InvalidConfig(
                "'to_addresses' must not be empty".to_string()
            ));
        }

        for addr in &cfg.to_addresses {
            if !is_valid_email(addr) {
                bail!(NotificationPluginError::InvalidConfig(format!(
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

    fn extension_manifests(&self) -> Vec<ExtensionManifest> {
        vec![
            // Channel management tab (grouped with other notification channels)
            ExtensionManifest::new(
                "notifications.email",
                "Email Channels",
                502,
                ExtensionPlacement::Panel {
                    target_page: "settings".to_string(),
                    position: PanelPosition::Tab,
                    tab_group: Some("Notification Channels".to_string()),
                },
                ExtensionUi::DataTable {
                    columns: vec![
                        TableColumn::new("name", "Name"),
                        TableColumn::new("to_addresses", "Recipients"),
                        TableColumn::new("enabled", "Enabled"),
                        TableColumn::new("created_at", "Created"),
                    ],
                    data_action: "list".to_string(),
                    row_actions: vec!["edit".to_string(), "test".to_string(), "delete".to_string()],
                    primary_actions: vec!["create".to_string(), "configure_smtp".to_string()],
                    context_selector: None,
                    default_per_page: Some(20),
                },
            )
            .with_permission("view_notifications"),
            // Global SMTP defaults panel (below global settings)
            ExtensionManifest::new(
                "notifications.email.global_smtp",
                "SMTP Defaults",
                600,
                ExtensionPlacement::Panel {
                    target_page: "global-settings".to_string(),
                    position: PanelPosition::Below,
                    tab_group: None,
                },
                ExtensionUi::Form(
                    FormDef::new(vec![
                        FieldDef::new("host", "SMTP Host").with_placeholder("smtp.example.com"),
                        FieldDef::new("port", "Port")
                            .with_type(FieldType::Number)
                            .with_default_value(serde_json::json!("587")),
                        FieldDef::new("tls_mode", "TLS Mode")
                            .with_type(FieldType::Select)
                            .with_options(vec![
                                SelectOption::new("starttls", "STARTTLS (port 587)"),
                                SelectOption::new("tls", "TLS (port 465)"),
                                SelectOption::new("none", "None (port 25)"),
                            ])
                            .with_default_value(serde_json::json!("starttls")),
                        FieldDef::new("from_address", "From Address")
                            .with_placeholder("noreply@example.com"),
                        FieldDef::new("from_name", "From Name")
                            .with_placeholder("Uptrakit Notifications"),
                        FieldDef::new("username", "Username").with_placeholder("SMTP username"),
                        FieldDef::new("password", "Password")
                            .with_type(FieldType::Password)
                            .with_help_text("Leave empty to keep current password"),
                    ])
                    .with_pre_load_action("get_global_smtp"),
                ),
            )
            .with_permission("manage_global_settings"),
        ]
    }

    fn extension_actions(&self) -> Vec<ActionDef> {
        vec![
            ActionDef::new("list", "List"),
            ActionDef::new("create", "Add Email Channel")
                .with_permission("manage_notifications")
                .with_ui(ActionUi::Form(FormDef::new(vec![
                    FieldDef::new("name", "Name").required(),
                    FieldDef::new("to_addresses", "Recipients")
                        .required()
                        .with_type(FieldType::Textarea)
                        .with_placeholder("user@example.com\nadmin@example.com")
                        .with_help_text("One email address per line"),
                    FieldDef::new("enabled", "Enabled")
                        .with_type(FieldType::Toggle)
                        .with_default_value(serde_json::json!("true")),
                ])))
                .with_api_submit(
                    ApiSubmitDef::new(
                        "POST",
                        "/api/v1/notifications/channels",
                        serde_json::json!({
                            "name": "{{name}}",
                            "channel_type": "email",
                            "config": {
                                "to_addresses": "{{to_addresses}}"
                            },
                            "enabled": "{{enabled:bool}}"
                        }),
                    )
                    .with_response_id_field("id"),
                ),
            ActionDef::new("edit", "Edit")
                .with_permission("manage_notifications")
                .with_ui(ActionUi::Form(FormDef::new(vec![
                    FieldDef::new("id", "ID").with_type(FieldType::Hidden),
                    FieldDef::new("name", "Name").required(),
                    FieldDef::new("to_addresses", "Recipients")
                        .required()
                        .with_type(FieldType::Textarea)
                        .with_placeholder("user@example.com\nadmin@example.com")
                        .with_help_text("One email address per line"),
                    FieldDef::new("enabled", "Enabled")
                        .with_type(FieldType::Toggle)
                        .with_default_value(serde_json::json!("true")),
                ])))
                .with_api_submit(ApiSubmitDef::new(
                    "PUT",
                    "/api/v1/notifications/channels/{{id}}",
                    serde_json::json!({
                        "name": "{{name}}",
                        "config": {
                            "to_addresses": "{{to_addresses}}"
                        },
                        "enabled": "{{enabled:bool}}"
                    }),
                )),
            ActionDef::new("test", "Test")
                .with_permission("manage_notifications")
                .with_api_submit(ApiSubmitDef::new(
                    "POST",
                    "/api/v1/notifications/channels/{{id}}/test",
                    serde_json::json!({}),
                )),
            ActionDef::new("delete", "Delete")
                .with_permission("manage_notifications")
                .destructive()
                .with_confirm_entity_field("name")
                .with_api_submit(ApiSubmitDef::new(
                    "DELETE",
                    "/api/v1/notifications/channels/{{id}}",
                    serde_json::json!({}),
                )),
            ActionDef::new("configure_smtp", "Override SMTP")
                .with_permission("manage_notifications")
                .with_ui(ActionUi::Form(
                    FormDef::new(vec![
                        FieldDef::new("host", "SMTP Host").with_placeholder("smtp.example.com"),
                        FieldDef::new("port", "Port")
                            .with_type(FieldType::Number)
                            .with_default_value(serde_json::json!("587")),
                        FieldDef::new("tls_mode", "TLS Mode")
                            .with_type(FieldType::Select)
                            .with_options(vec![
                                SelectOption::new("starttls", "STARTTLS (port 587)"),
                                SelectOption::new("tls", "TLS (port 465)"),
                                SelectOption::new("none", "None (port 25)"),
                            ])
                            .with_default_value(serde_json::json!("starttls")),
                        FieldDef::new("from_address", "From Address")
                            .with_placeholder("noreply@example.com"),
                        FieldDef::new("from_name", "From Name")
                            .with_placeholder("Uptrakit Notifications"),
                        FieldDef::new("username", "Username").with_placeholder("SMTP username"),
                        FieldDef::new("password", "Password")
                            .with_type(FieldType::Password)
                            .with_help_text("Leave empty to keep current password"),
                    ])
                    .with_pre_load_action("get_smtp"),
                )),
            ActionDef::new("get_smtp", "Get SMTP Settings"),
            ActionDef::new("save_smtp", "Save SMTP Settings")
                .with_permission("manage_notifications"),
            ActionDef::new("get_global_smtp", "Get Global SMTP Defaults"),
            ActionDef::new("save_global_smtp", "Save Global SMTP Defaults")
                .with_permission("manage_global_settings"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> EmailPlugin {
        EmailPlugin
    }

    // ── validate_config ──────────────────────────────────────────────────

    #[test]
    fn validate_config_rejects_empty_to_addresses() {
        let config = serde_json::json!({"to_addresses": []});
        let err = plugin().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("to_addresses"),
            "expected to_addresses mention, got: {msg}"
        );
    }

    #[test]
    fn validate_config_rejects_missing_to_addresses() {
        let config = serde_json::json!({});
        let err = plugin().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(!msg.is_empty(), "should produce an error for missing field");
    }

    #[test]
    fn validate_config_rejects_invalid_email_format() {
        let config = serde_json::json!({"to_addresses": ["not-an-email"]});
        let err = plugin().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("invalid email address"),
            "expected invalid email error, got: {msg}"
        );
    }

    #[test]
    fn validate_config_rejects_email_without_dot_in_domain() {
        let config = serde_json::json!({"to_addresses": ["user@nodomain"]});
        let err = plugin().validate_config(&config).unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("invalid email address"),
            "expected invalid email error, got: {msg}"
        );
    }

    #[test]
    fn validate_config_accepts_valid_config() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        assert!(plugin().validate_config(&config).is_ok());
    }

    #[test]
    fn validate_config_accepts_multiple_valid_addresses() {
        let config = serde_json::json!({
            "to_addresses": ["alice@example.com", "bob@example.org"]
        });
        assert!(plugin().validate_config(&config).is_ok());
    }

    // ── mask_config_secrets ──────────────────────────────────────────────

    #[test]
    fn mask_config_secrets_returns_config_unchanged() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let masked = plugin().mask_config_secrets(&config);
        assert_eq!(masked, config, "per-channel config has no secrets to mask");
    }

    // ── deliver ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn deliver_returns_error_on_missing_required_fields() {
        // Config missing smtp_host and from_address should fail deserialization or validation.
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let msg = DeliveryMessage::new("Test", "Body", None, serde_json::json!({}), vec![]);
        let result = plugin().deliver(&config, &msg).await;
        assert!(result.is_err(), "missing smtp_host should produce an error");
    }

    #[tokio::test]
    async fn deliver_returns_error_on_unreachable_smtp_host() {
        // Bind on a loopback port then immediately release it so that the
        // connection attempt gets an instant ECONNREFUSED rather than waiting
        // for an OS-level TCP timeout.
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
        let msg = DeliveryMessage::new("Test", "Body", None, serde_json::json!({}), vec![]);
        let result = plugin().deliver(&config, &msg).await;
        assert!(
            result.is_err(),
            "delivery to a refused connection should fail"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(
                err.current_context(),
                NotificationPluginError::DeliveryFailed(_)
                    | NotificationPluginError::InvalidConfig(_)
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

    // ── merge_smtp_into_config ────────────────────────────────────────────

    fn make_smtp(
        host: Option<&str>,
        port: Option<u16>,
        from: Option<&str>,
    ) -> SmtpSettingsSnapshot {
        SmtpSettingsSnapshot {
            host: host.map(|s| s.to_string()),
            port,
            username: None,
            password: None,
            from_address: from.map(|s| s.to_string()),
            from_name: None,
            tls_mode: "starttls".to_string(),
        }
    }

    #[test]
    fn merge_smtp_sets_host_and_default_port() {
        let smtp = make_smtp(Some("mail.example.com"), None, Some("noreply@example.com"));
        let config = serde_json::json!({ "to_addresses": ["user@test.local"] });
        let merged = merge_smtp_into_config(&smtp, config);

        assert_eq!(merged["smtp_host"], "mail.example.com");
        assert_eq!(
            merged["smtp_port"], 587,
            "default port must be 587 when port is None"
        );
        assert_eq!(merged["from_address"], "noreply@example.com");
        assert!(merged["to_addresses"].is_array());
    }

    #[test]
    fn merge_smtp_uses_explicit_port() {
        let smtp = make_smtp(
            Some("smtp.corp.internal"),
            Some(465),
            Some("alerts@corp.internal"),
        );
        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&smtp, config);

        assert_eq!(merged["smtp_port"], 465);
    }

    #[test]
    fn merge_smtp_propagates_all_optional_fields() {
        let mut smtp = make_smtp(Some("smtp.example.com"), None, Some("from@example.com"));
        smtp.username = Some("smtpuser".to_string());
        smtp.password = Some("secret".to_string());
        smtp.from_name = Some("Uptrakit Alerts".to_string());
        smtp.tls_mode = "tls".to_string();

        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&smtp, config);

        assert_eq!(merged["smtp_username"], "smtpuser");
        assert_eq!(merged["smtp_password"], "secret");
        assert_eq!(merged["from_name"], "Uptrakit Alerts");
        assert_eq!(merged["tls_mode"], "tls");
    }

    #[test]
    fn merge_smtp_omits_host_when_none() {
        let smtp = make_smtp(None, None, None);
        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&smtp, config);

        assert!(
            merged.get("smtp_host").is_none(),
            "smtp_host must not be set when host is None"
        );
    }
}
