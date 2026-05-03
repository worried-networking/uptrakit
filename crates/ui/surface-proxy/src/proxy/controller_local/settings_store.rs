use async_trait::async_trait;

use super::{AppStateSurfaceActionController, plugin_internal_error};
use uptrakit_plugin_infrastructure_registry::TelegramGlobalSettingsStore;

const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const KEY_GLOBAL_TELEGRAM_BOT_TOKEN: &str = "global_telegram.bot_token";

#[async_trait]
impl TelegramGlobalSettingsStore for AppStateSurfaceActionController<'_> {
    async fn load_global_bot_token(
        &self,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<String> {
        let map = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
            self.db(),
            GLOBAL_TELEGRAM_PREFIX,
        )
        .await
        .map_err(plugin_internal_error)?;

        Ok(map
            .get(KEY_GLOBAL_TELEGRAM_BOT_TOKEN)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    async fn save_global_bot_token(
        &self,
        bot_token: String,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<String> {
        uptrakit_shared_db::raw_settings::upsert_global_setting_raw(
            self.db(),
            KEY_GLOBAL_TELEGRAM_BOT_TOKEN,
            serde_json::json!(bot_token),
        )
        .await
        .map_err(plugin_internal_error)?;
        self.load_global_bot_token().await
    }
}
