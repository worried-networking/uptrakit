//! Extension action handlers for the Telegram notification plugin.

use uptrakit_plugin_infrastructure_core::ExtensionActionContext;

/// Handle an extension action for the Telegram notification plugin.
///
/// Supported actions:
/// - `list` -- list Telegram channels with masked secrets.
/// - `get_global_telegram` -- load global Telegram settings.
/// - `save_global_telegram` -- save global Telegram settings.
#[tracing::instrument(skip_all, fields(extension_id, action_id))]
pub async fn handle_action(
    ctx: &ExtensionActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    match action_id {
        "list" => {
            let tenant_id = ctx
                .tenant_id
                .ok_or_else(|| "tenant_id is required for listing channels".to_string())?;

            uptrakit_notification_plugin_core::list_channels::list_channels(
                ctx.db,
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
        "get_global_telegram" => handle_get_global_telegram(ctx.db).await,
        "save_global_telegram" => handle_save_global_telegram(ctx.db, &params).await,
        _ => Err(format!(
            "unknown action '{action_id}' for extension '{extension_id}'"
        )),
    }
}

/// Load global Telegram settings and return `{ "has_bot_token": bool }`.
async fn handle_get_global_telegram(
    db: &sea_orm::DatabaseConnection,
) -> std::result::Result<serde_json::Value, String> {
    let settings = uptrakit_web_api_auth::settings_store::load_global_settings_by_prefix(
        db,
        "global_telegram.",
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to load global Telegram settings: {e:?}");
        "Internal server error".to_string()
    })?;

    let has_bot_token = settings
        .get("global_telegram.bot_token")
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

    uptrakit_web_api_auth::settings_store::upsert_global_setting_raw(
        db,
        "global_telegram.bot_token",
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
