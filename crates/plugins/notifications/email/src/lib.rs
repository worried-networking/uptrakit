//! SMTP email notification plugin.
//!
//! Delivers notifications via SMTP using `mail-send`. Per-channel configuration
//! contains only the recipient addresses; SMTP server credentials and sender
//! identity are supplied at delivery time from the merged global SMTP settings.

pub mod extensions;

use std::time::Duration;

use async_trait::async_trait;
use mail_builder::MessageBuilder;
use mail_send::SmtpClientBuilder;
use rootcause::prelude::*;
use serde::Deserialize;
use uptrakit_shared_types::SecretString;

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ExtensionManifest, ExtensionPlacement, ExtensionUi,
    FieldDef, FieldType, FormDef, PanelPosition, SelectOption, TableColumn,
};
use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPluginError, Result, escape_html,
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
    /// Optional explicit EHLO hostname override.
    ///
    /// When absent, the domain part of `from_address` is used.
    #[serde(default)]
    helo_host: Option<String>,
}

fn default_tls_mode() -> String {
    "starttls".to_string()
}

/// Minimal email format validation: must contain exactly one `@` with
/// non-empty local and domain parts and at least one `.` in the domain.
fn is_valid_email(addr: &str) -> bool {
    let Some((local, domain)) = addr.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

/// Wrap an HTML snippet in a minimal HTML5 document shell.
fn wrap_html(html_body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>{html_body}</body></html>"
    )
}

/// Merge SMTP settings into a per-channel email config object.
///
/// The per-channel email config contains only `to_addresses`. This function
/// adds the SMTP connection and auth fields so that [`EmailPlugin::deliver`]
/// receives the full merged config.
///
/// When both `global` and `tenant` snapshots are provided, per-tenant
/// non-empty fields override the global defaults (field-by-field inheritance).
///
/// # Arguments
///
/// * `global` - Global SMTP defaults (shared across tenants). May be empty.
/// * `tenant` - Per-tenant SMTP settings. Non-empty fields override global.
/// * `config` - Per-channel config JSON (must be an object).
pub fn merge_smtp_into_config(
    global: &SmtpSettingsSnapshot,
    tenant: &SmtpSettingsSnapshot,
    mut config: serde_json::Value,
) -> serde_json::Value {
    let obj = config.as_object_mut().expect("config is always an object");

    // Merge each field: tenant overrides global when present
    let host = tenant.host.as_ref().or(global.host.as_ref());
    if let Some(host) = host {
        obj.insert("smtp_host".to_string(), serde_json::json!(host));
    }

    let port = tenant.port.or(global.port);
    obj.insert(
        "smtp_port".to_string(),
        serde_json::json!(port.unwrap_or(587)),
    );

    let username = tenant.username.as_ref().or(global.username.as_ref());
    if let Some(username) = username {
        obj.insert("smtp_username".to_string(), serde_json::json!(username));
    }

    let password = tenant.password.as_ref().or(global.password.as_ref());
    if let Some(password) = password {
        obj.insert(
            "smtp_password".to_string(),
            serde_json::json!(password.expose_secret()),
        );
    }

    let from_address = tenant
        .from_address
        .as_ref()
        .or(global.from_address.as_ref());
    if let Some(from_address) = from_address {
        obj.insert("from_address".to_string(), serde_json::json!(from_address));
    }

    let from_name = tenant.from_name.as_ref().or(global.from_name.as_ref());
    if let Some(from_name) = from_name {
        obj.insert("from_name".to_string(), serde_json::json!(from_name));
    }

    let helo_host = tenant.helo_host.as_ref().or(global.helo_host.as_ref());
    if let Some(h) = helo_host {
        obj.insert("helo_host".to_string(), serde_json::json!(h));
    }

    // TLS mode: use tenant if not the default, otherwise global
    let tls_mode = if tenant.tls_mode != "starttls" {
        &tenant.tls_mode
    } else if global.tls_mode != "starttls" {
        &global.tls_mode
    } else {
        &tenant.tls_mode
    };
    obj.insert("tls_mode".to_string(), serde_json::json!(tls_mode));

    config
}

/// A snapshot of SMTP settings used for merging into per-channel config.
///
/// This mirrors the fields provided by the settings store.
///
/// The `password` field uses [`SecretString`] to prevent the SMTP credential from
/// appearing in tracing logs or panic messages. Call `.expose_secret()` only at the
/// point where the plaintext password must be passed to the mail-send client.
#[derive(Clone, Debug)]
pub struct SmtpSettingsSnapshot {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    /// SMTP authentication password.
    ///
    /// Stored as [`SecretString`] so that `Debug` output never reveals the credential.
    pub password: Option<SecretString>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub tls_mode: String,
    /// Optional EHLO hostname override for the SMTP EHLO command.
    ///
    /// When `None`, the domain part of `from_address` is derived at send time.
    pub helo_host: Option<String>,
}

impl SmtpSettingsSnapshot {
    /// Returns `true` if the minimum required SMTP settings (host + from_address)
    /// are present.
    pub fn is_configured(&self) -> bool {
        self.host.is_some() && self.from_address.is_some()
    }
}

/// Build an [`SmtpSettingsSnapshot`] from a flat JSON settings map using the
/// given key prefix.
///
/// For example, with prefix `"smtp."`, looks up `"smtp.host"`, `"smtp.port"`,
/// etc. With prefix `"global_smtp."`, looks up `"global_smtp.host"`, etc.
pub fn smtp_from_settings_map(
    settings_map: &serde_json::Value,
    prefix: &str,
) -> SmtpSettingsSnapshot {
    let get_str = |suffix: &str| -> Option<String> {
        let key = format!("{prefix}{suffix}");
        settings_map
            .get(&key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    };

    let port = {
        let key = format!("{prefix}port");
        settings_map
            .get(&key)
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .and_then(|n| u16::try_from(n).ok())
    };

    let tls_mode = {
        let key = format!("{prefix}tls_mode");
        settings_map
            .get(&key)
            .and_then(|v| v.as_str())
            .filter(|s| matches!(*s, "starttls" | "tls" | "none"))
            .unwrap_or("starttls")
            .to_string()
    };

    SmtpSettingsSnapshot {
        host: get_str("host"),
        port,
        username: get_str("username"),
        password: get_str("password").map(SecretString::new),
        from_address: get_str("from_address"),
        from_name: get_str("from_name"),
        tls_mode,
        helo_host: get_str("helo_host"),
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

    let has_credentials = if let (Some(user), Some(_pass)) =
        (&cfg.smtp_username, &cfg.smtp_password)
        && !user.is_empty()
    {
        builder = builder.credentials((user.as_str(), _pass.as_str()));
        true
    } else {
        false
    };

    // Derive EHLO hostname: explicit config override → domain from from_address.
    // Never use gethostname() — short hostnames (e.g. Docker container names) are not
    // valid FQDNs and cause Gmail to reject with "555 5.5.2 Syntax error" (RFC 5321).
    let ehlo_host = cfg
        .helo_host
        .as_deref()
        .filter(|h| !h.is_empty())
        .or_else(|| cfg.from_address.split_once('@').map(|(_, domain)| domain))
        .unwrap_or("localhost")
        .to_string();
    builder = builder.helo_host(ehlo_host.clone());

    tracing::debug!(
        smtp_host = %cfg.smtp_host,
        smtp_port = cfg.smtp_port,
        tls_mode = %cfg.tls_mode,
        ehlo_host = %ehlo_host,
        has_credentials,
        "connecting to SMTP server"
    );

    match cfg.tls_mode.as_str() {
        "tls" => {
            builder = builder.implicit_tls(true);
            let mut client = builder.connect().await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP TLS connection failed: {e}"
                )))
            })?;
            tracing::trace!("SMTP TLS connection established, sending message");
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
            tracing::trace!("SMTP plaintext connection established, sending message");
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
            tracing::trace!("SMTP STARTTLS connection established, sending message");
            client.send(message).await.map_err(|e| {
                report!(NotificationPluginError::DeliveryFailed(format!(
                    "SMTP send failed: {e}"
                )))
            })?;
        }
    }

    Ok(())
}

// ── PluginBase + NotificationTransportPlugin ────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PluginBase for EmailPlugin {
    fn plugin_type_id(&self) -> &str {
        "email"
    }

    fn capabilities(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
        vec![uptrakit_plugin_infrastructure_core::PluginCapability::NotificationDelivery]
    }

    fn validate_config(&self, config: &serde_json::Value) -> std::result::Result<(), String> {
        let cfg: EmailChannelConfig =
            serde_json::from_value(config.clone()).map_err(|e| e.to_string())?;

        if cfg.to_addresses.is_empty() {
            return Err("'to_addresses' must not be empty".to_string());
        }

        for addr in &cfg.to_addresses {
            if !is_valid_email(addr) {
                return Err(format!("invalid email address: '{addr}'"));
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
                        FieldDef::new("helo_host", "EHLO Hostname")
                            .with_placeholder("mail.example.com")
                            .with_help_text(
                                "Hostname sent in the SMTP EHLO command. Defaults to the domain \
                                 of the From address. Set explicitly when using a relay server.",
                            ),
                        FieldDef::new("username", "Username").with_placeholder("SMTP username"),
                        FieldDef::new("password", "Password")
                            .with_type(FieldType::Password)
                            .with_help_text("Leave empty to keep current password"),
                    ])
                    .with_pre_load_action("get_global_smtp")
                    .with_footer_actions(vec!["test_global_smtp_email".to_string()]),
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
            ActionDef::new("test_global_smtp_email", "Send Test Email")
                .with_permission("manage_global_settings"),
            ActionDef::new("get_global_smtp", "Get Global SMTP Defaults"),
            ActionDef::new("save_global_smtp", "Save Global SMTP Defaults")
                .with_permission("manage_global_settings"),
        ]
    }

    fn as_notification_transport(
        &self,
    ) -> Option<&dyn uptrakit_plugin_infrastructure_core::NotificationTransportPlugin> {
        Some(self)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::NotificationTransportPlugin for EmailPlugin {
    fn channel_type(&self) -> &'static str {
        "email"
    }

    /// Deliver a notification to all configured recipients.
    ///
    /// The `config` argument contains the per-channel config (primarily
    /// `to_addresses`). SMTP credentials are extracted from the `settings`
    /// bag and merged into the config before sending.
    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()> {
        // Merge SMTP settings from the settings bag into the per-channel config.
        let merged_config = if config.get("smtp_host").is_some() {
            // Config is already merged (e.g. from a caller that pre-merged).
            config.clone()
        } else {
            let global =
                smtp_from_settings_map(&settings["global"], crate::extensions::GLOBAL_SMTP_PREFIX);
            let tenant =
                smtp_from_settings_map(&settings["tenant"], crate::extensions::SMTP_PREFIX);
            merge_smtp_into_config(&global, &tenant, config.clone())
        };

        let cfg: EmailConfig = serde_json::from_value(merged_config).map_err(|e| {
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

        // Build the HTML body — use existing HTML if provided, otherwise escape plain text.
        let html_body = if let Some(ref html) = message.body_html {
            wrap_html(html)
        } else {
            wrap_html(&escape_html(&message.body))
        };

        tracing::debug!(
            from_address = %cfg.from_address,
            from_name = cfg.from_name.as_deref().unwrap_or("<none>"),
            recipient_count = cfg.to_addresses.len(),
            subject = %message.title,
            "building email message"
        );

        // Send one message per recipient.
        // Use mail-builder's structured tuple API for From header so that
        // display names with special characters (spaces, quotes, commas) are
        // properly RFC 5322 encoded. Passing a pre-formatted string like
        // `"Name <addr>"` requires mail-builder to re-parse it, which can
        // produce malformed MAIL FROM commands rejected by strict servers
        // (e.g. Gmail 555 5.5.2 Syntax error).
        for to_addr in &cfg.to_addresses {
            let email = MessageBuilder::new();
            let email = if let Some(ref name) = cfg.from_name {
                email.from((name.as_str(), cfg.from_address.as_str()))
            } else {
                email.from(cfg.from_address.as_str())
            };
            let email = email
                .to(to_addr.as_str())
                .subject(&message.title)
                .text_body(&message.body)
                .html_body(&html_body);

            send_email(&cfg, email).await?;

            tracing::debug!(to = %to_addr, "email notification delivered");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{NotificationTransportPlugin, PluginBase};

    fn plugin() -> EmailPlugin {
        EmailPlugin
    }

    // ── validate_config ──────────────────────────────────────────────────

    #[test]
    fn validate_config_rejects_empty_to_addresses() {
        let config = serde_json::json!({"to_addresses": []});
        let err = PluginBase::validate_config(&plugin(), &config).unwrap_err();
        assert!(
            err.contains("to_addresses"),
            "expected to_addresses mention, got: {err}"
        );
    }

    #[test]
    fn validate_config_rejects_missing_to_addresses() {
        let config = serde_json::json!({});
        let err = PluginBase::validate_config(&plugin(), &config).unwrap_err();
        assert!(!err.is_empty(), "should produce an error for missing field");
    }

    #[test]
    fn validate_config_rejects_invalid_email_format() {
        let config = serde_json::json!({"to_addresses": ["not-an-email"]});
        let err = PluginBase::validate_config(&plugin(), &config).unwrap_err();
        assert!(
            err.contains("invalid email address"),
            "expected invalid email error, got: {err}"
        );
    }

    #[test]
    fn validate_config_rejects_email_without_dot_in_domain() {
        let config = serde_json::json!({"to_addresses": ["user@nodomain"]});
        let err = PluginBase::validate_config(&plugin(), &config).unwrap_err();
        assert!(
            err.contains("invalid email address"),
            "expected invalid email error, got: {err}"
        );
    }

    #[test]
    fn validate_config_accepts_valid_config() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        assert!(PluginBase::validate_config(&plugin(), &config).is_ok());
    }

    #[test]
    fn validate_config_accepts_multiple_valid_addresses() {
        let config = serde_json::json!({
            "to_addresses": ["alice@example.com", "bob@example.org"]
        });
        assert!(PluginBase::validate_config(&plugin(), &config).is_ok());
    }

    // ── mask_config_secrets ──────────────────────────────────────────────

    #[test]
    fn mask_config_secrets_returns_config_unchanged() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let masked = PluginBase::mask_config_secrets(&plugin(), &config);
        assert_eq!(masked, config, "per-channel config has no secrets to mask");
    }

    // ── deliver ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn deliver_returns_error_on_missing_required_fields() {
        // Config missing smtp_host and from_address should fail deserialization or validation.
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let msg = DeliveryMessage::new("Test", "Body", None, serde_json::json!({}), vec![]);
        let empty_settings = serde_json::json!({});
        let result =
            NotificationTransportPlugin::deliver(&plugin(), &config, &empty_settings, &msg).await;
        assert!(result.is_err(), "missing smtp_host should produce an error");
    }

    #[tokio::test]
    async fn deliver_returns_error_on_unreachable_smtp_host() {
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
        let empty_settings = serde_json::json!({});
        let result =
            NotificationTransportPlugin::deliver(&plugin(), &config, &empty_settings, &msg).await;
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

    fn empty_smtp() -> SmtpSettingsSnapshot {
        SmtpSettingsSnapshot {
            host: None,
            port: None,
            username: None,
            password: None,
            from_address: None,
            from_name: None,
            tls_mode: "starttls".to_string(),
            helo_host: None,
        }
    }

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
            helo_host: None,
        }
    }

    #[test]
    fn merge_smtp_sets_host_and_default_port() {
        let smtp = make_smtp(Some("mail.example.com"), None, Some("noreply@example.com"));
        let config = serde_json::json!({ "to_addresses": ["user@test.local"] });
        let merged = merge_smtp_into_config(&empty_smtp(), &smtp, config);

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
        let merged = merge_smtp_into_config(&empty_smtp(), &smtp, config);

        assert_eq!(merged["smtp_port"], 465);
    }

    #[test]
    fn merge_smtp_propagates_all_optional_fields() {
        let mut smtp = make_smtp(Some("smtp.example.com"), None, Some("from@example.com"));
        smtp.username = Some("smtpuser".to_string());
        smtp.password = Some(SecretString::new("secret"));
        smtp.from_name = Some("Uptrakit Alerts".to_string());
        smtp.tls_mode = "tls".to_string();

        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&empty_smtp(), &smtp, config);

        assert_eq!(merged["smtp_username"], "smtpuser");
        assert_eq!(merged["smtp_password"], "secret");
        assert_eq!(merged["from_name"], "Uptrakit Alerts");
        assert_eq!(merged["tls_mode"], "tls");
    }

    #[test]
    fn merge_smtp_omits_host_when_none() {
        let smtp = make_smtp(None, None, None);
        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&empty_smtp(), &smtp, config);

        assert!(
            merged.get("smtp_host").is_none(),
            "smtp_host must not be set when host is None"
        );
    }

    // ── EHLO hostname derivation ──────────────────────────────────────────────

    /// Helper: build an EmailConfig JSON and derive the EHLO host the same way
    /// `send_email` does (inline logic test — no SMTP connection required).
    fn derive_ehlo_host(from_address: &str, helo_host: Option<&str>) -> String {
        // Mirror the logic in `send_email()`.
        let helo_host_owned: Option<String> = helo_host.map(String::from);
        helo_host_owned
            .as_deref()
            .filter(|h| !h.is_empty())
            .or_else(|| from_address.split_once('@').map(|(_, domain)| domain))
            .unwrap_or("localhost")
            .to_string()
    }

    #[test]
    fn ehlo_host_derives_from_from_address_domain() {
        let host = derive_ehlo_host("sender@smtp.example.com", None);
        assert_eq!(host, "smtp.example.com");
    }

    #[test]
    fn ehlo_host_explicit_override_takes_precedence() {
        let host = derive_ehlo_host("sender@smtp.example.com", Some("relay.corp.internal"));
        assert_eq!(host, "relay.corp.internal");
    }

    #[test]
    fn ehlo_host_empty_override_falls_back_to_domain() {
        // An empty string override must be treated as absent.
        let host = derive_ehlo_host("sender@smtp.example.com", Some(""));
        assert_eq!(host, "smtp.example.com");
    }

    #[test]
    fn ehlo_host_subdomain_preserved() {
        let host = derive_ehlo_host("noreply@mail.corp.example.org", None);
        assert_eq!(host, "mail.corp.example.org");
    }

    #[test]
    fn merge_smtp_propagates_helo_host_tenant_override() {
        let mut global = empty_smtp();
        global.helo_host = Some("global.example.com".to_string());

        let mut tenant = empty_smtp();
        tenant.helo_host = Some("tenant.example.com".to_string());

        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&global, &tenant, config);

        assert_eq!(
            merged["helo_host"], "tenant.example.com",
            "tenant helo_host must override global"
        );
    }

    #[test]
    fn merge_smtp_uses_global_helo_host_when_tenant_absent() {
        let mut global = empty_smtp();
        global.helo_host = Some("global.example.com".to_string());

        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&global, &empty_smtp(), config);

        assert_eq!(merged["helo_host"], "global.example.com");
    }

    #[test]
    fn merge_smtp_omits_helo_host_when_both_absent() {
        let config = serde_json::json!({});
        let merged = merge_smtp_into_config(&empty_smtp(), &empty_smtp(), config);

        assert!(
            merged.get("helo_host").is_none(),
            "helo_host must not appear in merged config when neither global nor tenant sets it"
        );
    }
}
