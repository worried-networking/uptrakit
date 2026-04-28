use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::plugin_type_settings::{
    PluginTypeSettingsResponse, UpsertPluginTypeSettingsRequest,
};

impl UptrakitClient {
    /// List all plugin-type-level settings.
    ///
    /// Returns settings for every plugin type that has been configured.
    pub async fn list_plugin_type_settings(&self) -> Result<Vec<PluginTypeSettingsResponse>> {
        self.get(crate::paths::plugin_type_settings::BASE).await
    }

    /// Get plugin-type-level settings for a specific plugin type.
    pub async fn get_plugin_type_settings(
        &self,
        plugin_type: &str,
    ) -> Result<PluginTypeSettingsResponse> {
        self.get(&crate::paths::plugin_type_settings::by_type(plugin_type))
            .await
    }

    /// Create or update plugin-type-level settings for a specific plugin type.
    pub async fn upsert_plugin_type_settings(
        &self,
        plugin_type: &str,
        req: &UpsertPluginTypeSettingsRequest,
    ) -> Result<PluginTypeSettingsResponse> {
        self.put_json(
            &crate::paths::plugin_type_settings::by_type(plugin_type),
            req,
        )
        .await
    }

    /// Delete plugin-type-level settings for a specific plugin type.
    pub async fn delete_plugin_type_settings(&self, plugin_type: &str) -> Result<()> {
        self.delete(&crate::paths::plugin_type_settings::by_type(plugin_type))
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::generated::types::plugin_type_settings::UpsertPluginTypeSettingsRequest;

    #[test]
    fn upsert_plugin_type_settings_request_serialization() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::json!({"poll_interval_secs": 300, "max_pages": 10}),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["config"]["poll_interval_secs"], 300);
        assert_eq!(json["config"]["max_pages"], 10);
    }

    #[test]
    fn upsert_plugin_type_settings_request_empty_config() {
        let req = UpsertPluginTypeSettingsRequest {
            config: serde_json::json!({}),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert!(json["config"].is_object());
    }
}
