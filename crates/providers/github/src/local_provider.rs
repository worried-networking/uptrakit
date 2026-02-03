use rootcause::report;
use uptrakit_provider_core::{LocalProvider, ProviderError, Result, UpstreamRelease, Version};

use crate::config::GitHubConfig;

/// Local provider for GitHub Releases.
///
/// Provides stub implementation for version detection and updates.
pub struct GitHubLocalProvider {
    /// Provider configuration.
    pub config: GitHubConfig,
    /// Package identifier (owner/repo).
    pub package_identifier: String,
}

impl GitHubLocalProvider {
    /// Create a new GitHub local provider.
    pub fn new(config: GitHubConfig, package_identifier: String) -> Self {
        Self {
            config,
            package_identifier,
        }
    }
}

impl LocalProvider for GitHubLocalProvider {
    fn detect_installed_version(&self) -> impl Future<Output = Result<Option<Version>>> + Send {
        // Stub: version detection not yet implemented
        std::future::ready(Ok(None))
    }

    fn execute_update(
        &self,
        _release: &UpstreamRelease,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(report!(ProviderError::Configuration(
            "execute_update not yet implemented for GitHub provider".to_string()
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> GitHubConfig {
        GitHubConfig {
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
        }
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = GitHubLocalProvider::new(test_config(), "octocat/hello-world".to_string());
        let result = provider.detect_installed_version().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_returns_error() {
        let provider = GitHubLocalProvider::new(test_config(), "octocat/hello-world".to_string());
        let release = UpstreamRelease {
            version: Version::new("1.0.0"),
            tag: "v1.0.0".to_string(),
            is_prerelease: false,
            release_url: "https://example.com".to_string(),
            release_notes: None,
            published_at: None,
            assets: vec![],
        };
        let result = provider.execute_update(&release).await;
        assert!(result.is_err());
    }
}
