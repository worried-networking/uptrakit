//! Extension action handlers for the email notification plugin.
//!
//! Handles SMTP settings management (per-tenant and global) and channel
//! listing. Replaces the SMTP settings logic that was previously in
//! `web-api/src/routes/notification_extensions.rs`.

use std::collections::HashMap;

use sea_orm::EntityTrait;
use uptrakit_crypto::{decrypt_str, encrypt_str, is_encrypted};
use uptrakit_notification_plugin_core::DeliveryMessage;
use uptrakit_plugin_infrastructure_core::{ExtensionActionContext, PluginBase as _};
use uptrakit_shared_db::entity::prelude::User;
use uptrakit_web_api_auth::settings_store::{
    load_global_settings_by_prefix, load_settings_by_prefix, upsert_global_setting_raw,
    upsert_setting_raw,
};

use crate::{EmailPlugin, SmtpSettingsSnapshot, merge_smtp_into_config};

/// Password AAD for per-tenant SMTP password encryption.
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
/// Password AAD for global SMTP password encryption.
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

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

    let tenant_map = load_settings_by_prefix(ctx.db, tenant_id, "smtp.")
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to load tenant SMTP settings");
            "Internal server error".to_string()
        })?;

    let global_map = load_global_settings_by_prefix(ctx.db, "global_smtp.")
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
        upsert_setting_raw(ctx.db, tenant_id, "smtp.host", serde_json::json!(host))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to save smtp.host");
                "Internal server error".to_string()
            })?;
    }

    if let Some(port) = params.get("port").and_then(|v| {
        v.as_u64()
            .map(|n| n as u16)
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }) {
        upsert_setting_raw(ctx.db, tenant_id, "smtp.port", serde_json::json!(port))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to save smtp.port");
                "Internal server error".to_string()
            })?;
    }

    if let Some(username) = params.get("username").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            "smtp.username",
            serde_json::json!(username),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save smtp.username");
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
            "smtp.password",
            serde_json::json!(encrypted),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save smtp.password");
            "Internal server error".to_string()
        })?;
    }

    if let Some(from_address) = params.get("from_address").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            "smtp.from_address",
            serde_json::json!(from_address),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save smtp.from_address");
            "Internal server error".to_string()
        })?;
    }

    if let Some(from_name) = params.get("from_name").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            "smtp.from_name",
            serde_json::json!(from_name),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save smtp.from_name");
            "Internal server error".to_string()
        })?;
    }

    if let Some(tls_mode) = params.get("tls_mode").and_then(|v| v.as_str()) {
        upsert_setting_raw(
            ctx.db,
            tenant_id,
            "smtp.tls_mode",
            serde_json::json!(tls_mode),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save smtp.tls_mode");
            "Internal server error".to_string()
        })?;
    }

    // Re-read saved settings to return the current state.
    let tenant_map = load_settings_by_prefix(ctx.db, tenant_id, "smtp.")
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
    let global_map = load_global_settings_by_prefix(ctx.db, "global_smtp.")
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
        upsert_global_setting_raw(ctx.db, "global_smtp.host", serde_json::json!(host))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to save global_smtp.host");
                "Internal server error".to_string()
            })?;
    }

    if let Some(port) = params.get("port").and_then(|v| {
        v.as_u64()
            .map(|n| n as u16)
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }) {
        upsert_global_setting_raw(ctx.db, "global_smtp.port", serde_json::json!(port))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to save global_smtp.port");
                "Internal server error".to_string()
            })?;
    }

    if let Some(username) = params.get("username").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(ctx.db, "global_smtp.username", serde_json::json!(username))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to save global_smtp.username");
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
        upsert_global_setting_raw(ctx.db, "global_smtp.password", serde_json::json!(encrypted))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to save global_smtp.password");
                "Internal server error".to_string()
            })?;
    }

    if let Some(from_address) = params.get("from_address").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            "global_smtp.from_address",
            serde_json::json!(from_address),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save global_smtp.from_address");
            "Internal server error".to_string()
        })?;
    }

    if let Some(from_name) = params.get("from_name").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            "global_smtp.from_name",
            serde_json::json!(from_name),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save global_smtp.from_name");
            "Internal server error".to_string()
        })?;
    }

    if let Some(tls_mode) = params.get("tls_mode").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(ctx.db, "global_smtp.tls_mode", serde_json::json!(tls_mode))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to save global_smtp.tls_mode");
                "Internal server error".to_string()
            })?;
    }

    if let Some(helo_host) = params.get("helo_host").and_then(|v| v.as_str()) {
        upsert_global_setting_raw(
            ctx.db,
            "global_smtp.helo_host",
            serde_json::json!(helo_host),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to save global_smtp.helo_host");
            "Internal server error".to_string()
        })?;
    }

    // Re-read saved settings to return the current state.
    let global_map = load_global_settings_by_prefix(ctx.db, "global_smtp.")
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
    let global_map = load_global_settings_by_prefix(ctx.db, "global_smtp.")
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

    let plugin = EmailPlugin;
    let transport = plugin
        .as_notification_transport()
        .ok_or_else(|| "Email plugin does not support delivery".to_string())?;

    let test_msg = DeliveryMessage::new(
        "Test Email from Uptrakit",
        "This is a test email sent from the Global SMTP settings page.",
        None,
        serde_json::json!({}),
        vec![],
    );

    // The config is already merged with SMTP settings above, so pass an
    // empty settings bag — deliver() will see smtp_host in the config and
    // skip re-merging.
    let empty_settings = serde_json::json!({});
    transport
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
        host: get_string(map, "smtp.host"),
        port: get_port(map, "smtp.port"),
        username: get_string(map, "smtp.username"),
        password: get_decrypted_password(map, "smtp.password", SMTP_PASSWORD_AAD),
        from_address: get_string(map, "smtp.from_address"),
        from_name: get_string(map, "smtp.from_name"),
        tls_mode: get_tls_mode(map, "smtp.tls_mode"),
        helo_host: None, // helo_host is global-only
    }
}

/// Build an `SmtpSettingsSnapshot` from global settings loaded with the
/// `global_smtp.` prefix.
fn smtp_from_global_map(map: &HashMap<String, serde_json::Value>) -> SmtpSettingsSnapshot {
    SmtpSettingsSnapshot {
        host: get_string(map, "global_smtp.host"),
        port: get_port(map, "global_smtp.port"),
        username: get_string(map, "global_smtp.username"),
        password: get_decrypted_password(map, "global_smtp.password", GLOBAL_SMTP_PASSWORD_AAD),
        from_address: get_string(map, "global_smtp.from_address"),
        from_name: get_string(map, "global_smtp.from_name"),
        tls_mode: get_tls_mode(map, "global_smtp.tls_mode"),
        helo_host: get_string(map, "global_smtp.helo_host"),
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
