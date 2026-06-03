//! Surface action handlers for the email notification plugin.
//!
//! Handles SMTP settings management (per-tenant and global) and channel
//! listing. SMTP `password` is excluded from the serde-driven
//! `SmtpNonSecretSnapshot` decode and is decrypted separately via
//! `uptrakit_crypto::decrypt_str` because it is stored as `SecretString`
//! ciphertext at rest.

use serde::Deserialize;
use uptrakit_notification_plugin_core::DeliveryMessage;
use uptrakit_plugin_infrastructure_core::{
    NotificationTransport as _, SurfaceActionContext, SurfaceActionError,
};
use uptrakit_shared_types::SecretString;

use crate::{EmailPlugin, SmtpSettingsSnapshot, merge_smtp_into_config};

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
    uptrakit_notification_plugin_core::list_channels::list_channels(
        ctx.tenant_db().db(),
        ctx.tenant_id(),
        "email",
        params,
        |_channel_type, config| {
            // email has no secrets in channel config (to_addresses are plaintext),
            // return config as-is
            config.clone()
        },
    )
    .await
    .map_err(SurfaceActionError::ControllerIntegration)
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

// ── Serde helpers for SmtpNonSecretSnapshot ───────────────────────────────────

/// Filters empty strings to `None`, mirroring the legacy `get_str` helper.
fn deserialize_non_empty_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    Ok(opt.filter(|s| !s.is_empty()))
}

#[derive(Debug, Default, serde::Deserialize)]
struct SmtpNonSecretSnapshot {
    #[serde(default, deserialize_with = "deserialize_non_empty_string")]
    host: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_optional_port")]
    port: Option<u16>,
    #[serde(default, deserialize_with = "deserialize_non_empty_string")]
    username: Option<String>,
    #[serde(default, deserialize_with = "deserialize_non_empty_string")]
    from_address: Option<String>,
    #[serde(default, deserialize_with = "deserialize_non_empty_string")]
    from_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_non_empty_string")]
    tls_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_non_empty_string")]
    helo_host: Option<String>,
}

/// Two-phase SMTP snapshot decoder.
///
/// Phase 1: decode all non-secret SMTP fields via
/// [`uptrakit_shared_db::raw_settings::decode_prefixed_settings`] into
/// [`SmtpNonSecretSnapshot`].
///
/// Phase 2: decrypt the password field directly from the raw map (it is
/// stored as an `uptrakit_crypto`-encrypted ciphertext).
fn smtp_snapshot_from_raw(
    raw: &std::collections::HashMap<String, serde_json::Value>,
    prefix: &str,
    password_key: &str,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<uuid::Uuid>,
) -> SmtpSettingsSnapshot {
    let non_secret: SmtpNonSecretSnapshot =
        match uptrakit_shared_db::raw_settings::decode_prefixed_settings(prefix, raw) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    scope,
                    prefix,
                    "smtp non-secret settings failed typed decode; falling back to defaults",
                );
                SmtpNonSecretSnapshot::default()
            }
        };

    let password = raw
        .get(password_key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .and_then(|raw_str| {
            if uptrakit_crypto::is_encrypted(raw_str) {
                match uptrakit_crypto::decrypt_str(raw_str, password_aad) {
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
                Some(raw_str.to_string())
            }
        })
        .map(SecretString::new);

    SmtpSettingsSnapshot {
        host: non_secret.host,
        port: non_secret.port,
        username: non_secret.username,
        password,
        from_address: non_secret.from_address,
        from_name: non_secret.from_name,
        tls_mode: normalize_tls_mode(non_secret.tls_mode),
        helo_host: non_secret.helo_host,
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
    Ok(smtp_snapshot_from_raw(
        &raw,
        SMTP_PREFIX,
        KEY_SMTP_PASSWORD,
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
    Ok(smtp_snapshot_from_raw(
        &raw,
        GLOBAL_SMTP_PREFIX,
        KEY_GLOBAL_SMTP_PASSWORD,
        GLOBAL_SMTP_PASSWORD_AAD,
        "global",
        None,
    ))
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
    use super::{SmtpPatchActionParams, normalize_tls_mode, parse_action_params};

    fn make_raw(
        pairs: &[(&str, serde_json::Value)],
    ) -> std::collections::HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn smtp_snapshot_from_raw_collapses_empty_string_to_none() {
        let raw = make_raw(&[("smtp.host", serde_json::json!(""))]);
        let snapshot =
            super::smtp_snapshot_from_raw(&raw, "smtp.", "smtp.password", "aad", "tenant", None);
        assert!(
            snapshot.host.is_none(),
            "empty string host should become None"
        );
    }

    #[test]
    fn smtp_snapshot_from_raw_accepts_port_as_json_string() {
        let raw = make_raw(&[("smtp.port", serde_json::json!("587"))]);
        let snapshot =
            super::smtp_snapshot_from_raw(&raw, "smtp.", "smtp.password", "aad", "tenant", None);
        assert_eq!(
            snapshot.port,
            Some(587),
            "port stored as JSON string should parse to 587"
        );
    }

    #[test]
    fn smtp_snapshot_from_raw_normalizes_unknown_tls_mode_to_starttls() {
        let raw = make_raw(&[("smtp.tls_mode", serde_json::json!("legacy_mode"))]);
        let snapshot =
            super::smtp_snapshot_from_raw(&raw, "smtp.", "smtp.password", "aad", "tenant", None);
        assert_eq!(
            snapshot.tls_mode, "starttls",
            "unknown tls_mode should normalize to starttls"
        );
    }

    #[test]
    fn smtp_snapshot_from_raw_decrypts_encrypted_password() {
        uptrakit_crypto::enable_plaintext_mode();
        let plaintext_password = "s3cr3t";
        let aad = super::SMTP_PASSWORD_AAD;
        let encrypted_password =
            uptrakit_crypto::encrypt_str(plaintext_password, aad).expect("encrypt");
        let raw = make_raw(&[("smtp.password", serde_json::json!(encrypted_password))]);
        let snapshot =
            super::smtp_snapshot_from_raw(&raw, "smtp.", "smtp.password", aad, "tenant", None);
        let password = snapshot.password.expect("password should be present");
        assert_eq!(
            password.expose_secret(),
            plaintext_password,
            "encrypted password should decrypt back to plaintext"
        );
    }

    #[test]
    fn smtp_snapshot_from_raw_passes_through_all_string_fields() {
        let raw = make_raw(&[
            ("smtp.host", serde_json::json!("smtp.example.com")),
            ("smtp.username", serde_json::json!("user@example.com")),
            ("smtp.from_address", serde_json::json!("alerts@example.com")),
            ("smtp.from_name", serde_json::json!("Uptrakit Alerts")),
            ("smtp.helo_host", serde_json::json!("relay.example.com")),
            ("smtp.tls_mode", serde_json::json!("tls")),
        ]);
        let snapshot =
            super::smtp_snapshot_from_raw(&raw, "smtp.", "smtp.password", "aad", "tenant", None);
        assert_eq!(snapshot.host.as_deref(), Some("smtp.example.com"));
        assert_eq!(snapshot.username.as_deref(), Some("user@example.com"));
        assert_eq!(snapshot.from_address.as_deref(), Some("alerts@example.com"));
        assert_eq!(snapshot.from_name.as_deref(), Some("Uptrakit Alerts"));
        assert_eq!(snapshot.helo_host.as_deref(), Some("relay.example.com"));
        assert_eq!(snapshot.tls_mode, "tls");
    }

    #[test]
    fn normalize_tls_mode_preserves_supported_values() {
        assert_eq!(normalize_tls_mode(Some("starttls".to_string())), "starttls");
        assert_eq!(normalize_tls_mode(Some("tls".to_string())), "tls");
        assert_eq!(normalize_tls_mode(Some("none".to_string())), "none");
        assert_eq!(normalize_tls_mode(None), "starttls");
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
