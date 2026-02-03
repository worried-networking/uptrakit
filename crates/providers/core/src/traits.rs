use async_trait::async_trait;

use crate::error::Result;
use crate::types::{DiscoveredSoftware, UpstreamRelease};
use crate::version::Version;

/// A provider that fetches release metadata from a remote source.
///
/// Runs on the controller side to check for available upstream versions.
#[async_trait]
pub trait RemoteProvider: Send + Sync {
    /// Fetch available releases from the upstream source.
    async fn fetch_releases(&self) -> Result<Vec<UpstreamRelease>>;
}

/// A provider that detects installed versions and executes updates locally.
///
/// Runs on the agent side for local version detection and update execution.
/// Defined for future agent-side use.
#[async_trait]
pub trait LocalProvider: Send + Sync {
    /// Detect the currently installed version, if any.
    async fn detect_installed_version(&self) -> Result<Option<Version>>;

    /// Execute an update to the specified release.
    async fn execute_update(&self, release: &UpstreamRelease) -> Result<()>;

    /// Discover software items that this provider can manage on the local system.
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

    /// Minimal LocalProvider implementation that relies on the default `discover_software()`.
    struct StubProvider;

    #[async_trait]
    impl LocalProvider for StubProvider {
        async fn detect_installed_version(&self) -> Result<Option<Version>> {
            Ok(None)
        }

        async fn execute_update(&self, _release: &UpstreamRelease) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn default_discover_software_returns_empty_list() {
        let provider = StubProvider;
        let result = provider.discover_software().await.expect("should succeed");
        assert!(result.is_empty());
    }
}
