use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;

use crate::error::{ProviderError, Result};
use crate::types::{
    DiscoveredSoftware, ProviderCapability, ProviderType, ReleaseInfo, UpstreamRelease,
};
use crate::version::Version;
use uptrakit_command::UpdateOutputLine;

/// Empty capabilities slice for providers that have no special capabilities.
const NO_CAPABILITIES: &[ProviderCapability] = &[];

/// Describes a single command that a provider needs to run with passwordless sudo.
///
/// Providers return a [`Vec<SudoCommandEntry>`] from
/// [`Provider::required_sudo_commands`] to declare which commands they need
/// elevated privileges for. The bootstrap process and `update-sudoers` command
/// use these declarations to generate minimal, specific sudoers entries instead
/// of a blanket `NOPASSWD: ALL` rule.
///
/// # Contract
///
/// - `command` must be a **bare command name** (e.g. `"apt-get"`), never an
///   absolute path. The agent resolves it to an absolute path on the target
///   host at sudoers-generation time using `command -v`.
/// - `explanation` is shown as a comment in the generated sudoers file and in
///   CLI output for human reviewers.
pub struct SudoCommandEntry {
    /// Bare command name (e.g. `"apt-get"`).
    ///
    /// Must not contain path separators or shell metacharacters.
    pub command: String,
    /// Human-readable explanation shown in sudoers comments and CLI output.
    pub explanation: String,
}

/// A unified provider trait for both remote and local operations.
///
/// This trait abstracts over both controller-side (remote) and agent-side (local)
/// provider operations. Each provider may declare its capabilities, and all
/// methods have default implementations that return appropriate errors,
/// empty results, or no capabilities.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Returns the provider type for this instance.
    ///
    /// Used for logging, telemetry, and debugging after a provider is boxed
    /// as `Box<dyn Provider>` (which erases the concrete type).
    fn provider_type(&self) -> ProviderType;

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
    async fn fetch_releases(&self, _package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        Err(report!(ProviderError::Configuration(
            "fetch_releases not supported by this provider".to_string()
        )))
    }

    /// Detect the currently installed version (local operation).
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn detect_installed_version(&self, _package_identifier: &str) -> Result<Option<Version>> {
        Err(report!(ProviderError::Configuration(
            "detect_installed_version not supported by this provider".to_string()
        )))
    }

    /// Execute an update with full context (local operation).
    ///
    /// Providers implement this to perform the actual update. Output is streamed
    /// through the provided channel. Returns the accumulated output on success.
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn execute_update(
        &self,
        _package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        Err(report!(ProviderError::Configuration(
            "execute_update not supported by this provider".to_string()
        )))
    }

    /// Discover software that this provider can manage on the local system.
    ///
    /// Returns a list of discovered software with their identifiers and optionally
    /// detected installed versions. Providers that do not support discovery return
    /// an error via the default implementation.
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        Err(report!(ProviderError::Configuration(
            "discover_software not supported by this provider".to_string()
        )))
    }

    /// Refresh the local package index from remote sources.
    ///
    /// This is the equivalent of `apt update` or `brew update` — it syncs the local
    /// package database without installing or upgrading packages. Default implementation
    /// returns an error indicating the operation is not supported.
    async fn refresh_package_index(&self) -> Result<()> {
        Err(report!(ProviderError::Configuration(
            "refresh_package_index not supported by this provider".to_string()
        )))
    }

    /// Returns the list of commands this provider needs to run with passwordless sudo.
    ///
    /// The bootstrap process and the `update-sudoers` CLI command use these
    /// declarations to generate minimal, per-command sudoers entries. Providers
    /// that never execute privileged commands should return an empty `Vec` (the
    /// default).
    ///
    /// # Provider contract
    ///
    /// - Each [`SudoCommandEntry::command`] must be a **bare command name**,
    ///   not an absolute path. The agent resolves absolute paths at sudoers-
    ///   generation time via `command -v` on the target host.
    /// - Entries are deduplicated by the caller — listing the same command
    ///   twice is harmless.
    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal Provider implementation that relies on all defaults.
    struct StubProvider;

    #[async_trait]
    impl Provider for StubProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::GithubReleases
        }
    }

    /// Provider with DiscoverLocalSoftware capability.
    struct DiscoveryProvider;

    #[async_trait]
    impl Provider for DiscoveryProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::GithubReleases
        }

        fn capabilities(&self) -> &'static [ProviderCapability] {
            &[ProviderCapability::DiscoverLocalSoftware]
        }
    }

    /// Provider with RefreshPackageIndex capability.
    struct RefreshProvider;

    #[async_trait]
    impl Provider for RefreshProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::GithubReleases
        }

        fn capabilities(&self) -> &'static [ProviderCapability] {
            &[ProviderCapability::RefreshPackageIndex]
        }
    }

    #[tokio::test]
    async fn default_fetch_releases_returns_error() {
        let provider = StubProvider;
        let result = provider.fetch_releases("example").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_detect_installed_version_returns_error() {
        let provider = StubProvider;
        let result = provider.detect_installed_version("example").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_execute_update_returns_error() {
        let provider = StubProvider;
        let (tx, _rx) = mpsc::channel(10);
        let result = provider.execute_update("test", "1.0.0", None, &tx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_discover_software_returns_error() {
        let provider = StubProvider;
        let result = provider.discover_software().await;
        assert!(result.is_err());
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

    #[tokio::test]
    async fn default_refresh_package_index_returns_error() {
        let provider = StubProvider;
        let result = provider.refresh_package_index().await;
        assert!(result.is_err());
    }

    #[test]
    fn stub_and_discovery_providers_lack_refresh_capability() {
        let stub = StubProvider;
        assert!(!stub.has_capability(ProviderCapability::RefreshPackageIndex));

        let discovery = DiscoveryProvider;
        assert!(!discovery.has_capability(ProviderCapability::RefreshPackageIndex));
    }

    #[test]
    fn refresh_provider_has_refresh_but_not_discover() {
        let refresh = RefreshProvider;
        assert!(refresh.has_capability(ProviderCapability::RefreshPackageIndex));
        assert!(!refresh.has_capability(ProviderCapability::DiscoverLocalSoftware));
    }

    /// Provider with multiple capabilities.
    struct MultiCapabilityProvider;

    #[async_trait]
    impl Provider for MultiCapabilityProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::GithubReleases
        }

        fn capabilities(&self) -> &'static [ProviderCapability] {
            &[
                ProviderCapability::DiscoverLocalSoftware,
                ProviderCapability::RefreshPackageIndex,
            ]
        }
    }

    #[test]
    fn has_capability_with_multiple_capabilities() {
        let provider = MultiCapabilityProvider;

        // First in slice
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
        // Last in slice
        assert!(provider.has_capability(ProviderCapability::RefreshPackageIndex));
    }

    #[test]
    fn capabilities_returns_correct_count_for_multi() {
        let provider = MultiCapabilityProvider;
        assert_eq!(provider.capabilities().len(), 2);
    }

    #[tokio::test]
    async fn default_error_messages_contain_operation_name() {
        let provider = StubProvider;

        let err = provider.fetch_releases("pkg").await.unwrap_err();
        assert!(
            format!("{}", err.current_context()).contains("fetch_releases"),
            "fetch_releases error should mention the operation"
        );

        let err = provider.detect_installed_version("pkg").await.unwrap_err();
        assert!(
            format!("{}", err.current_context()).contains("detect_installed_version"),
            "detect_installed_version error should mention the operation"
        );

        let err = provider.discover_software().await.unwrap_err();
        assert!(
            format!("{}", err.current_context()).contains("discover_software"),
            "discover_software error should mention the operation"
        );

        let err = provider.refresh_package_index().await.unwrap_err();
        assert!(
            format!("{}", err.current_context()).contains("refresh_package_index"),
            "refresh_package_index error should mention the operation"
        );
    }
}
