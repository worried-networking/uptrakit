use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::provider_configs::{
    CreateProviderConfigRequest, ProviderConfigResponse, UpdateProviderConfigRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// Create a new provider configuration.
    pub async fn create_provider_config(
        &self,
        req: &CreateProviderConfigRequest,
    ) -> Result<ProviderConfigResponse> {
        self.post_json("/api/v1/provider-configs", req).await
    }

    /// List provider configurations with pagination.
    pub async fn list_provider_configs(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<ProviderConfigResponse>> {
        self.get_with_query("/api/v1/provider-configs", params)
            .await
    }

    /// Get a single provider configuration by ID.
    pub async fn get_provider_config(&self, id: &Uuid) -> Result<ProviderConfigResponse> {
        let path = format!("/api/v1/provider-configs/{id}");
        self.get(&path).await
    }

    /// Update an existing provider configuration.
    pub async fn update_provider_config(
        &self,
        id: &Uuid,
        req: &UpdateProviderConfigRequest,
    ) -> Result<ProviderConfigResponse> {
        let path = format!("/api/v1/provider-configs/{id}");
        self.put_json(&path, req).await
    }

    /// Delete a provider configuration.
    pub async fn delete_provider_config(&self, id: &Uuid) -> Result<()> {
        let path = format!("/api/v1/provider-configs/{id}");
        self.delete(&path).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_shared_types::ProviderType;
    use uptrakit_web_api_types::pagination::PaginationParams;
    use uptrakit_web_api_types::provider_configs::{
        CreateProviderConfigRequest, UpdateProviderConfigRequest,
    };

    #[test]
    fn create_provider_config_request_serialization() {
        let req = CreateProviderConfigRequest {
            name: "GitHub Releases".to_string(),
            provider_type: ProviderType::GithubReleases,
            config: serde_json::json!({"owner": "nodejs", "repo": "node"}),
            enabled: true,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "GitHub Releases");
        assert_eq!(json["provider_type"], "github_releases");
        assert_eq!(json["config"]["owner"], "nodejs");
        assert_eq!(json["enabled"], true);
    }

    #[test]
    fn update_provider_config_request_serialization() {
        let req = UpdateProviderConfigRequest {
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
    fn pagination_params_for_provider_configs() {
        let params = PaginationParams {
            page: Some(1),
            per_page: Some(25),
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.contains("page=1"));
        assert!(qs.contains("per_page=25"));
    }
}
