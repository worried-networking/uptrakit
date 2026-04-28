use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::settings_provider_github::{
    GitHubProviderSettingsResponse, UpdateGitHubProviderSettingsRequest,
};

impl UptrakitClient {
    /// Get global GitHub provider settings.
    pub async fn get_github_provider_settings(&self) -> Result<GitHubProviderSettingsResponse> {
        self.get(crate::paths::settings_provider_github::BASE).await
    }

    /// Update global GitHub provider settings.
    pub async fn update_github_provider_settings(
        &self,
        req: &UpdateGitHubProviderSettingsRequest,
    ) -> Result<GitHubProviderSettingsResponse> {
        self.put_json(crate::paths::settings_provider_github::BASE, req)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::types_impl::settings_provider_github::UpdateGitHubProviderSettingsRequest;

    #[test]
    fn settings_provider_github_update_request_set_serializes() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: Some("ghp_abc123".to_string()),
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["auth_token"], "ghp_abc123");
        assert_eq!(json["api_base_url"], "https://ghe.example.com/api/v3");
    }

    #[test]
    fn settings_provider_github_update_request_clear_serializes() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: Some(String::new()),
            api_base_url: Some(String::new()),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["auth_token"], "");
        assert_eq!(json["api_base_url"], "");
    }

    #[test]
    fn settings_provider_github_update_request_keep_serializes() {
        let req = UpdateGitHubProviderSettingsRequest {
            auth_token: Some("***".to_string()),
            api_base_url: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["auth_token"], "***");
        assert!(json.get("api_base_url").is_none());
    }
}
