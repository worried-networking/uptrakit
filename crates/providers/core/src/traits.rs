use crate::error::Result;
use crate::types::{DiscoveredSoftware, UpstreamRelease};
use crate::version::Version;

/// A provider that fetches release metadata from a remote source.
///
/// Runs on the controller side to check for available upstream versions.
pub trait RemoteProvider: Send + Sync {
    /// Fetch available releases from the upstream source.
    fn fetch_releases(&self) -> impl Future<Output = Result<Vec<UpstreamRelease>>> + Send;
}

/// A provider that detects installed versions and executes updates locally.
///
/// Runs on the agent side for local version detection and update execution.
/// Defined for future agent-side use.
pub trait LocalProvider: Send + Sync {
    /// Detect the currently installed version, if any.
    fn detect_installed_version(&self) -> impl Future<Output = Result<Option<Version>>> + Send;

    /// Execute an update to the specified release.
    fn execute_update(&self, release: &UpstreamRelease) -> impl Future<Output = Result<()>> + Send;

    /// Discover software items that this provider can manage on the local system.
    ///
    /// Returns a list of discovered software with their identifiers and optionally
    /// detected installed versions. Providers that do not support discovery return
    /// an empty list via the default implementation.
    fn discover_software(&self) -> impl Future<Output = Result<Vec<DiscoveredSoftware>>> + Send {
        std::future::ready(Ok(vec![]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal LocalProvider implementation that relies on the default `discover_software()`.
    struct StubProvider;

    impl LocalProvider for StubProvider {
        fn detect_installed_version(&self) -> impl Future<Output = Result<Option<Version>>> + Send {
            std::future::ready(Ok(None))
        }

        fn execute_update(
            &self,
            _release: &UpstreamRelease,
        ) -> impl Future<Output = Result<()>> + Send {
            std::future::ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn default_discover_software_returns_empty_list() {
        let provider = StubProvider;
        let result = provider.discover_software().await.expect("should succeed");
        assert!(result.is_empty());
    }
}
