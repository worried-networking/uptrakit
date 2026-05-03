//! Surface action handlers for the Telegram notification plugin.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uptrakit_plugin_infrastructure_core::{SurfaceActionContext, SurfaceActionError};
use uptrakit_shared_db::entity::notification_log;

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

const CALLBACK_ERR_INVALID_SECRET: &str = "Unauthorized: invalid_secret";
const CALLBACK_ERR_MISSING_ACTION_TOKEN: &str = "Bad request: missing_action_token";
const CALLBACK_ERR_INVALID_ACTION_TOKEN: &str = "Bad request: invalid_action_token";
const CALLBACK_ERR_NOTIFICATION_LOG_LOOKUP_FAILED: &str =
    "Internal server error: notification_log_lookup_failed";
const CALLBACK_ERR_NOTIFICATION_LOG_UPDATE_FAILED: &str =
    "Internal server error: notification_log_update_failed";

/// Handle a surface action for the Telegram notification plugin.
///
/// Supported actions:
/// - `list` -- list Telegram channels with masked secrets.
/// - `get_global_telegram` -- load global Telegram settings.
/// - `save_global_telegram` -- save global Telegram settings.
/// - `handle_callback` -- handle Telegram Bot API webhook callback.
#[tracing::instrument(skip_all, fields(surface_id, action_id))]
pub async fn handle_surface_action(
    ctx: &SurfaceActionContext<'_>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    match action_id {
        "list" => handle_list(ctx, &params).await,
        "get_global_telegram" => handle_get_global_telegram(ctx).await,
        "save_global_telegram" => handle_save_global_telegram(ctx, &params).await,
        "handle_callback" => handle_callback(ctx, &params).await,
        _ => Err(SurfaceActionError::InvalidInput(format!(
            "unknown action '{action_id}' for surface '{surface_id}'",
        ))),
    }
}

async fn handle_list(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    uptrakit_notification_plugin_core::list_channels::list_channels(
        ctx.tenant_db().db(),
        ctx.tenant_id(),
        "telegram",
        params,
        |_channel_type, config| config.clone(),
    )
    .await
    .map_err(SurfaceActionError::ControllerIntegration)
}

/// Load global Telegram settings and return `{ "has_bot_token": bool }`.
async fn handle_get_global_telegram(
    ctx: &SurfaceActionContext<'_>,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let bot_token = db_load_global_bot_token(ctx.tenant_db().db()).await?;
    Ok(serde_json::json!({
        "has_bot_token": !bot_token.is_empty(),
    }))
}

/// Save global Telegram settings (bot_token) and return `{ "has_bot_token": bool }`.
async fn handle_save_global_telegram(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let bot_token = params
        .get("bot_token")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();

    let saved = db_save_global_bot_token(ctx.tenant_db().db(), bot_token).await?;
    Ok(serde_json::json!({
        "has_bot_token": !saved.is_empty(),
    }))
}

/// Handle a Telegram callback from the Bot API webhook.
///
/// Verifies the secret token, extracts the action token from the callback
/// query data, and updates the notification log.
async fn handle_callback(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;

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
        return Err(SurfaceActionError::InvalidInput(
            CALLBACK_ERR_INVALID_SECRET.to_string(),
        ));
    }

    // Extract callback_query.data (action token UUID)
    let action_token_str = match body
        .get("callback_query")
        .and_then(|cq| cq.get("data"))
        .and_then(serde_json::Value::as_str)
    {
        Some(s) => s,
        None => {
            return Err(SurfaceActionError::InvalidInput(
                CALLBACK_ERR_MISSING_ACTION_TOKEN.to_string(),
            ));
        }
    };

    let action_token: Uuid = match action_token_str.parse() {
        Ok(id) => id,
        Err(_) => {
            tracing::warn!(
                action_token = %action_token_str,
                "invalid action token UUID in Telegram callback"
            );
            return Err(SurfaceActionError::InvalidInput(
                CALLBACK_ERR_INVALID_ACTION_TOKEN.to_string(),
            ));
        }
    };

    let tenant_id = ctx.tenant_id();

    // Look up notification log by action token, scoped to the current tenant.
    let model = notification_log::Entity::find()
        .filter(notification_log::Column::TenantId.eq(tenant_id))
        .filter(notification_log::Column::ActionToken.eq(action_token))
        .one(ctx.tenant_db().db())
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, %action_token, "failed to look up action token");
            SurfaceActionError::ControllerIntegration(
                CALLBACK_ERR_NOTIFICATION_LOG_LOOKUP_FAILED.to_string(),
            )
        })?;

    let Some(log_entry) = model else {
        tracing::warn!(%action_token, "no notification log found for action token");
        return Ok(serde_json::json!({}));
    };

    // If already actioned, return success
    if log_entry.action_taken.is_some() {
        return Ok(serde_json::json!({}));
    }

    // Update action_taken
    {
        use sea_orm::sea_query::Expr;

        notification_log::Entity::update_many()
            .col_expr(
                notification_log::Column::ActionTaken,
                Expr::value(Some("triggered".to_string())),
            )
            .filter(notification_log::Column::TenantId.eq(tenant_id))
            .filter(notification_log::Column::ActionToken.eq(action_token))
            .exec(ctx.tenant_db().db())
            .await
            .map_err(|error| {
                tracing::error!(error = ?error, %action_token, "failed to update action token state");
                SurfaceActionError::ControllerIntegration(
                    CALLBACK_ERR_NOTIFICATION_LOG_UPDATE_FAILED.to_string(),
                )
            })?;
    }

    Ok(serde_json::json!({}))
}

async fn db_load_global_bot_token(
    db: &sea_orm::DatabaseConnection,
) -> std::result::Result<String, SurfaceActionError> {
    let map =
        uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, "global_telegram.")
            .await
            .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;

    Ok(map
        .get(KEY_GLOBAL_TELEGRAM_BOT_TOKEN)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string())
}

async fn db_save_global_bot_token(
    db: &sea_orm::DatabaseConnection,
    bot_token: String,
) -> std::result::Result<String, SurfaceActionError> {
    uptrakit_shared_db::raw_settings::upsert_global_setting_raw(
        db,
        KEY_GLOBAL_TELEGRAM_BOT_TOKEN,
        serde_json::json!(bot_token),
    )
    .await
    .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;
    db_load_global_bot_token(db).await
}
