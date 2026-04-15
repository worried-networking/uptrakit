//! Surface action handlers for the Telegram notification plugin.

use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_core::SurfaceActionContext;

// ── Raw settings key constants ────────────────────────────────────────────────

/// Key prefix for global Telegram settings (used with `load_global_settings_by_prefix`).
pub const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";

/// Global Telegram bot token (stored in the `global_settings` table).
pub const KEY_GLOBAL_TELEGRAM_BOT_TOKEN: &str = "global_telegram.bot_token";

/// All raw settings keys written by the Telegram plugin to the `global_settings`
/// table via [`uptrakit_shared_db::raw_settings`].
///
/// Aggregated by [`uptrakit_plugin_infrastructure_registry::all_plugin_raw_settings_keys`]
/// so the controller can suppress false-positive "unrecognised setting key" startup warnings
/// for these legitimately plugin-owned entries.
pub const RAW_SETTINGS_KEYS: &[&str] = &[KEY_GLOBAL_TELEGRAM_BOT_TOKEN];
use uuid::Uuid;

/// Handle a surface action for the Telegram notification plugin.
///
/// Supported actions:
/// - `list` -- list Telegram channels with masked secrets.
/// - `get_global_telegram` -- load global Telegram settings.
/// - `save_global_telegram` -- save global Telegram settings.
/// - `handle_callback` -- handle Telegram Bot API webhook callback.
#[tracing::instrument(skip_all, fields(extension_id, action_id))]
pub async fn handle_action(
    ctx: &SurfaceActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let db = ctx
        .db
        .downcast_ref::<DatabaseConnection>()
        .ok_or_else(|| "expected DatabaseConnection".to_string())?;

    match action_id {
        "list" => {
            let tenant_id = ctx
                .tenant_id
                .ok_or_else(|| "tenant_id is required for listing channels".to_string())?;

            uptrakit_notification_plugin_core::list_channels::list_channels(
                db,
                tenant_id,
                "telegram",
                &params,
                |_channel_type, config| {
                    let mut masked = config.clone();
                    if let Some(obj) = masked.as_object_mut() {
                        if let Some(val) = obj.get("bot_token")
                            && val.as_str().is_some_and(|s| !s.is_empty())
                        {
                            obj.insert("bot_token".to_string(), serde_json::json!("***"));
                        }
                        if let Some(val) = obj.get("webhook_secret")
                            && val.as_str().is_some_and(|s| !s.is_empty())
                        {
                            obj.insert("webhook_secret".to_string(), serde_json::json!("***"));
                        }
                    }
                    masked
                },
            )
            .await
        }
        "get_global_telegram" => handle_get_global_telegram(db).await,
        "save_global_telegram" => handle_save_global_telegram(db, &params).await,
        "handle_callback" => handle_callback(db, &params).await,
        _ => Err(format!(
            "unknown action '{action_id}' for extension '{extension_id}'"
        )),
    }
}

/// Load global Telegram settings and return `{ "has_bot_token": bool }`.
async fn handle_get_global_telegram(
    db: &sea_orm::DatabaseConnection,
) -> std::result::Result<serde_json::Value, String> {
    let settings = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
        db,
        GLOBAL_TELEGRAM_PREFIX,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to load global Telegram settings: {e:?}");
        "Internal server error".to_string()
    })?;

    let has_bot_token = settings
        .get(KEY_GLOBAL_TELEGRAM_BOT_TOKEN)
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());

    Ok(serde_json::json!({ "has_bot_token": has_bot_token }))
}

/// Save global Telegram settings (bot_token) and return `{ "has_bot_token": bool }`.
async fn handle_save_global_telegram(
    db: &sea_orm::DatabaseConnection,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let bot_token = params
        .get("bot_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    uptrakit_shared_db::raw_settings::upsert_global_setting_raw(
        db,
        KEY_GLOBAL_TELEGRAM_BOT_TOKEN,
        serde_json::json!(bot_token),
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to save global Telegram bot_token: {e:?}");
        "Internal server error".to_string()
    })?;

    let has_bot_token = !bot_token.is_empty();

    Ok(serde_json::json!({ "has_bot_token": has_bot_token }))
}

/// Handle a Telegram callback from the Bot API webhook.
///
/// Verifies the secret token, extracts the action token from the callback
/// query data, and updates the notification log.
async fn handle_callback(
    db: &DatabaseConnection,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;
    use uptrakit_shared_db::entity::notification_log;

    let config = params
        .get("channel_config")
        .unwrap_or(&serde_json::Value::Null);
    let headers = params.get("headers").unwrap_or(&serde_json::Value::Null);
    let body = params.get("body").unwrap_or(&serde_json::Value::Null);

    // Verify secret token using constant-time comparison to prevent timing attacks.
    // Both secrets are hashed to SHA-256 so ct_eq always compares equal-length arrays,
    // eliminating any length-based information leak.
    let expected_secret = config
        .get("webhook_secret")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    let provided_secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    let expected_hash: [u8; 32] = Sha256::digest(expected_secret.as_bytes()).into();
    let provided_hash: [u8; 32] = Sha256::digest(provided_secret.as_bytes()).into();
    let secrets_match: bool = expected_hash.ct_eq(&provided_hash).into();

    if expected_secret.is_empty() || !secrets_match {
        return Err("Unauthorized: Invalid secret token".to_string());
    }

    // Extract callback_query.data (action token UUID)
    let action_token_str = match body
        .get("callback_query")
        .and_then(|cq| cq.get("data"))
        .and_then(serde_json::Value::as_str)
    {
        Some(s) => s,
        None => {
            // Not a callback query we care about — acknowledge silently
            return Ok(serde_json::json!({}));
        }
    };

    let action_token: Uuid = match action_token_str.parse() {
        Ok(id) => id,
        Err(_) => {
            tracing::warn!(
                action_token = %action_token_str,
                "invalid action token UUID in Telegram callback"
            );
            return Ok(serde_json::json!({}));
        }
    };

    // Look up notification log by action token
    let log_entry = notification_log::Entity::find()
        .filter(notification_log::Column::ActionToken.eq(action_token))
        .one(db)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, %action_token, "failed to look up action token");
            "Internal server error".to_string()
        })?;

    let Some(log_entry) = log_entry else {
        tracing::warn!(%action_token, "no notification log found for action token");
        return Ok(serde_json::json!({}));
    };

    // If already actioned, return success
    if log_entry.action_taken.is_some() {
        return Ok(serde_json::json!({}));
    }

    // Update action_taken
    let mut active: notification_log::ActiveModel = log_entry.into();
    active.action_taken = Set(Some("triggered".to_string()));

    active.update(db).await.map_err(|e| {
        tracing::error!(error = ?e, "failed to update notification log action_taken");
        "Internal server error".to_string()
    })?;

    Ok(serde_json::json!({}))
}
