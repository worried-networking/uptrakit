use crate::error::Result;
use crate::types::UpstreamRelease;
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
}
