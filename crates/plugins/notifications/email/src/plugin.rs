//! Email notification plugin implementation and `declare_plugin!` invocation.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mail_builder::MessageBuilder;
use mail_send::SmtpClientBuilder;
use rootcause::prelude::*;
use serde::Deserialize;
use uptrakit_shared_types::SecretString;

use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPluginError, Result, escape_html,
};
use uptrakit_plugin_infrastructure_core::{
    ApiSubmitDescriptor, ConfigModel, FormFieldDescriptor, FormFieldType,
    FormSelectOptionDescriptor, PluginFamily, SurfaceActionDescriptor, SurfaceActionUi,
    SurfaceFormDescriptor, declare_plugin, surfaces,
};

use crate::config::EmailChannelConfig;

// ── Internal config types ────────────────────────────────────────────────────

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

// ── SMTP settings snapshot ───────────────────────────────────────────────────

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

// ── HTML helpers ─────────────────────────────────────────────────────────────

/// Wrap an HTML snippet in a minimal HTML5 document shell.
fn wrap_html(html_body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>{html_body}</body></html>"
    )
}

// ── Plugin struct ────────────────────────────────────────────────────────────

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
/// `connect_plain()` returns `SmtpClient<TcpStream>` -- different concrete types.
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

    // Derive EHLO hostname: explicit config override -> domain from from_address.
    // Never use gethostname() -- short hostnames (e.g. Docker container names) are not
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

// ── NotificationTransport ────────────────────────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::NotificationTransport for EmailPlugin {
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
                smtp_from_settings_map(&settings["global"], crate::surfaces::GLOBAL_SMTP_PREFIX);
            let tenant = smtp_from_settings_map(&settings["tenant"], crate::surfaces::SMTP_PREFIX);
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

        // Build the HTML body -- use existing HTML if provided, otherwise escape plain text.
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

/// Return surface action definitions for the email plugin.
fn email_surface_actions() -> Vec<SurfaceActionDescriptor> {
    vec![
        SurfaceActionDescriptor::new("list", "List"),
        SurfaceActionDescriptor::new("create", "Add Email Channel")
            .with_permission("manage_notifications")
            .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
                FormFieldDescriptor::new("name", "Name").required(),
                FormFieldDescriptor::new("to_addresses", "Recipients")
                    .required()
                    .with_type(FormFieldType::Textarea)
                    .with_placeholder("user@example.com\nadmin@example.com")
                    .with_help_text("One email address per line"),
                FormFieldDescriptor::new("enabled", "Enabled")
                    .with_type(FormFieldType::Toggle)
                    .with_default_value(serde_json::json!("true")),
            ])))
            .with_api_submit(
                ApiSubmitDescriptor::new(
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
        SurfaceActionDescriptor::new("edit", "Edit")
            .with_permission("manage_notifications")
            .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
                FormFieldDescriptor::new("id", "ID").with_type(FormFieldType::Hidden),
                FormFieldDescriptor::new("name", "Name").required(),
                FormFieldDescriptor::new("to_addresses", "Recipients")
                    .required()
                    .with_type(FormFieldType::Textarea)
                    .with_placeholder("user@example.com\nadmin@example.com")
                    .with_help_text("One email address per line"),
                FormFieldDescriptor::new("enabled", "Enabled")
                    .with_type(FormFieldType::Toggle)
                    .with_default_value(serde_json::json!("true")),
            ])))
            .with_api_submit(ApiSubmitDescriptor::new(
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
        SurfaceActionDescriptor::new("test", "Test")
            .with_permission("manage_notifications")
            .with_api_submit(ApiSubmitDescriptor::new(
                "POST",
                "/api/v1/notifications/channels/{{id}}/test",
                serde_json::json!({}),
            )),
        SurfaceActionDescriptor::new("delete", "Delete")
            .with_permission("manage_notifications")
            .destructive()
            .with_confirm_entity_field("name")
            .with_api_submit(ApiSubmitDescriptor::new(
                "DELETE",
                "/api/v1/notifications/channels/{{id}}",
                serde_json::json!({}),
            )),
        SurfaceActionDescriptor::new("configure_smtp", "Override SMTP")
            .with_permission("manage_notifications")
            .with_ui(SurfaceActionUi::Form(
                SurfaceFormDescriptor::new(vec![
                    FormFieldDescriptor::new("host", "SMTP Host")
                        .with_placeholder("smtp.example.com"),
                    FormFieldDescriptor::new("port", "Port")
                        .with_type(FormFieldType::Number)
                        .with_default_value(serde_json::json!("587")),
                    FormFieldDescriptor::new("tls_mode", "TLS Mode")
                        .with_type(FormFieldType::Select)
                        .with_options(vec![
                            FormSelectOptionDescriptor::new("starttls", "STARTTLS (port 587)"),
                            FormSelectOptionDescriptor::new("tls", "TLS (port 465)"),
                            FormSelectOptionDescriptor::new("none", "None (port 25)"),
                        ])
                        .with_default_value(serde_json::json!("starttls")),
                    FormFieldDescriptor::new("from_address", "From Address")
                        .with_placeholder("noreply@example.com"),
                    FormFieldDescriptor::new("from_name", "From Name")
                        .with_placeholder("Uptrakit Notifications"),
                    FormFieldDescriptor::new("username", "Username")
                        .with_placeholder("SMTP username"),
                    FormFieldDescriptor::new("password", "Password")
                        .with_type(FormFieldType::Password)
                        .with_help_text("Leave empty to keep current password"),
                ])
                .with_pre_load_action("get_smtp"),
            )),
        SurfaceActionDescriptor::new("get_smtp", "Get SMTP Settings"),
        SurfaceActionDescriptor::new("test_global_smtp_email", "Send Test Email")
            .with_permission("manage_global_settings"),
        SurfaceActionDescriptor::new("get_global_smtp", "Get Global SMTP Defaults"),
        SurfaceActionDescriptor::new("save_global_smtp", "Save Global SMTP Defaults")
            .with_permission("manage_global_settings"),
    ]
}

/// Surface action handler wrapper for the `declare_plugin!` macro.
///
/// Matches the `SurfaceActionHandler` type signature which receives
/// `SurfaceActionContext` (with `db: &dyn Any`). Downcasts
/// the database connection and delegates to `surfaces::handle_surface_action`.
fn email_handle_surface_action<'a>(
    ctx: &'a uptrakit_plugin_infrastructure_core::SurfaceActionContext<'a>,
    surface_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>> {
    Box::pin(async move {
        let db = ctx
            .db
            .downcast_ref::<sea_orm::DatabaseConnection>()
            .ok_or_else(|| "internal error: expected DatabaseConnection".to_string())?;

        // Build the shared-surface context that the existing handler expects.
        let inner_ctx = uptrakit_plugin_infrastructure_core::SurfaceActionContext {
            db,
            tenant_id: ctx.tenant_id,
            caller_user_id: ctx.caller_user_id,
        };

        crate::surfaces::handle_surface_action(&inner_ctx, surface_id, action_id, params).await
    })
}

/// Create the email transport singleton from catalog config.
fn create_email_transport(
    _config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<
    Arc<dyn uptrakit_plugin_infrastructure_core::NotificationTransport>,
> {
    Ok(Arc::new(EmailPlugin))
}

fn collect_registration_capabilities(
    surfaces: &[surfaces::RegisteredSurface],
) -> surfaces::CapabilitySet {
    let mut caps = BTreeSet::new();
    for surface in surfaces {
        caps.extend(surface.descriptor.required_capabilities.0.iter().cloned());
    }
    surfaces::CapabilitySet(caps)
}

fn email_surface_registrations() -> Vec<surfaces::SurfaceRegistration> {
    let channel_surface = {
        let data_source_id =
            surfaces::DataSourceId::new("data.primary").expect("literal data source id is valid");
        surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("notifications.email")
                    .expect("literal surface id is valid"),
                label: "Email Channels".to_string(),
                priority: 502,
                slot: surfaces::SLOT_SETTINGS_TABS.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: Some("view_notifications".to_string()),
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::SectionNode,
                    surfaces::Capability::ActionBarNode,
                    surfaces::Capability::TableNode,
                    surfaces::Capability::DataLoad,
                    surfaces::Capability::FormSubmit,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::ConfirmableAction,
                    surfaces::Capability::ProviderQueryDataSource,
                    surfaces::Capability::UniversalTargeting,
                    surfaces::Capability::SensitiveFields,
                ]),
                root_node: surfaces::SurfaceNode::Section {
                    title: None,
                    children: vec![
                        surfaces::SurfaceNode::ActionBar {
                            action_ids: vec![
                                surfaces::InteractionId::new("create")
                                    .expect("literal interaction id is valid"),
                                surfaces::InteractionId::new("configure_smtp")
                                    .expect("literal interaction id is valid"),
                            ],
                        },
                        surfaces::SurfaceNode::Table {
                            data_source_id: data_source_id.clone(),
                            columns: vec![
                                surfaces::SurfaceTableColumn {
                                    key: "name".to_string(),
                                    label: "Name".to_string(),
                                },
                                surfaces::SurfaceTableColumn {
                                    key: "to_addresses".to_string(),
                                    label: "Recipients".to_string(),
                                },
                                surfaces::SurfaceTableColumn {
                                    key: "enabled".to_string(),
                                    label: "Enabled".to_string(),
                                },
                                surfaces::SurfaceTableColumn {
                                    key: "created_at".to_string(),
                                    label: "Created".to_string(),
                                },
                            ],
                            row_actions: vec![
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("edit")
                                        .expect("literal interaction id is valid"),
                                    visible_when: None,
                                },
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("test")
                                        .expect("literal interaction id is valid"),
                                    visible_when: None,
                                },
                                surfaces::SurfaceTableRowAction {
                                    interaction_id: surfaces::InteractionId::new("delete")
                                        .expect("literal interaction id is valid"),
                                    visible_when: None,
                                },
                            ],
                        },
                    ],
                },
            },
            interactions: vec![
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("list")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::DataLoad,
                    label: Some("List".to_string()),
                    required_permission: None,
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("create")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: Some("Add Email Channel".to_string()),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: Some(surfaces::FormUiDescriptor {
                        fields: vec![
                            surfaces::FormFieldDescriptor {
                                key: "name".to_string(),
                                label: "Name".to_string(),
                                field_type: "text".to_string(),
                                required: true,
                                placeholder: None,
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "to_addresses".to_string(),
                                label: "Recipients".to_string(),
                                field_type: "textarea".to_string(),
                                required: true,
                                placeholder: Some(
                                    "user@example.com\nadmin@example.com".to_string(),
                                ),
                                help_text: Some("One email address per line".to_string()),
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "enabled".to_string(),
                                label: "Enabled".to_string(),
                                field_type: "toggle".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("true".to_string()),
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                        ],
                        pre_load_interaction_id: None,
                    }),
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("edit")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: Some("Edit".to_string()),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: Some(surfaces::FormUiDescriptor {
                        fields: vec![
                            surfaces::FormFieldDescriptor {
                                key: "id".to_string(),
                                label: "ID".to_string(),
                                field_type: "hidden".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "name".to_string(),
                                label: "Name".to_string(),
                                field_type: "text".to_string(),
                                required: true,
                                placeholder: None,
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "to_addresses".to_string(),
                                label: "Recipients".to_string(),
                                field_type: "textarea".to_string(),
                                required: true,
                                placeholder: Some(
                                    "user@example.com\nadmin@example.com".to_string(),
                                ),
                                help_text: Some("One email address per line".to_string()),
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "enabled".to_string(),
                                label: "Enabled".to_string(),
                                field_type: "toggle".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("true".to_string()),
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                        ],
                        pre_load_interaction_id: None,
                    }),
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("test")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: Some("Test".to_string()),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("delete")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::ConfirmableAction,
                    label: Some("Delete".to_string()),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: Some(surfaces::InteractionConfirmation {
                        title: "Confirm Delete".to_string(),
                        message: "This action may modify existing data.".to_string(),
                        confirm_label: None,
                        cancel_label: None,
                        severity: surfaces::ConfirmationSeverity::Danger,
                    }),
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("configure_smtp")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: Some("Override SMTP".to_string()),
                    required_permission: Some("manage_notifications".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec!["password".to_string()],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: Some(surfaces::FormUiDescriptor {
                        fields: vec![
                            surfaces::FormFieldDescriptor {
                                key: "host".to_string(),
                                label: "SMTP Host".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("smtp.example.com".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "port".to_string(),
                                label: "Port".to_string(),
                                field_type: "number".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("587".to_string()),
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "tls_mode".to_string(),
                                label: "TLS Mode".to_string(),
                                field_type: "select".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("starttls".to_string()),
                                options: vec![
                                    surfaces::FormSelectOption {
                                        value: "starttls".to_string(),
                                        label: "STARTTLS (port 587)".to_string(),
                                    },
                                    surfaces::FormSelectOption {
                                        value: "tls".to_string(),
                                        label: "TLS (port 465)".to_string(),
                                    },
                                    surfaces::FormSelectOption {
                                        value: "none".to_string(),
                                        label: "None (port 25)".to_string(),
                                    },
                                ],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "from_address".to_string(),
                                label: "From Address".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("noreply@example.com".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "from_name".to_string(),
                                label: "From Name".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("Uptrakit Notifications".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "username".to_string(),
                                label: "Username".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("SMTP username".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "password".to_string(),
                                label: "Password".to_string(),
                                field_type: "password".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: Some("Leave empty to keep current password".to_string()),
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                        ],
                        pre_load_interaction_id: Some(
                            surfaces::InteractionId::new("get_smtp")
                                .expect("literal interaction id is valid"),
                        ),
                    }),
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("get_smtp")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::DataLoad,
                    label: Some("Get SMTP Settings".to_string()),
                    required_permission: None,
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
            ],
            data_sources: vec![surfaces::DataSourceDescriptor {
                data_source_id,
                kind: surfaces::DataSourceKind::ProviderQuery {
                    operation_id: "list".to_string(),
                },
                result_schema: surfaces::SchemaContract::Array,
                pagination: Some(surfaces::DataSourcePagination {
                    default_page_size: 20,
                    max_page_size: 200,
                }),
                sorting: None,
                filtering: None,
                refresh_policy: surfaces::RefreshPolicy::Manual,
                empty_state: None,
            }],
        }
    };

    let global_smtp_surface = {
        let save_global_smtp_interaction = surfaces::InteractionId::new("save_global_smtp")
            .expect("literal interaction id is valid");
        surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("notifications.email.global_smtp")
                    .expect("literal surface id is valid"),
                label: "SMTP Defaults".to_string(),
                priority: 600,
                slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: Some("manage_global_settings".to_string()),
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::SectionNode,
                    surfaces::Capability::FormNode,
                    surfaces::Capability::ActionBarNode,
                    surfaces::Capability::DataLoad,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                    surfaces::Capability::SensitiveFields,
                ]),
                root_node: surfaces::SurfaceNode::Section {
                    title: None,
                    children: vec![
                        surfaces::SurfaceNode::Form {
                            interaction_id: save_global_smtp_interaction.clone(),
                        },
                        surfaces::SurfaceNode::ActionBar {
                            action_ids: vec![
                                surfaces::InteractionId::new("test_global_smtp_email")
                                    .expect("literal interaction id is valid"),
                            ],
                        },
                    ],
                },
            },
            interactions: vec![
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("get_global_smtp")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::DataLoad,
                    label: Some("Get Global SMTP Defaults".to_string()),
                    required_permission: None,
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("test_global_smtp_email")
                        .expect("literal interaction id is valid"),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: Some("Send Test Email".to_string()),
                    required_permission: Some("manage_global_settings".to_string()),
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec![],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                },
                surfaces::InteractionDescriptor {
                    interaction_id: save_global_smtp_interaction,
                    kind: surfaces::InteractionKind::MutationAction,
                    label: Some("Save Global SMTP Defaults".to_string()),
                    required_permission: Some("manage_global_settings".to_string()),
                    input_schema: None,
                    result_schema: Some(surfaces::SchemaContract::Any),
                    sensitive_fields: vec!["password".to_string()],
                    timeout_seconds: None,
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: Some(surfaces::FormUiDescriptor {
                        fields: vec![
                            surfaces::FormFieldDescriptor {
                                key: "host".to_string(),
                                label: "SMTP Host".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("smtp.example.com".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "port".to_string(),
                                label: "Port".to_string(),
                                field_type: "number".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("587".to_string()),
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "tls_mode".to_string(),
                                label: "TLS Mode".to_string(),
                                field_type: "select".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: None,
                                default_value: Some("starttls".to_string()),
                                options: vec![
                                    surfaces::FormSelectOption {
                                        value: "starttls".to_string(),
                                        label: "STARTTLS (port 587)".to_string(),
                                    },
                                    surfaces::FormSelectOption {
                                        value: "tls".to_string(),
                                        label: "TLS (port 465)".to_string(),
                                    },
                                    surfaces::FormSelectOption {
                                        value: "none".to_string(),
                                        label: "None (port 25)".to_string(),
                                    },
                                ],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "from_address".to_string(),
                                label: "From Address".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("noreply@example.com".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "from_name".to_string(),
                                label: "From Name".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("Uptrakit Notifications".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "helo_host".to_string(),
                                label: "EHLO Hostname".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("mail.example.com".to_string()),
                                help_text: Some(
                                    "Hostname sent in the SMTP EHLO command. Defaults to the domain of the From address. Set explicitly when using a relay server.".to_string(),
                                ),
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "username".to_string(),
                                label: "Username".to_string(),
                                field_type: "text".to_string(),
                                required: false,
                                placeholder: Some("SMTP username".to_string()),
                                help_text: None,
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                            surfaces::FormFieldDescriptor {
                                key: "password".to_string(),
                                label: "Password".to_string(),
                                field_type: "password".to_string(),
                                required: false,
                                placeholder: None,
                                help_text: Some("Leave empty to keep current password".to_string()),
                                default_value: None,
                                options: vec![],
                                select_source: None,
                                sensitive: false,
                                list: false,
                                visible_when: None,
                            },
                        ],
                        pre_load_interaction_id: Some(
                            surfaces::InteractionId::new("get_global_smtp")
                                .expect("literal interaction id is valid"),
                        ),
                    }),
                },
            ],
            data_sources: vec![],
        }
    };

    let surfaces = vec![channel_surface, global_smtp_surface];
    vec![surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "plugin.email".to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: collect_registration_capabilities(&surfaces),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces,
        encryption_metadata: None,
    }]
}

// ── declare_plugin! ──────────────────────────────────────────────────────────

declare_plugin!(EmailPlugin, EmailChannelConfig, "email", {
    display_name: "Email",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_email_transport,
    owned_surface_ids: &["notifications.email", "notifications.email.global_smtp"],
    raw_settings_keys: &[
        "smtp.host", "smtp.port", "smtp.username", "smtp.password",
        "smtp.from_address", "smtp.from_name", "smtp.tls_mode",
        "global_smtp.host", "global_smtp.port", "global_smtp.username", "global_smtp.password",
        "global_smtp.from_address", "global_smtp.from_name", "global_smtp.tls_mode",
        "global_smtp.helo_host",
    ],
    surface_actions: {
        actions: email_surface_actions,
        handle_action: email_handle_surface_action,
    },
    surfaces: {
        registrations: email_surface_registrations,
    },
});

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginMeta as _, surfaces};

    #[test]
    fn plugin_type_id() {
        let plugin = EmailPlugin;
        assert_eq!(plugin.plugin_type_id().as_str(), "email");
    }

    // ── Descriptor tests ─────────────────────────────────────────────────

    #[test]
    fn descriptor_type_id() {
        assert_eq!(DESCRIPTOR.type_id, "email");
        assert_eq!(DESCRIPTOR.display_name, "Email");
    }

    #[test]
    fn descriptor_family_and_config_model() {
        assert_eq!(DESCRIPTOR.family, PluginFamily::Notification);
        assert_eq!(DESCRIPTOR.config_model, ConfigModel::NotificationChannel);
    }

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::NotificationDelivery)
        );
    }

    #[test]
    fn descriptor_has_notification_transport() {
        assert!(DESCRIPTOR.roles.notification_transport.is_some());
    }

    #[test]
    fn descriptor_has_no_software_roles() {
        assert!(DESCRIPTOR.roles.discoverer.is_none());
        assert!(DESCRIPTOR.roles.version_detector.is_none());
        assert!(DESCRIPTOR.roles.release_fetcher.is_none());
        assert!(DESCRIPTOR.roles.package_indexer.is_none());
        assert!(DESCRIPTOR.roles.update_executor.is_none());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }

    #[test]
    fn descriptor_has_surface_actions() {
        assert!(DESCRIPTOR.surface_actions.is_some());
        let surface_actions = DESCRIPTOR.surface_actions.unwrap();
        assert!(
            surface_actions
                .owned_surface_ids()
                .contains(&"notifications.email")
        );
        assert!(
            surface_actions
                .owned_surface_ids()
                .contains(&"notifications.email.global_smtp")
        );
    }

    #[test]
    fn descriptor_has_plugin_surface_registrations() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        assert!(!registrations.is_empty());
        assert!(registrations.iter().all(|registration| {
            registration.provider.provider_kind
                == uptrakit_plugin_infrastructure_core::surfaces::ProviderKind::Plugin
        }));
        let all_surface_ids: Vec<String> = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .map(|surface| surface.descriptor.surface_id.to_string())
            .collect();
        assert!(
            all_surface_ids
                .iter()
                .any(|id| id == "notifications.email.global_smtp")
        );
        assert!(
            all_surface_ids.iter().any(|id| id == "notifications.email"),
            "notification channel data-table should be registered as an action-driven shared surface"
        );
    }

    #[test]
    fn email_channel_surface_keeps_table_data_and_action_contract() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let channel_surface = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| surface.descriptor.surface_id.as_str() == "notifications.email")
            .expect("notifications.email surface should be present");

        assert_eq!(
            channel_surface.descriptor.slot,
            surfaces::SLOT_SETTINGS_TABS
        );
        assert_eq!(channel_surface.data_sources.len(), 1);
        assert!(matches!(
            &channel_surface.data_sources[0].kind,
            surfaces::DataSourceKind::ProviderQuery { operation_id } if operation_id == "list"
        ));

        match &channel_surface.descriptor.root_node {
            surfaces::SurfaceNode::Section { children, .. } => {
                assert!(matches!(
                    children.first(),
                    Some(surfaces::SurfaceNode::ActionBar { action_ids })
                        if action_ids.iter().map(|id| id.as_str()).collect::<Vec<_>>()
                            == vec!["create", "configure_smtp"]
                ));
                assert!(matches!(
                    children.get(1),
                    Some(surfaces::SurfaceNode::Table { row_actions, .. })
                        if row_actions
                            .iter()
                            .map(|action| action.interaction_id.as_str())
                            .collect::<Vec<_>>()
                            == vec!["edit", "test", "delete"]
                ));
            }
            other => panic!("expected section root node, got {other:?}"),
        }

        let find_interaction = |id: &str| {
            channel_surface
                .interactions
                .iter()
                .find(|interaction| interaction.interaction_id.as_str() == id)
                .unwrap_or_else(|| panic!("interaction `{id}` should exist"))
        };

        assert_eq!(
            find_interaction("list").kind,
            surfaces::InteractionKind::DataLoad
        );
        assert_eq!(
            find_interaction("create").kind,
            surfaces::InteractionKind::FormSubmit
        );
        assert_eq!(
            find_interaction("edit").kind,
            surfaces::InteractionKind::FormSubmit
        );
        assert_eq!(
            find_interaction("configure_smtp").kind,
            surfaces::InteractionKind::FormSubmit
        );
        assert_eq!(
            find_interaction("test").kind,
            surfaces::InteractionKind::MutationAction
        );
        assert_eq!(
            find_interaction("delete").kind,
            surfaces::InteractionKind::ConfirmableAction
        );
        assert_eq!(
            find_interaction("get_smtp").kind,
            surfaces::InteractionKind::DataLoad
        );

        assert!(find_interaction("delete").confirmation.is_some());
        let configure_smtp = find_interaction("configure_smtp");
        assert!(
            configure_smtp
                .sensitive_fields
                .iter()
                .any(|field| field == "password")
        );
        assert_eq!(
            configure_smtp
                .form_ui
                .as_ref()
                .and_then(|form_ui| form_ui.pre_load_interaction_id.as_ref())
                .map(|interaction_id| interaction_id.as_str()),
            Some("get_smtp")
        );
    }

    #[test]
    fn email_global_smtp_surface_keeps_form_submit_shape() {
        let registrations = (DESCRIPTOR
            .surfaces
            .expect("surfaces are registered")
            .registrations)();
        let smtp_surface = registrations
            .iter()
            .flat_map(|registration| registration.surfaces.iter())
            .find(|surface| {
                surface.descriptor.surface_id.as_str() == "notifications.email.global_smtp"
            })
            .expect("notifications.email.global_smtp surface should be present");

        assert_eq!(
            smtp_surface.descriptor.slot,
            surfaces::SLOT_SETTINGS_BELOW_GLOBAL
        );
        match &smtp_surface.descriptor.root_node {
            surfaces::SurfaceNode::Section { children, .. } => {
                assert!(matches!(
                    children.first(),
                    Some(surfaces::SurfaceNode::Form { interaction_id })
                        if interaction_id.as_str() == "save_global_smtp"
                ));
                assert!(matches!(
                    children.get(1),
                    Some(surfaces::SurfaceNode::ActionBar { action_ids })
                        if action_ids.iter().map(|id| id.as_str()).collect::<Vec<_>>()
                            == vec!["test_global_smtp_email"]
                ));
            }
            other => panic!("expected section root node, got {other:?}"),
        }

        let save = smtp_surface
            .interactions
            .iter()
            .find(|interaction| interaction.interaction_id.as_str() == "save_global_smtp")
            .expect("save_global_smtp interaction should exist");
        assert_eq!(save.kind, surfaces::InteractionKind::MutationAction);
        assert!(
            save.sensitive_fields
                .iter()
                .any(|field| field == "password")
        );
        assert_eq!(
            save.form_ui
                .as_ref()
                .and_then(|form_ui| form_ui.pre_load_interaction_id.as_ref())
                .map(|interaction_id| interaction_id.as_str()),
            Some("get_global_smtp")
        );
        assert!(smtp_surface.interactions.iter().any(|interaction| {
            interaction.interaction_id.as_str() == "get_global_smtp"
                && interaction.kind == surfaces::InteractionKind::DataLoad
        }));
        assert!(smtp_surface.interactions.iter().any(|interaction| {
            interaction.interaction_id.as_str() == "test_global_smtp_email"
                && interaction.kind == surfaces::InteractionKind::MutationAction
        }));
    }

    #[test]
    fn descriptor_raw_settings_keys() {
        assert!(!DESCRIPTOR.raw_settings_keys.is_empty());
        assert!(DESCRIPTOR.raw_settings_keys.contains(&"smtp.host"));
        assert!(DESCRIPTOR.raw_settings_keys.contains(&"global_smtp.host"));
        assert!(
            DESCRIPTOR
                .raw_settings_keys
                .contains(&"global_smtp.helo_host")
        );
    }

    // ── Config operations via descriptor ──────────────────────────────────

    #[test]
    fn descriptor_validate_config_rejects_empty_to_addresses() {
        let config = serde_json::json!({"to_addresses": []});
        let err = (DESCRIPTOR.config.validate)(&config).unwrap_err();
        assert!(
            err.contains("to_addresses"),
            "expected to_addresses mention, got: {err}"
        );
    }

    #[test]
    fn descriptor_validate_config_rejects_missing_to_addresses() {
        let config = serde_json::json!({});
        let err = (DESCRIPTOR.config.validate)(&config).unwrap_err();
        assert!(!err.is_empty(), "should produce an error for missing field");
    }

    #[test]
    fn descriptor_validate_config_rejects_invalid_email_format() {
        let config = serde_json::json!({"to_addresses": ["not-an-email"]});
        let err = (DESCRIPTOR.config.validate)(&config).unwrap_err();
        assert!(
            err.contains("invalid email address"),
            "expected invalid email error, got: {err}"
        );
    }

    #[test]
    fn descriptor_validate_config_rejects_email_without_dot_in_domain() {
        let config = serde_json::json!({"to_addresses": ["user@nodomain"]});
        let err = (DESCRIPTOR.config.validate)(&config).unwrap_err();
        assert!(
            err.contains("invalid email address"),
            "expected invalid email error, got: {err}"
        );
    }

    #[test]
    fn descriptor_validate_config_accepts_valid_config() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        assert!((DESCRIPTOR.config.validate)(&config).is_ok());
    }

    #[test]
    fn descriptor_validate_config_accepts_multiple_valid_addresses() {
        let config = serde_json::json!({
            "to_addresses": ["alice@example.com", "bob@example.org"]
        });
        assert!((DESCRIPTOR.config.validate)(&config).is_ok());
    }

    #[test]
    fn descriptor_mask_secrets_returns_config_unchanged() {
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let masked = (DESCRIPTOR.config.mask_secrets)(&config);
        assert_eq!(masked, config, "per-channel config has no secrets to mask");
    }

    #[test]
    fn descriptor_sample_config() {
        let sample = (DESCRIPTOR.config.sample)();
        assert!(sample.is_object());
    }

    // ── deliver ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn deliver_returns_error_on_missing_required_fields() {
        // Config missing smtp_host and from_address should fail deserialization or validation.
        let config = serde_json::json!({"to_addresses": ["user@example.com"]});
        let msg = DeliveryMessage::new("Test", "Body", None, serde_json::json!({}), vec![]);
        let empty_settings = serde_json::json!({});
        let plugin = EmailPlugin;
        let result = uptrakit_plugin_infrastructure_core::NotificationTransport::deliver(
            &plugin,
            &config,
            &empty_settings,
            &msg,
        )
        .await;
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
        let plugin = EmailPlugin;
        let result = uptrakit_plugin_infrastructure_core::NotificationTransport::deliver(
            &plugin,
            &config,
            &empty_settings,
            &msg,
        )
        .await;
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

    // ── EHLO hostname derivation ──────────────────────────────────────────

    /// Helper: build an EmailConfig JSON and derive the EHLO host the same way
    /// `send_email` does (inline logic test -- no SMTP connection required).
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

    // ── Extension actions ─────────────────────────────────────────────────

    #[test]
    fn surface_actions_not_empty() {
        let actions = email_surface_actions();
        assert!(!actions.is_empty());
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert!(ids.contains(&"list"));
        assert!(ids.contains(&"create"));
        assert!(ids.contains(&"edit"));
        assert!(ids.contains(&"test"));
        assert!(ids.contains(&"delete"));
        assert!(ids.contains(&"configure_smtp"));
        assert!(ids.contains(&"get_smtp"));
        assert!(ids.contains(&"test_global_smtp_email"));
        assert!(ids.contains(&"get_global_smtp"));
        assert!(ids.contains(&"save_global_smtp"));
    }
}
