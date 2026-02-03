use rootcause::report;
use uptrakit_provider_core::{LocalProvider, ProviderError, Result, UpstreamRelease, Version};

/// Local provider for Docker Registry.
///
/// Provides stub implementation for version detection and updates.
pub struct DockerRegistryLocalProvider;

impl DockerRegistryLocalProvider {
    /// Create a new Docker Registry local provider.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DockerRegistryLocalProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalProvider for DockerRegistryLocalProvider {
    fn detect_installed_version(&self) -> impl Future<Output = Result<Option<Version>>> + Send {
        // Stub: version detection not yet implemented
        std::future::ready(Ok(None))
    }

    fn execute_update(
        &self,
        _release: &UpstreamRelease,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(report!(ProviderError::Configuration(
            "execute_update not yet implemented for Docker Registry provider".to_string()
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = DockerRegistryLocalProvider::new();
        let result = provider.detect_installed_version().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_returns_error() {
        let provider = DockerRegistryLocalProvider::new();
        let release = UpstreamRelease {
            version: Version::new("1.0.0"),
            tag: "1.0.0".to_string(),
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
