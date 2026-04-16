use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::settings_provider_github::{
    GitHubProviderSettingsResponse, UpdateGitHubProviderSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum ProviderGithubCommands {
    /// Show the shared GitHub provider defaults
    Show,
    /// Set one or both shared GitHub provider defaults
    Set {
        /// Shared GitHub auth token
        #[arg(long)]
        auth_token: Option<String>,
        /// Shared GitHub API base URL
        #[arg(long)]
        api_base_url: Option<String>,
    },
    /// Clear both shared GitHub provider defaults
    Clear,
}

impl HumanOutput for GitHubProviderSettingsResponse {
    fn to_human_string(&self) -> String {
        let api_base_url = self.api_base_url.as_deref().unwrap_or("-");
        let auth_token = self.auth_token.as_deref().unwrap_or("-");
        format!(
            "API Base URL:   {api_base_url}\nHas Auth Token: {has_auth_token}\nAuth Token:     {auth_token}\n",
            has_auth_token = self.has_auth_token
        )
    }
}

pub async fn provider_github_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<GitHubProviderSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_github_provider_settings().await.context_to()
}

pub async fn provider_github_set(
    auth_token: Option<String>,
    api_base_url: Option<String>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<GitHubProviderSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateGitHubProviderSettingsRequest {
        auth_token,
        api_base_url,
    };
    client
        .update_github_provider_settings(&req)
        .await
        .context_to()
}

pub async fn provider_github_clear(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<GitHubProviderSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateGitHubProviderSettingsRequest {
        auth_token: Some(String::new()),
        api_base_url: Some(String::new()),
    };
    client
        .update_github_provider_settings(&req)
        .await
        .context_to()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_github_settings_human_output_with_values() {
        let resp = GitHubProviderSettingsResponse {
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
            has_auth_token: true,
            auth_token: Some("***".to_string()),
        };
        let rendered = resp.to_human_string();
        assert!(rendered.contains("https://ghe.example.com/api/v3"));
        assert!(rendered.contains("true"));
        assert!(rendered.contains("***"));
    }

    #[test]
    fn provider_github_settings_human_output_empty_state() {
        let resp = GitHubProviderSettingsResponse {
            api_base_url: None,
            has_auth_token: false,
            auth_token: None,
        };
        let rendered = resp.to_human_string();
        assert!(rendered.contains('-'));
        assert!(rendered.contains("false"));
    }
}
