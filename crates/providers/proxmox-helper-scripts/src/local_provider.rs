use async_trait::async_trait;
use rootcause::report;
use uptrakit_provider_core::{
    Provider, ProviderCapability, ProviderError, Result, UpstreamRelease, Version,
};

/// Local provider for Proxmox Helper Scripts.
///
/// Provides stub implementation for version detection and updates.
pub struct ProxmoxHelperScriptsLocalProvider {
    /// Package identifier (script name).
    pub package_identifier: String,
}

impl ProxmoxHelperScriptsLocalProvider {
    /// Create a new Proxmox Helper Scripts local provider.
    pub fn new(package_identifier: String) -> Self {
        Self { package_identifier }
    }
}

#[async_trait]
impl Provider for ProxmoxHelperScriptsLocalProvider {
    fn capabilities(&self) -> &'static [ProviderCapability] {
        &[ProviderCapability::DiscoverLocalSoftware]
    }

    async fn detect_installed_version(&self) -> Result<Option<Version>> {
        // Stub: version detection not yet implemented
        Ok(None)
    }

    async fn execute_update(&self, _release: &UpstreamRelease) -> Result<()> {
        Err(report!(ProviderError::Configuration(
            "execute_update not yet implemented for Proxmox Helper Scripts provider".to_string()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = ProxmoxHelperScriptsLocalProvider::new("test-script".to_string());
        let result = provider.detect_installed_version().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_returns_error() {
        let provider = ProxmoxHelperScriptsLocalProvider::new("test-script".to_string());
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
