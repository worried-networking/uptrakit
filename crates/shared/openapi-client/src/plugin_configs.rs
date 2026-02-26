use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse, UpdatePluginConfigRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// Create a new plugin configuration.
    pub async fn create_plugin_config(
        &self,
        req: &CreatePluginConfigRequest,
    ) -> Result<PluginConfigResponse> {
        self.post_json(crate::paths::plugin_configs::BASE, req)
            .await
    }

    /// List plugin configurations with pagination.
    pub async fn list_plugin_configs(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<PluginConfigResponse>> {
        self.get_with_query(crate::paths::plugin_configs::BASE, params)
            .await
    }

    /// Fetch all plugin configurations across all pages.
    ///
    /// Automatically iterates through every page at [`MAX_PER_PAGE`] items per
    /// request. Use [`list_plugin_configs`] for manual pagination control.
    ///
    /// [`MAX_PER_PAGE`]: uptrakit_web_api_types::pagination::MAX_PER_PAGE
    /// [`list_plugin_configs`]: Self::list_plugin_configs
    pub async fn list_all_plugin_configs(&self) -> Result<Vec<PluginConfigResponse>> {
        let base = PaginationParams {
            page: None,
            per_page: None,
        };
        self.fetch_all_pages(crate::paths::plugin_configs::BASE, &base)
            .await
    }

    /// Get a single plugin configuration by ID.
    pub async fn get_plugin_config(&self, id: &Uuid) -> Result<PluginConfigResponse> {
        self.get(&crate::paths::plugin_configs::by_id(id)).await
    }

    /// Update an existing plugin configuration.
    pub async fn update_plugin_config(
        &self,
        id: &Uuid,
        req: &UpdatePluginConfigRequest,
    ) -> Result<PluginConfigResponse> {
        self.put_json(&crate::paths::plugin_configs::by_id(id), req)
            .await
    }

    /// Delete a plugin configuration.
    pub async fn delete_plugin_config(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::plugin_configs::by_id(id)).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_shared_types::PluginType;
    use uptrakit_web_api_types::pagination::PaginationParams;
    use uptrakit_web_api_types::plugin_configs::{
        CreatePluginConfigRequest, UpdatePluginConfigRequest,
    };

    #[test]
    fn create_plugin_config_request_serialization() {
        let req = CreatePluginConfigRequest {
            name: "GitHub Releases".to_string(),
            plugin_type: PluginType::ReleasesGithub,
            config: serde_json::json!({"tag_strip_prefix": "v", "include_prereleases": false}),
            enabled: true,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "GitHub Releases");
        assert_eq!(json["plugin_type"], "releases_github");
        assert_eq!(json["config"]["tag_strip_prefix"], "v");
        assert_eq!(json["enabled"], true);
    }

    #[test]
    fn update_plugin_config_request_serialization() {
        let req = UpdatePluginConfigRequest {
            name: Some("Updated Config".to_string()),
            config: Some(serde_json::json!({"key": "value"})),
            enabled: Some(false),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Updated Config");
        assert_eq!(json["config"]["key"], "value");
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn pagination_params_for_plugin_configs() {
        let params = PaginationParams {
            page: Some(1),
            per_page: Some(25),
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.contains("page=1"));
        assert!(qs.contains("per_page=25"));
    }
}
