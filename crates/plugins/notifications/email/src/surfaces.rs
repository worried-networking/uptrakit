//! Surface action handlers for the email notification plugin.
//!
//! Handles SMTP settings management (per-tenant and global) and channel
//! listing.

use serde::Deserialize;
use uptrakit_notification_plugin_core::DeliveryMessage;
use uptrakit_plugin_infrastructure_core::{
    EmailSmtpSettings, EmailSmtpSettingsPatch, NotificationChannelListRequest,
    NotificationTransport as _, SurfaceActionContext, SurfaceActionError,
};
use uptrakit_shared_types::SecretString;

use crate::{EmailPlugin, SmtpSettingsSnapshot, merge_smtp_into_config};

#[derive(Debug, Default, serde::Deserialize)]
struct ListActionParams {
    #[serde(default, deserialize_with = "deserialize_lenient_optional_u64")]
    page: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_u64")]
    per_page: Option<u64>,
}

impl ListActionParams {
    fn page(&self) -> u64 {
        self.page.unwrap_or(1).max(1)
    }

    fn per_page(&self) -> u64 {
        self.per_page.unwrap_or(50).clamp(1, 100)
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct SmtpPatchActionParams {
    #[serde(default, deserialize_with = "deserialize_lenient_optional_string")]
    host: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_port")]
    port: Option<u16>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_string")]
    username: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_string")]
    password: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_string")]
    from_address: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_string")]
    from_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_string")]
    tls_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_string")]
    helo_host: Option<String>,
}

impl SmtpPatchActionParams {
    fn into_patch(self) -> EmailSmtpSettingsPatch {
        EmailSmtpSettingsPatch {
            host: self.host.map(Some),
            port: self.port.map(Some),
            username: self.username.map(Some),
            password: self.password.filter(|value| !value.is_empty()).map(Some),
            from_address: self.from_address.map(Some),
            from_name: self.from_name.map(Some),
            tls_mode: self.tls_mode.map(Some),
            helo_host: self.helo_host.map(Some),
        }
    }
}

// ── Raw settings key constants ────────────────────────────────────────────────

/// Key prefix for per-tenant SMTP settings.
pub const SMTP_PREFIX: &str = "smtp.";
/// Key prefix for global SMTP settings.
pub const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";

// Per-tenant SMTP settings (stored in the `settings` table)
pub const KEY_SMTP_HOST: &str = "smtp.host";
pub const KEY_SMTP_PORT: &str = "smtp.port";
pub const KEY_SMTP_USERNAME: &str = "smtp.username";
pub const KEY_SMTP_PASSWORD: &str = "smtp.password";
pub const KEY_SMTP_FROM_ADDRESS: &str = "smtp.from_address";
pub const KEY_SMTP_FROM_NAME: &str = "smtp.from_name";
pub const KEY_SMTP_TLS_MODE: &str = "smtp.tls_mode";

// Global SMTP defaults (stored in the `global_settings` table)
pub const KEY_GLOBAL_SMTP_HOST: &str = "global_smtp.host";
pub const KEY_GLOBAL_SMTP_PORT: &str = "global_smtp.port";
pub const KEY_GLOBAL_SMTP_USERNAME: &str = "global_smtp.username";
pub const KEY_GLOBAL_SMTP_PASSWORD: &str = "global_smtp.password";
pub const KEY_GLOBAL_SMTP_FROM_ADDRESS: &str = "global_smtp.from_address";
pub const KEY_GLOBAL_SMTP_FROM_NAME: &str = "global_smtp.from_name";
pub const KEY_GLOBAL_SMTP_TLS_MODE: &str = "global_smtp.tls_mode";
pub const KEY_GLOBAL_SMTP_HELO_HOST: &str = "global_smtp.helo_host";

/// All raw settings keys written by the email plugin to the `settings` and
/// `global_settings` tables.
pub const RAW_SETTINGS_KEYS: &[&str] = &[
    KEY_SMTP_HOST,
    KEY_SMTP_PORT,
    KEY_SMTP_USERNAME,
    KEY_SMTP_PASSWORD,
    KEY_SMTP_FROM_ADDRESS,
    KEY_SMTP_FROM_NAME,
    KEY_SMTP_TLS_MODE,
    KEY_GLOBAL_SMTP_HOST,
    KEY_GLOBAL_SMTP_PORT,
    KEY_GLOBAL_SMTP_USERNAME,
    KEY_GLOBAL_SMTP_PASSWORD,
    KEY_GLOBAL_SMTP_FROM_ADDRESS,
    KEY_GLOBAL_SMTP_FROM_NAME,
    KEY_GLOBAL_SMTP_TLS_MODE,
    KEY_GLOBAL_SMTP_HELO_HOST,
];

/// Handle a surface action for the email notification plugin.
#[tracing::instrument(skip_all, fields(surface_id, action_id))]
pub async fn handle_surface_action(
    ctx: &SurfaceActionContext<'_>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    match action_id {
        "list" => handle_list(ctx, &params).await,
        "get_smtp" => handle_get_smtp(ctx).await,
        "configure_smtp" => handle_configure_smtp(ctx, &params).await,
        "get_global_smtp" => handle_get_global_smtp(ctx).await,
        "save_global_smtp" => handle_save_global_smtp(ctx, &params).await,
        "test_global_smtp_email" => handle_test_global_smtp_email(ctx).await,
        _ => Err(SurfaceActionError::InvalidInput(format!(
            "unknown action '{action_id}' for surface '{surface_id}'",
        ))),
    }
}

// ── List channels ────────────────────────────────────────────────────────────

async fn handle_list(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let request_params = parse_action_params::<ListActionParams>(params, "list");
    let store = require_notification_channel_store(ctx)?;
    let page = store
        .list_channels(NotificationChannelListRequest {
            tenant_id: ctx.tenant_id(),
            channel_type: "email",
            page: request_params.page(),
            per_page: request_params.per_page(),
        })
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "failed to list email channels");
            SurfaceActionError::ControllerIntegration("failed to list channels".to_string())
        })?;

    let mut items = Vec::with_capacity(page.items.len());
    for channel in page.items {
        let mut row = serde_json::json!({
            "id": channel.id,
            "name": channel.name,
            "enabled": channel.enabled,
            "created_at": channel.created_at_rfc3339,
        });
        if let (Some(config), Some(row_obj)) = (channel.config.as_object(), row.as_object_mut()) {
            for (key, value) in config {
                row_obj.insert(key.clone(), value.clone());
            }
        }
        items.push(row);
    }

    Ok(serde_json::json!({
        "items": items,
        "total": page.total,
        "page": page.page,
        "per_page": page.per_page,
        "total_pages": page.total_pages,
    }))
}

// ── Per-tenant SMTP settings ─────────────────────────────────────────────────

async fn handle_get_smtp(
    ctx: &SurfaceActionContext<'_>,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let store = require_email_smtp_store(ctx)?;
    let tenant_smtp = store
        .load_tenant_smtp_settings(ctx.tenant_id())
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "failed to load tenant SMTP settings");
            SurfaceActionError::ControllerIntegration(
                "failed to load tenant SMTP settings".to_string(),
            )
        })?;
    let global_smtp = store.load_global_smtp_settings().await.map_err(|error| {
        tracing::error!(error = ?error, "failed to load global SMTP settings");
        SurfaceActionError::ControllerIntegration("failed to load global SMTP settings".to_string())
    })?;

    let smtp = smtp_snapshot_from_store(tenant_smtp);
    let global = smtp_snapshot_from_store(global_smtp);

    Ok(serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
        "effective_host": smtp.host.as_ref().or(global.host.as_ref()).cloned().unwrap_or_default(),
        "effective_from_address": smtp.from_address.as_ref().or(global.from_address.as_ref()).cloned().unwrap_or_default(),
        "has_global_defaults": global.is_configured(),
    }))
}

async fn handle_configure_smtp(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let request_params = parse_action_params::<SmtpPatchActionParams>(params, "configure_smtp");
    let store = require_email_smtp_store(ctx)?;
    let smtp = store
        .save_tenant_smtp_settings(ctx.tenant_id(), request_params.into_patch())
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "failed to save tenant SMTP settings");
            SurfaceActionError::ControllerIntegration(
                "failed to save tenant SMTP settings".to_string(),
            )
        })?;
    let smtp = smtp_snapshot_from_store(smtp);
    Ok(smtp_json(&smtp))
}

// ── Global SMTP settings ─────────────────────────────────────────────────────

async fn handle_get_global_smtp(
    ctx: &SurfaceActionContext<'_>,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let store = require_email_smtp_store(ctx)?;
    let smtp = store.load_global_smtp_settings().await.map_err(|error| {
        tracing::error!(error = ?error, "failed to load global SMTP settings");
        SurfaceActionError::ControllerIntegration("failed to load global SMTP settings".to_string())
    })?;
    let smtp = smtp_snapshot_from_store(smtp);
    Ok(global_smtp_json(&smtp))
}

async fn handle_save_global_smtp(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let request_params = parse_action_params::<SmtpPatchActionParams>(params, "save_global_smtp");
    let store = require_email_smtp_store(ctx)?;
    let smtp = store
        .save_global_smtp_settings(request_params.into_patch())
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "failed to save global SMTP settings");
            SurfaceActionError::ControllerIntegration(
                "failed to save global SMTP settings".to_string(),
            )
        })?;
    let smtp = smtp_snapshot_from_store(smtp);
    Ok(global_smtp_json(&smtp))
}

// ── Test global SMTP email ───────────────────────────────────────────────────

async fn handle_test_global_smtp_email(
    ctx: &SurfaceActionContext<'_>,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let caller_user_id = ctx.caller_user_id().ok_or_else(|| {
        SurfaceActionError::InvalidInput(
            "caller_user_id is required for test_global_smtp_email".to_string(),
        )
    })?;

    let store = require_email_smtp_store(ctx)?;
    let to_address = store
        .load_user_email(caller_user_id)
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "failed to load user for test email");
            SurfaceActionError::ControllerIntegration("failed to load user email".to_string())
        })?
        .ok_or_else(|| SurfaceActionError::InvalidInput("User not found".to_string()))?;

    let global_smtp = store.load_global_smtp_settings().await.map_err(|error| {
        tracing::error!(error = ?error, "failed to load global SMTP settings for test email");
        SurfaceActionError::ControllerIntegration("failed to load global SMTP settings".to_string())
    })?;
    let global_smtp = smtp_snapshot_from_store(global_smtp);

    if !global_smtp.is_configured() {
        return Err(SurfaceActionError::InvalidInput(
            "Global SMTP is not configured. Set SMTP host and from address before sending a test email."
                .to_string(),
        ));
    }

    let empty_smtp = SmtpSettingsSnapshot {
        host: None,
        port: None,
        username: None,
        password: None,
        from_address: None,
        from_name: None,
        tls_mode: "starttls".to_string(),
        helo_host: None,
    };
    let config = merge_smtp_into_config(
        &global_smtp,
        &empty_smtp,
        serde_json::json!({ "to_addresses": [to_address] }),
    );

    let plugin = EmailPlugin;
    let test_msg = DeliveryMessage::new(
        "Test Email from Uptrakit",
        "This is a test email sent from the Global SMTP settings page.",
        None,
        serde_json::json!({}),
        vec![],
    );
    plugin
        .deliver(&config, &serde_json::json!({}), &test_msg)
        .await
        .map_err(|error| {
            tracing::warn!(error = ?error, "test global smtp email failed");
            SurfaceActionError::PluginInternal(error.to_string())
        })?;

    let success_msg = format!("Test email sent successfully to {to_address}");
    Ok(serde_json::json!({
        "success": true,
        "message": success_msg,
    }))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn require_notification_channel_store<'a>(
    ctx: &'a SurfaceActionContext<'a>,
) -> std::result::Result<
    &'a dyn uptrakit_plugin_infrastructure_core::NotificationChannelStore,
    SurfaceActionError,
> {
    ctx.controller.notification_channel_store().ok_or_else(|| {
        SurfaceActionError::ControllerIntegration(
            "notification channel store is not available".to_string(),
        )
    })
}

fn require_email_smtp_store<'a>(
    ctx: &'a SurfaceActionContext<'a>,
) -> std::result::Result<
    &'a dyn uptrakit_plugin_infrastructure_core::EmailSmtpSettingsStore,
    SurfaceActionError,
> {
    ctx.controller.email_smtp_settings_store().ok_or_else(|| {
        SurfaceActionError::ControllerIntegration(
            "email SMTP settings store is not available".to_string(),
        )
    })
}

fn parse_action_params<T>(params: &serde_json::Value, action_id: &str) -> T
where
    T: serde::de::DeserializeOwned + Default,
{
    let params_object = params.as_object().cloned().unwrap_or_default();
    let normalized = serde_json::Value::Object(params_object);
    match serde_json::from_value(normalized) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                action_id,
                error = ?error,
                "failed to deserialize action params; using defaults"
            );
            T::default()
        }
    }
}

fn deserialize_lenient_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| value.as_u64()))
}

fn deserialize_lenient_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| value.as_str().map(str::to_string)))
}

fn deserialize_lenient_optional_port<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| parse_port_value(&value)))
}

fn parse_port_value(value: &serde_json::Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|raw| u16::try_from(raw).ok())
        .or_else(|| value.as_str().and_then(|raw| raw.parse::<u16>().ok()))
}

fn smtp_snapshot_from_store(settings: EmailSmtpSettings) -> SmtpSettingsSnapshot {
    SmtpSettingsSnapshot {
        host: settings.host,
        port: settings.port,
        username: settings.username,
        password: settings.password.map(SecretString::new),
        from_address: settings.from_address,
        from_name: settings.from_name,
        tls_mode: normalize_tls_mode(settings.tls_mode),
        helo_host: settings.helo_host,
    }
}

fn normalize_tls_mode(tls_mode: Option<String>) -> String {
    match tls_mode {
        Some(value) if matches!(value.as_str(), "starttls" | "tls" | "none") => value,
        Some(_) | None => "starttls".to_string(),
    }
}

fn smtp_json(smtp: &SmtpSettingsSnapshot) -> serde_json::Value {
    serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
    })
}

fn global_smtp_json(smtp: &SmtpSettingsSnapshot) -> serde_json::Value {
    serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "helo_host": smtp.helo_host.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ListActionParams, SmtpPatchActionParams, normalize_tls_mode, parse_action_params,
        smtp_snapshot_from_store,
    };
    use uptrakit_plugin_infrastructure_core::{EmailSmtpSettings, EmailSmtpSettingsPatch};

    #[test]
    fn smtp_snapshot_normalizes_unknown_tls_mode_to_starttls() {
        let snapshot = smtp_snapshot_from_store(EmailSmtpSettings {
            host: None,
            port: None,
            username: None,
            password: None,
            from_address: None,
            from_name: None,
            tls_mode: Some("legacy".to_string()),
            helo_host: None,
        });

        assert_eq!(snapshot.tls_mode, "starttls");
    }

    #[test]
    fn normalize_tls_mode_preserves_supported_values() {
        assert_eq!(normalize_tls_mode(Some("starttls".to_string())), "starttls");
        assert_eq!(normalize_tls_mode(Some("tls".to_string())), "tls");
        assert_eq!(normalize_tls_mode(Some("none".to_string())), "none");
        assert_eq!(normalize_tls_mode(None), "starttls");
    }

    #[test]
    fn list_action_params_keep_legacy_defaults_and_bounds() {
        let defaults = parse_action_params::<ListActionParams>(&serde_json::json!({}), "list");
        assert_eq!(defaults.page(), 1);
        assert_eq!(defaults.per_page(), 50);

        let bounded = parse_action_params::<ListActionParams>(
            &serde_json::json!({
                "page": 0,
                "per_page": 999,
            }),
            "list",
        );
        assert_eq!(bounded.page(), 1);
        assert_eq!(bounded.per_page(), 100);

        let string_values = parse_action_params::<ListActionParams>(
            &serde_json::json!({
                "page": "3",
                "per_page": "20",
            }),
            "list",
        );
        assert_eq!(string_values.page(), 1);
        assert_eq!(string_values.per_page(), 50);
    }

    #[test]
    fn smtp_patch_action_params_keep_port_and_password_compatibility() {
        let params = parse_action_params::<SmtpPatchActionParams>(
            &serde_json::json!({
                "host": "smtp.example.com",
                "port": "465",
                "password": "",
                "from_address": "alerts@example.com",
            }),
            "configure_smtp",
        );
        let patch = params.into_patch();

        assert_eq!(patch.host, Some(Some("smtp.example.com".to_string())));
        assert_eq!(patch.port, Some(Some(465)));
        assert_eq!(patch.password, None, "empty passwords must not update");
        assert_eq!(
            patch.from_address,
            Some(Some("alerts@example.com".to_string()))
        );
    }

    #[test]
    fn action_param_parsing_treats_non_object_payload_as_empty_object() {
        let list_params =
            parse_action_params::<ListActionParams>(&serde_json::json!("oops"), "list");
        assert_eq!(list_params.page(), 1);
        assert_eq!(list_params.per_page(), 50);

        let smtp_patch = parse_action_params::<SmtpPatchActionParams>(
            &serde_json::json!(["not", "an", "object"]),
            "configure_smtp",
        )
        .into_patch();
        assert_eq!(smtp_patch, EmailSmtpSettingsPatch::default());
    }
}
