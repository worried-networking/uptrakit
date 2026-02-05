use async_trait::async_trait;
use rootcause::report;

use crate::error::{ProviderError, Result};
use crate::types::{DiscoveredSoftware, ProviderCapability, UpstreamRelease};
use crate::version::Version;

/// Empty capabilities slice for providers that have no special capabilities.
const NO_CAPABILITIES: &[ProviderCapability] = &[];

/// A unified provider trait for both remote and local operations.
///
/// This trait abstracts over both controller-side (remote) and agent-side (local)
/// provider operations. Each provider may declare its capabilities, and all
/// methods have default implementations that return appropriate errors,
/// empty results, or no capabilities.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Returns the capabilities supported by this provider instance.
    ///
    /// Default implementation returns an empty slice (no capabilities).
    fn capabilities(&self) -> &'static [ProviderCapability] {
        NO_CAPABILITIES
    }

    /// Check if the provider has a specific capability.
    fn has_capability(&self, capability: ProviderCapability) -> bool {
        self.capabilities().contains(&capability)
    }

    /// Fetch available releases from the upstream source (remote operation).
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn fetch_releases(&self) -> Result<Vec<UpstreamRelease>> {
        Err(report!(ProviderError::Configuration(
            "fetch_releases not supported by this provider".to_string()
        )))
    }

    /// Detect the currently installed version (local operation).
    ///
    /// Default implementation returns `None` (no version detected).
    async fn detect_installed_version(&self) -> Result<Option<Version>> {
        Ok(None)
    }

    /// Execute an update to the specified release (local operation).
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn execute_update(&self, _release: &UpstreamRelease) -> Result<()> {
        Err(report!(ProviderError::Configuration(
            "execute_update not supported by this provider".to_string()
        )))
    }

    /// Discover software that this provider can manage on the local system.
    ///
    /// Returns a list of discovered software with their identifiers and optionally
    /// detected installed versions. Providers that do not support discovery return
    /// an empty list via the default implementation.
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal Provider implementation that relies on all defaults.
    struct StubProvider;

    #[async_trait]
    impl Provider for StubProvider {}

    /// Provider with DiscoverLocalSoftware capability.
    struct DiscoveryProvider;

    #[async_trait]
    impl Provider for DiscoveryProvider {
        fn capabilities(&self) -> &'static [ProviderCapability] {
            &[ProviderCapability::DiscoverLocalSoftware]
        }
    }

    #[tokio::test]
    async fn default_fetch_releases_returns_error() {
        let provider = StubProvider;
        let result = provider.fetch_releases().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_detect_installed_version_returns_none() {
        let provider = StubProvider;
        let result = provider
            .detect_installed_version()
            .await
            .expect("should succeed");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn default_execute_update_returns_error() {
        let provider = StubProvider;
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

    #[tokio::test]
    async fn default_discover_software_returns_empty_list() {
        let provider = StubProvider;
        let result = provider.discover_software().await.expect("should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn has_capability_returns_false_for_stub() {
        let provider = StubProvider;
        assert!(!provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn has_capability_returns_true_for_discovery_provider() {
        let provider = DiscoveryProvider;
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn capabilities_returns_correct_slice() {
        let stub = StubProvider;
        assert!(stub.capabilities().is_empty());

        let discovery = DiscoveryProvider;
        assert_eq!(discovery.capabilities().len(), 1);
        assert_eq!(
            discovery.capabilities()[0],
            ProviderCapability::DiscoverLocalSoftware
        );
    }
}
