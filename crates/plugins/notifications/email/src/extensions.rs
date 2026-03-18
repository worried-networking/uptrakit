//! Extension action handlers for the email notification plugin.
//!
//! Handles SMTP settings management (per-tenant and global) and channel
//! listing. Replaces the SMTP settings logic that was previously in
//! `web-api/src/routes/notification_extensions.rs`.

use std::collections::HashMap;

use sea_orm::EntityTrait;
use uptrakit_crypto::{decrypt_str, encrypt_str, is_encrypted};
use uptrakit_notification_plugin_core::DeliveryMessage;
use uptrakit_plugin_infrastructure_core::ExtensionActionContext;
use uptrakit_shared_db::entity::prelude::User;
use uptrakit_shared_db::raw_settings::{
    load_global_settings_by_prefix, load_settings_by_prefix, upsert_global_setting_raw,
    upsert_setting_raw,
};

use uptrakit_shared_types::SecretString;

use crate::{EmailPlugin, SmtpSettingsSnapshot, merge_smtp_into_config};

/// Password AAD for per-tenant SMTP password encryption.
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
/// Password AAD for global SMTP password encryption.
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

// ── Raw settings key constants ────────────────────────────────────────────────

/// Key prefix for per-tenant SMTP settings (used with `load_settings_by_prefix`).
pub const SMTP_PREFIX: &str = "smtp.";
/// Key prefix for global SMTP settings (used with `load_global_settings_by_prefix`).
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
/// `global_settings` tables via [`uptrakit_shared_db::raw_settings`].
///
/// Aggregated by [`uptrakit_plugin_infrastructure_registry::all_plugin_raw_settings_keys`]
/// so the controller can suppress false-positive "unrecognised setting key" startup warnings
/// for these legitimately plugin-owned entries.
pub const RAW_SETTINGS_KEYS: &[&str] = &[
    // Per-tenant SMTP settings (stored in `settings` table)
    KEY_SMTP_HOST,
    KEY_SMTP_PORT,
    KEY_SMTP_USERNAME,
    KEY_SMTP_PASSWORD,
    KEY_SMTP_FROM_ADDRESS,
    KEY_SMTP_FROM_NAME,
    KEY_SMTP_TLS_MODE,
    // Global SMTP defaults (stored in `global_settings` table)
    KEY_GLOBAL_SMTP_HOST,
    KEY_GLOBAL_SMTP_PORT,
    KEY_GLOBAL_SMTP_USERNAME,
    KEY_GLOBAL_SMTP_PASSWORD,
    KEY_GLOBAL_SMTP_FROM_ADDRESS,
    KEY_GLOBAL_SMTP_FROM_NAME,
    KEY_GLOBAL_SMTP_TLS_MODE,
    KEY_GLOBAL_SMTP_HELO_HOST,
];

/// Handle an extension action for the email notification plugin.
#[tracing::instrument(skip_all, fields(extension_id, action_id))]
pub async fn handle_action(
    ctx: &ExtensionActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    match action_id {
        "list" => handle_list(ctx, &params).await,
        "get_smtp" => handle_get_smtp(ctx).await,
        "save_smtp" => handle_save_smtp(ctx, &params).await,
        "get_global_smtp" => handle_get_global_smtp(ctx).await,
        "save_global_smtp" => handle_save_global_smtp(ctx, &params).await,
        "test_global_smtp_email" => handle_test_global_smtp_email(ctx).await,
        _ => Err(format!(
            "unknown action '{action_id}' for extension '{extension_id}'"
        )),
    }
}

// ── List channels ────────────────────────────────────────────────────────────

async fn handle_list(
    ctx: &ExtensionActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| "tenant_id is required for listing channels".to_string())?;

    // Email per-channel config has no secrets, return config unchanged.
    uptrakit_notification_plugin_core::list_channels::list_channels(
        ctx.db,
        tenant_id,
        "email",
        params,
        |_channel_type, config| config.clone(),
    )
    .await
}

// ── Per-tenant SMTP settings ─────────────────────────────────────────────────

async fn handle_get_smtp(
    ctx: &ExtensionActionContext<'_>,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| "tenant_id is required for get_smtp".to_string())?;

    let tenant_map = load_settings_by_prefix(ctx.db, tenant_id, SMTP_PREFIX)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to load tenant SMTP settings");
            "Internal server error".to_string()
        })?;

    let global_map = load_global_settings_by_prefix(ctx.db, GLOBAL_SMTP_PREFIX)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to load global SMTP settings");
            "Internal server error".to_string()
        })?;

    let smtp = smtp_from_tenant_map(&tenant_map);
    let global = smtp_from_global_map(&global_map);

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

async fn handle_save_smtp(
    ctx: &ExtensionActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| "tenant_id is required for save_smtp".to_string())?;

    if let Some(host) = params.get("host").and_then(|v| v.as_str()) {
        upsert_setting_raw(ctx.db, tenant_id, KEY_SMTP_HOST, serde_json::json!(host))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, key = KEY_SMTP_HOST, "failed to save setting");
                "Internal server error".to_string()
            })?;
    }

    if let Some(port) = params.get("port").and_then(|v| {
        v.as_u64()
            .map(|n| n as u16)
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }) {
        upsert_setting_raw(ctx.db, tenant_id, KEY_SMTP_PORT, serde_json::json!(port))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, key = KEY_SMTP_PORT, "failed to save setting");
                "Internal server error".to_string()
            })?;
    }

    if let Some(username) = params.get("username").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            KEY_SMTP_USERNAME,
            serde_json::json!(username),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_SMTP_USERNAME, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(password) = params
        .get("password")
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty())
    {
        let encrypted = encrypt_str(password, SMTP_PASSWORD_AAD).map_err(|e| {
            tracing::error!(error = ?e, "failed to encrypt SMTP password");
            "Internal server error".to_string()
        })?;
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            KEY_SMTP_PASSWORD,
            serde_json::json!(encrypted),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_SMTP_PASSWORD, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(from_address) = params.get("from_address").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            KEY_SMTP_FROM_ADDRESS,
            serde_json::json!(from_address),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_SMTP_FROM_ADDRESS, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(from_name) = params.get("from_name").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            KEY_SMTP_FROM_NAME,
            serde_json::json!(from_name),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_SMTP_FROM_NAME, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(tls_mode) = params.get("tls_mode").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            KEY_SMTP_TLS_MODE,
            serde_json::json!(tls_mode),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_SMTP_TLS_MODE, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    // Re-read saved settings to return the current state.
    let tenant_map = load_settings_by_prefix(ctx.db, tenant_id, SMTP_PREFIX)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to reload tenant SMTP settings");
            "Internal server error".to_string()
        })?;
    let smtp = smtp_from_tenant_map(&tenant_map);

    Ok(serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
    }))
}

// ── Global SMTP settings ─────────────────────────────────────────────────────

async fn handle_get_global_smtp(
    ctx: &ExtensionActionContext<'_>,
) -> std::result::Result<serde_json::Value, String> {
    let global_map = load_global_settings_by_prefix(ctx.db, GLOBAL_SMTP_PREFIX)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to load global SMTP settings");
            "Internal server error".to_string()
        })?;

    let smtp = smtp_from_global_map(&global_map);

    Ok(serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "helo_host": smtp.helo_host.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
    }))
}

async fn handle_save_global_smtp(
    ctx: &ExtensionActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    if let Some(host) = params.get("host").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(ctx.db, KEY_GLOBAL_SMTP_HOST, serde_json::json!(host))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_HOST, "failed to save setting");
                "Internal server error".to_string()
            })?;
    }

    if let Some(port) = params.get("port").and_then(|v| {
        v.as_u64()
            .map(|n| n as u16)
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }) {
        upsert_global_setting_raw(ctx.db, KEY_GLOBAL_SMTP_PORT, serde_json::json!(port))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_PORT, "failed to save setting");
                "Internal server error".to_string()
            })?;
    }

    if let Some(username) = params.get("username").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            KEY_GLOBAL_SMTP_USERNAME,
            serde_json::json!(username),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_USERNAME, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(password) = params
        .get("password")
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty())
    {
        let encrypted = encrypt_str(password, GLOBAL_SMTP_PASSWORD_AAD).map_err(|e| {
            tracing::error!(error = ?e, "failed to encrypt global SMTP password");
            "Internal server error".to_string()
        })?;
        upsert_global_setting_raw(
            ctx.db,
            KEY_GLOBAL_SMTP_PASSWORD,
            serde_json::json!(encrypted),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_PASSWORD, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(from_address) = params.get("from_address").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            KEY_GLOBAL_SMTP_FROM_ADDRESS,
            serde_json::json!(from_address),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_FROM_ADDRESS, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(from_name) = params.get("from_name").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            KEY_GLOBAL_SMTP_FROM_NAME,
            serde_json::json!(from_name),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_FROM_NAME, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(tls_mode) = params.get("tls_mode").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            KEY_GLOBAL_SMTP_TLS_MODE,
            serde_json::json!(tls_mode),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_TLS_MODE, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    if let Some(helo_host) = params.get("helo_host").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            KEY_GLOBAL_SMTP_HELO_HOST,
            serde_json::json!(helo_host),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, key = KEY_GLOBAL_SMTP_HELO_HOST, "failed to save setting");
            "Internal server error".to_string()
        })?;
    }

    // Re-read saved settings to return the current state.
    let global_map = load_global_settings_by_prefix(ctx.db, GLOBAL_SMTP_PREFIX)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to reload global SMTP settings");
            "Internal server error".to_string()
        })?;
    let smtp = smtp_from_global_map(&global_map);

    Ok(serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "helo_host": smtp.helo_host.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
    }))
}

// ── Test global SMTP email ───────────────────────────────────────────────────

async fn handle_test_global_smtp_email(
    ctx: &ExtensionActionContext<'_>,
) -> std::result::Result<serde_json::Value, String> {
    let caller_user_id = ctx
        .caller_user_id
        .ok_or_else(|| "caller_user_id is required for test_global_smtp_email".to_string())?;

    // Load caller's email address from the database.
    let user = User::find_by_id(caller_user_id)
        .one(ctx.db)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to load user for test email");
            "Internal server error".to_string()
        })?
        .ok_or_else(|| "User not found".to_string())?;

    let to_address = user.email.expose_email().to_string();

    // Load global SMTP settings from the database.
    let global_map = load_global_settings_by_prefix(ctx.db, GLOBAL_SMTP_PREFIX)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to load global SMTP settings for test email");
            "Internal server error".to_string()
        })?;
    let global_smtp = smtp_from_global_map(&global_map);

    if !global_smtp.is_configured() {
        return Err(
            "Global SMTP is not configured. Set SMTP host and from address before sending a test email."
                .to_string(),
        );
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

    tracing::debug!(
        smtp_host = global_smtp.host.as_deref().unwrap_or("<none>"),
        smtp_port = global_smtp.port.unwrap_or(587),
        tls_mode = %global_smtp.tls_mode,
        from_address = global_smtp.from_address.as_deref().unwrap_or("<none>"),
        from_name = global_smtp.from_name.as_deref().unwrap_or("<none>"),
        helo_host = global_smtp.helo_host.as_deref().unwrap_or("<auto>"),
        has_password = global_smtp.password.is_some(),
        to_address,
        "sending test email with global SMTP settings"
    );

    use uptrakit_plugin_infrastructure_core::NotificationTransport as _;

    let plugin = EmailPlugin;

    let test_msg = DeliveryMessage::new(
        "Test Email from Uptrakit",
        "This is a test email sent from the Global SMTP settings page.",
        None,
        serde_json::json!({}),
        vec![],
    );

    // The config is already merged with SMTP settings above, so pass an
    // empty settings bag -- deliver() will see smtp_host in the config and
    // skip re-merging.
    let empty_settings = serde_json::json!({});
    plugin
        .deliver(&config, &empty_settings, &test_msg)
        .await
        .map_err(|e| {
            tracing::warn!(error = ?e, to_address, "test global smtp email failed");
            e.to_string()
        })?;

    Ok(serde_json::json!({
        "success": true,
        "message": format!("Test email sent successfully to {to_address}")
    }))
}

// ── Helper functions ─────────────────────────────────────────────────────────

/// Build an `SmtpSettingsSnapshot` from per-tenant settings loaded with the
/// `smtp.` prefix.
fn smtp_from_tenant_map(map: &HashMap<String, serde_json::Value>) -> SmtpSettingsSnapshot {
    SmtpSettingsSnapshot {
        host: get_string(map, KEY_SMTP_HOST),
        port: get_port(map, KEY_SMTP_PORT),
        username: get_string(map, KEY_SMTP_USERNAME),
        password: get_decrypted_password(map, KEY_SMTP_PASSWORD, SMTP_PASSWORD_AAD)
            .map(SecretString::new),
        from_address: get_string(map, KEY_SMTP_FROM_ADDRESS),
        from_name: get_string(map, KEY_SMTP_FROM_NAME),
        tls_mode: get_tls_mode(map, KEY_SMTP_TLS_MODE),
        helo_host: None, // helo_host is global-only
    }
}

/// Build an `SmtpSettingsSnapshot` from global settings loaded with the
/// `global_smtp.` prefix.
fn smtp_from_global_map(map: &HashMap<String, serde_json::Value>) -> SmtpSettingsSnapshot {
    SmtpSettingsSnapshot {
        host: get_string(map, KEY_GLOBAL_SMTP_HOST),
        port: get_port(map, KEY_GLOBAL_SMTP_PORT),
        username: get_string(map, KEY_GLOBAL_SMTP_USERNAME),
        password: get_decrypted_password(map, KEY_GLOBAL_SMTP_PASSWORD, GLOBAL_SMTP_PASSWORD_AAD)
            .map(SecretString::new),
        from_address: get_string(map, KEY_GLOBAL_SMTP_FROM_ADDRESS),
        from_name: get_string(map, KEY_GLOBAL_SMTP_FROM_NAME),
        tls_mode: get_tls_mode(map, KEY_GLOBAL_SMTP_TLS_MODE),
        helo_host: get_string(map, KEY_GLOBAL_SMTP_HELO_HOST),
    }
}

/// Extract a non-empty string value from a settings map.
fn get_string(map: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Extract a port number from a settings map.
fn get_port(map: &HashMap<String, serde_json::Value>, key: &str) -> Option<u16> {
    map.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|n| u16::try_from(n).ok())
}

/// Extract and decrypt a password value from a settings map.
///
/// Returns `None` if the key is missing, the value is empty, or decryption fails.
fn get_decrypted_password(
    map: &HashMap<String, serde_json::Value>,
    key: &str,
    aad: &str,
) -> Option<String> {
    let raw = map
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;

    if is_encrypted(raw) {
        match decrypt_str(raw, aad) {
            Ok(decrypted) if !decrypted.is_empty() => Some(decrypted),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(key, error = ?e, "failed to decrypt SMTP password");
                None
            }
        }
    } else {
        // Unencrypted legacy value.
        Some(raw.to_string())
    }
}

/// Extract and validate a TLS mode from a settings map.
///
/// Returns one of `"starttls"`, `"tls"`, or `"none"`, defaulting to `"starttls"`.
fn get_tls_mode(map: &HashMap<String, serde_json::Value>, key: &str) -> String {
    map.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| matches!(*s, "starttls" | "tls" | "none"))
        .unwrap_or("starttls")
        .to_string()
}
