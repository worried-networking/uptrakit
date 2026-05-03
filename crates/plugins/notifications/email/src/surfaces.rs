//! Surface action handlers for the email notification plugin.
//!
//! Handles SMTP settings management (per-tenant and global) and channel
//! listing.

use serde::Deserialize;
use uptrakit_notification_plugin_core::DeliveryMessage;
use uptrakit_plugin_infrastructure_core::{
    NotificationChannelListRequest, NotificationTransport as _, SurfaceActionContext,
    SurfaceActionError,
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
pub const KEY_SMTP_HELO_HOST: &str = "smtp.helo_host";

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
    KEY_SMTP_HELO_HOST,
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
    let db = ctx.tenant_db().db();
    let tenant_id = ctx.tenant_id();
    let smtp = db_load_tenant_smtp(db, tenant_id).await?;
    let global = db_load_global_smtp(db).await?;

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
    let db = ctx.tenant_db().db();
    let tenant_id = ctx.tenant_id();
    db_save_tenant_smtp(db, tenant_id, &request_params).await?;
    let smtp = db_load_tenant_smtp(db, tenant_id).await?;
    Ok(smtp_json(&smtp))
}

// ── Global SMTP settings ─────────────────────────────────────────────────────

async fn handle_get_global_smtp(
    ctx: &SurfaceActionContext<'_>,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let smtp = db_load_global_smtp(ctx.tenant_db().db()).await?;
    Ok(global_smtp_json(&smtp))
}

async fn handle_save_global_smtp(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let request_params = parse_action_params::<SmtpPatchActionParams>(params, "save_global_smtp");
    let db = ctx.tenant_db().db();
    db_save_global_smtp(db, &request_params).await?;
    let smtp = db_load_global_smtp(db).await?;
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

    let db = ctx.tenant_db().db();
    let to_address = db_load_user_email(db, caller_user_id)
        .await?
        .ok_or_else(|| SurfaceActionError::InvalidInput("User not found".to_string()))?;

    let global_smtp = db_load_global_smtp(db).await?;

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

// ── DB helpers (replaces EmailSmtpSettingsStore) ─────────────────────────────

const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

async fn db_load_tenant_smtp(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> std::result::Result<SmtpSettingsSnapshot, SurfaceActionError> {
    let raw = uptrakit_shared_db::raw_settings::load_settings_by_prefix(db, tenant_id, SMTP_PREFIX)
        .await
        .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;
    Ok(settings_map_to_snapshot(
        &raw,
        KEY_SMTP_HOST,
        KEY_SMTP_PORT,
        KEY_SMTP_USERNAME,
        KEY_SMTP_PASSWORD,
        KEY_SMTP_FROM_ADDRESS,
        KEY_SMTP_FROM_NAME,
        KEY_SMTP_TLS_MODE,
        KEY_SMTP_HELO_HOST,
        SMTP_PASSWORD_AAD,
        "tenant",
        Some(tenant_id),
    ))
}

async fn db_load_global_smtp(
    db: &sea_orm::DatabaseConnection,
) -> std::result::Result<SmtpSettingsSnapshot, SurfaceActionError> {
    let raw =
        uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, GLOBAL_SMTP_PREFIX)
            .await
            .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;
    Ok(settings_map_to_snapshot(
        &raw,
        KEY_GLOBAL_SMTP_HOST,
        KEY_GLOBAL_SMTP_PORT,
        KEY_GLOBAL_SMTP_USERNAME,
        KEY_GLOBAL_SMTP_PASSWORD,
        KEY_GLOBAL_SMTP_FROM_ADDRESS,
        KEY_GLOBAL_SMTP_FROM_NAME,
        KEY_GLOBAL_SMTP_TLS_MODE,
        KEY_GLOBAL_SMTP_HELO_HOST,
        GLOBAL_SMTP_PASSWORD_AAD,
        "global",
        None,
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "each SMTP field has a distinct key name; a builder or struct would add more indirection without clarity gain"
)]
fn settings_map_to_snapshot(
    raw: &std::collections::HashMap<String, serde_json::Value>,
    host_key: &str,
    port_key: &str,
    username_key: &str,
    password_key: &str,
    from_address_key: &str,
    from_name_key: &str,
    tls_mode_key: &str,
    helo_host_key: &str,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<uuid::Uuid>,
) -> SmtpSettingsSnapshot {
    let get_str = |key: &str| -> Option<String> {
        raw.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let get_u16 = |key: &str| -> Option<u16> {
        raw.get(key).and_then(|v| {
            v.as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
    };
    let password = get_str(password_key).and_then(|raw_str| {
        if uptrakit_crypto::is_encrypted(&raw_str) {
            match uptrakit_crypto::decrypt_str(&raw_str, password_aad) {
                Ok(v) if !v.is_empty() => Some(v),
                Ok(_) => None,
                Err(e) => {
                    if let Some(tid) = tenant_id {
                        tracing::warn!(error = ?e, %tid, scope, "failed to decrypt SMTP password");
                    } else {
                        tracing::warn!(error = ?e, scope, "failed to decrypt SMTP password");
                    }
                    None
                }
            }
        } else {
            Some(raw_str)
        }
    }).map(SecretString::new);

    SmtpSettingsSnapshot {
        host: get_str(host_key),
        port: get_u16(port_key),
        username: get_str(username_key),
        password,
        from_address: get_str(from_address_key),
        from_name: get_str(from_name_key),
        tls_mode: normalize_tls_mode(get_str(tls_mode_key)),
        helo_host: get_str(helo_host_key),
    }
}

async fn db_save_tenant_smtp(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    patch: &SmtpPatchActionParams,
) -> std::result::Result<(), SurfaceActionError> {
    if let Some(ref v) = patch.host {
        upsert_tenant_raw(
            db,
            tenant_id,
            KEY_SMTP_HOST,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(v) = patch.port {
        upsert_tenant_raw(db, tenant_id, KEY_SMTP_PORT, Some(serde_json::json!(v))).await?;
    }
    if let Some(ref v) = patch.username {
        upsert_tenant_raw(
            db,
            tenant_id,
            KEY_SMTP_USERNAME,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.password {
        // Empty passwords are not stored — leave the existing value unchanged.
        if !v.is_empty() {
            let enc = uptrakit_crypto::encrypt_str(v.as_str(), SMTP_PASSWORD_AAD)
                .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;
            upsert_tenant_raw(
                db,
                tenant_id,
                KEY_SMTP_PASSWORD,
                Some(serde_json::Value::String(enc)),
            )
            .await?;
        }
    }
    if let Some(ref v) = patch.from_address {
        upsert_tenant_raw(
            db,
            tenant_id,
            KEY_SMTP_FROM_ADDRESS,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.from_name {
        upsert_tenant_raw(
            db,
            tenant_id,
            KEY_SMTP_FROM_NAME,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.tls_mode {
        upsert_tenant_raw(
            db,
            tenant_id,
            KEY_SMTP_TLS_MODE,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.helo_host {
        upsert_tenant_raw(
            db,
            tenant_id,
            KEY_SMTP_HELO_HOST,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    Ok(())
}

async fn db_save_global_smtp(
    db: &sea_orm::DatabaseConnection,
    patch: &SmtpPatchActionParams,
) -> std::result::Result<(), SurfaceActionError> {
    if let Some(ref v) = patch.host {
        upsert_global_raw(
            db,
            KEY_GLOBAL_SMTP_HOST,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(v) = patch.port {
        upsert_global_raw(db, KEY_GLOBAL_SMTP_PORT, Some(serde_json::json!(v))).await?;
    }
    if let Some(ref v) = patch.username {
        upsert_global_raw(
            db,
            KEY_GLOBAL_SMTP_USERNAME,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.password {
        // Empty passwords are not stored — leave the existing value unchanged.
        if !v.is_empty() {
            let enc = uptrakit_crypto::encrypt_str(v.as_str(), GLOBAL_SMTP_PASSWORD_AAD)
                .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;
            upsert_global_raw(
                db,
                KEY_GLOBAL_SMTP_PASSWORD,
                Some(serde_json::Value::String(enc)),
            )
            .await?;
        }
    }
    if let Some(ref v) = patch.from_address {
        upsert_global_raw(
            db,
            KEY_GLOBAL_SMTP_FROM_ADDRESS,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.from_name {
        upsert_global_raw(
            db,
            KEY_GLOBAL_SMTP_FROM_NAME,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.tls_mode {
        upsert_global_raw(
            db,
            KEY_GLOBAL_SMTP_TLS_MODE,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    if let Some(ref v) = patch.helo_host {
        upsert_global_raw(
            db,
            KEY_GLOBAL_SMTP_HELO_HOST,
            Some(serde_json::Value::String(v.clone())),
        )
        .await?;
    }
    Ok(())
}

async fn upsert_tenant_raw(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    key: &str,
    value: Option<serde_json::Value>,
) -> std::result::Result<(), SurfaceActionError> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_shared_db::raw_settings::upsert_setting_raw(db, tenant_id, key, payload)
        .await
        .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))
}

async fn upsert_global_raw(
    db: &sea_orm::DatabaseConnection,
    key: &str,
    value: Option<serde_json::Value>,
) -> std::result::Result<(), SurfaceActionError> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_shared_db::raw_settings::upsert_global_setting_raw(db, key, payload)
        .await
        .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))
}

async fn db_load_user_email(
    db: &sea_orm::DatabaseConnection,
    user_id: uuid::Uuid,
) -> std::result::Result<Option<String>, SurfaceActionError> {
    use sea_orm::EntityTrait as _;
    let model = uptrakit_shared_db::entity::prelude::User::find_by_id(user_id)
        .one(db)
        .await
        .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;
    Ok(model.map(|user| user.email.expose_email().to_string()))
}

#[cfg(test)]
mod tests {
    use super::{
        ListActionParams, SmtpPatchActionParams, normalize_tls_mode, parse_action_params,
        settings_map_to_snapshot,
    };

    #[test]
    fn settings_map_to_snapshot_normalizes_unknown_tls_mode_to_starttls() {
        let mut raw = std::collections::HashMap::new();
        raw.insert(
            "smtp.tls_mode".to_string(),
            serde_json::Value::String("legacy".to_string()),
        );
        let snapshot = settings_map_to_snapshot(
            &raw,
            "smtp.host",
            "smtp.port",
            "smtp.username",
            "smtp.password",
            "smtp.from_address",
            "smtp.from_name",
            "smtp.tls_mode",
            "smtp.helo_host",
            "unused_aad",
            "tenant",
            None,
        );
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

        assert_eq!(
            params.host.as_deref(),
            Some("smtp.example.com"),
            "host should be present"
        );
        assert_eq!(params.port, Some(465), "port should be parsed from string");
        assert_eq!(
            params.password.as_deref(),
            Some(""),
            "password field is present but empty"
        );
        assert_eq!(
            params.from_address.as_deref(),
            Some("alerts@example.com"),
            "from_address should be present"
        );
    }

    #[test]
    fn action_param_parsing_treats_non_object_payload_as_empty_object() {
        let list_params =
            parse_action_params::<ListActionParams>(&serde_json::json!("oops"), "list");
        assert_eq!(list_params.page(), 1);
        assert_eq!(list_params.per_page(), 50);

        // Non-object payload → defaults; empty password is not saved (checked in save path)
        let smtp_patch = parse_action_params::<SmtpPatchActionParams>(
            &serde_json::json!(["not", "an", "object"]),
            "configure_smtp",
        );
        assert!(smtp_patch.host.is_none());
        assert!(smtp_patch.port.is_none());
        assert!(smtp_patch.password.is_none());
    }
}
