use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;

use crate::error::{PluginError, Result};
use crate::types::{
    DiscoveredSoftware, PluginCapability, PluginType, ReleaseInfo, UpstreamRelease,
};
use crate::version::Version;
use uptrakit_command::UpdateOutputLine;

/// Empty capabilities slice for plugins that have no special capabilities.
const NO_CAPABILITIES: &[PluginCapability] = &[];

/// Whether a plugin is applicable to the current host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCompatibility {
    /// Plugin is applicable.
    Compatible,
    /// Plugin is not applicable (e.g., APT on a non-Debian host).
    Incompatible(String),
}

/// Contextual data passed to plugin lifecycle hooks.
#[derive(Debug, Clone)]
pub struct UpdateHookContext {
    /// The package identifier being updated.
    pub package_identifier: String,
    /// The target version being installed.
    pub to_version: String,
    /// Optional release metadata from the upstream source.
    pub release_info: Option<ReleaseInfo>,
}

/// Result of a pre-update hook.
#[derive(Debug, Clone)]
pub struct PreUpdateHookResult {
    /// Whether the update should proceed.
    pub should_proceed: bool,
    /// Reason for aborting if `should_proceed` is false.
    pub abort_reason: Option<String>,
}

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
pub trait Plugin: Send + Sync {
    /// Returns the provider type for this instance.
    ///
    /// Used for logging, telemetry, and debugging after a provider is boxed
    /// as `Box<dyn Provider>` (which erases the concrete type).
    fn plugin_type(&self) -> PluginType;

    /// Returns the capabilities supported by this provider instance.
    ///
    /// Default implementation returns an empty slice (no capabilities).
    fn capabilities(&self) -> &'static [PluginCapability] {
        NO_CAPABILITIES
    }

    /// Check if the provider has a specific capability.
    fn has_capability(&self, capability: PluginCapability) -> bool {
        self.capabilities().contains(&capability)
    }

    /// Fetch available releases from the upstream source (remote operation).
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn fetch_releases(&self, _package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        Err(report!(PluginError::Configuration(
            "fetch_releases not supported by this provider".to_string()
        )))
    }

    /// Detect the currently installed version (local operation).
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn detect_installed_version(&self, _package_identifier: &str) -> Result<Option<Version>> {
        Err(report!(PluginError::Configuration(
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
        Err(report!(PluginError::Configuration(
            "execute_update not supported by this provider".to_string()
        )))
    }

    /// Discover software that this provider can manage on the local system.
    ///
    /// Returns a list of discovered software with their identifiers and optionally
    /// detected installed versions. Providers that do not support discovery return
    /// an error via the default implementation.
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        Err(report!(PluginError::Configuration(
            "discover_software not supported by this provider".to_string()
        )))
    }

    /// Refresh the local package index from remote sources.
    ///
    /// This is the equivalent of `apt update` or `brew update` — it syncs the local
    /// package database without installing or upgrading packages. Default implementation
    /// returns an error indicating the operation is not supported.
    async fn refresh_package_index(&self) -> Result<()> {
        Err(report!(PluginError::Configuration(
            "refresh_package_index not supported by this provider".to_string()
        )))
    }

    /// Detect whether this plugin is applicable to the current host.
    ///
    /// Used before running updates to determine whether the plugin makes sense
    /// on the host (e.g., APT is incompatible on non-Debian systems). The
    /// default implementation always returns [`HostCompatibility::Compatible`]
    /// — plugins opt in to incompatibility detection by overriding this method
    /// and declaring [`PluginCapability::DetectHostCompatibility`].
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        Ok(HostCompatibility::Compatible)
    }

    /// Run before an update is applied.
    ///
    /// The hook may abort the update by returning a result with
    /// `should_proceed = false`. Plugins that implement this method must
    /// declare [`PluginCapability::PreUpdateHook`]. The default implementation
    /// always proceeds.
    async fn pre_update_hook(
        &self,
        _ctx: &UpdateHookContext,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<PreUpdateHookResult> {
        Ok(PreUpdateHookResult {
            should_proceed: true,
            abort_reason: None,
        })
    }

    /// Run after an update has been applied.
    ///
    /// Errors from this hook are logged at `WARN` level and do not fail the
    /// update. Plugins that implement this method must declare
    /// [`PluginCapability::PostUpdateHook`]. The default implementation is a
    /// no-op.
    async fn post_update_hook(
        &self,
        _ctx: &UpdateHookContext,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<()> {
        Ok(())
    }

    /// Returns the list of commands this plugin needs to run with passwordless sudo.
    ///
    /// The bootstrap process and the `update-sudoers` CLI command use these
    /// declarations to generate minimal, per-command sudoers entries. Plugins
    /// that never execute privileged commands should return an empty `Vec` (the
    /// default).
    ///
    /// # Plugin contract
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
    struct StubPlugin;

    #[async_trait]
    impl Plugin for StubPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::GithubReleases
        }
    }

    /// Provider with DiscoverLocalSoftware capability.
    struct DiscoveryPlugin;

    #[async_trait]
    impl Plugin for DiscoveryPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::GithubReleases
        }

        fn capabilities(&self) -> &'static [PluginCapability] {
            &[PluginCapability::DiscoverLocalSoftware]
        }
    }

    /// Provider with RefreshPackageIndex capability.
    struct RefreshPlugin;

    #[async_trait]
    impl Plugin for RefreshPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::GithubReleases
        }

        fn capabilities(&self) -> &'static [PluginCapability] {
            &[PluginCapability::RefreshPackageIndex]
        }
    }

    #[tokio::test]
    async fn default_fetch_releases_returns_error() {
        let provider = StubPlugin;
        let result = provider.fetch_releases("example").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_detect_installed_version_returns_error() {
        let provider = StubPlugin;
        let result = provider.detect_installed_version("example").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_execute_update_returns_error() {
        let provider = StubPlugin;
        let (tx, _rx) = mpsc::channel(10);
        let result = provider.execute_update("test", "1.0.0", None, &tx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_discover_software_returns_error() {
        let provider = StubPlugin;
        let result = provider.discover_software().await;
        assert!(result.is_err());
    }

    #[test]
    fn has_capability_returns_false_for_stub() {
        let provider = StubPlugin;
        assert!(!provider.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn has_capability_returns_true_for_discovery_provider() {
        let provider = DiscoveryPlugin;
        assert!(provider.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn capabilities_returns_correct_slice() {
        let stub = StubPlugin;
        assert!(stub.capabilities().is_empty());

        let discovery = DiscoveryPlugin;
        assert_eq!(discovery.capabilities().len(), 1);
        assert_eq!(
            discovery.capabilities()[0],
            PluginCapability::DiscoverLocalSoftware
        );
    }

    #[tokio::test]
    async fn default_refresh_package_index_returns_error() {
        let provider = StubPlugin;
        let result = provider.refresh_package_index().await;
        assert!(result.is_err());
    }

    #[test]
    fn stub_and_discovery_providers_lack_refresh_capability() {
        let stub = StubPlugin;
        assert!(!stub.has_capability(PluginCapability::RefreshPackageIndex));

        let discovery = DiscoveryPlugin;
        assert!(!discovery.has_capability(PluginCapability::RefreshPackageIndex));
    }

    #[test]
    fn refresh_provider_has_refresh_but_not_discover() {
        let refresh = RefreshPlugin;
        assert!(refresh.has_capability(PluginCapability::RefreshPackageIndex));
        assert!(!refresh.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    /// Provider with multiple capabilities.
    struct MultiCapabilityPlugin;

    #[async_trait]
    impl Plugin for MultiCapabilityPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::GithubReleases
        }

        fn capabilities(&self) -> &'static [PluginCapability] {
            &[
                PluginCapability::DiscoverLocalSoftware,
                PluginCapability::RefreshPackageIndex,
            ]
        }
    }

    #[test]
    fn has_capability_with_multiple_capabilities() {
        let provider = MultiCapabilityPlugin;

        // First in slice
        assert!(provider.has_capability(PluginCapability::DiscoverLocalSoftware));
        // Last in slice
        assert!(provider.has_capability(PluginCapability::RefreshPackageIndex));
    }

    #[test]
    fn capabilities_returns_correct_count_for_multi() {
        let provider = MultiCapabilityPlugin;
        assert_eq!(provider.capabilities().len(), 2);
    }

    #[tokio::test]
    async fn default_error_messages_contain_operation_name() {
        let provider = StubPlugin;

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

    // ── New lifecycle hook default method tests ───────────────────────────

    #[tokio::test]
    async fn default_detect_host_compatibility_returns_compatible() {
        let plugin = StubPlugin;
        let result = plugin.detect_host_compatibility().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn default_pre_update_hook_returns_proceed() {
        let plugin = StubPlugin;
        let ctx = UpdateHookContext {
            package_identifier: "test-pkg".to_string(),
            to_version: "1.0.0".to_string(),
            release_info: None,
        };
        let (tx, _rx) = mpsc::channel(10);
        let result = plugin.pre_update_hook(&ctx, &tx).await;
        assert!(result.is_ok());
        let hook_result = result.unwrap();
        assert!(hook_result.should_proceed);
        assert!(hook_result.abort_reason.is_none());
    }

    #[tokio::test]
    async fn default_post_update_hook_returns_ok() {
        let plugin = StubPlugin;
        let ctx = UpdateHookContext {
            package_identifier: "test-pkg".to_string(),
            to_version: "1.0.0".to_string(),
            release_info: None,
        };
        let (tx, _rx) = mpsc::channel(10);
        let result = plugin.post_update_hook(&ctx, &tx).await;
        assert!(result.is_ok());
    }

    #[test]
    fn stub_plugin_lacks_lifecycle_capabilities() {
        let plugin = StubPlugin;
        assert!(!plugin.has_capability(PluginCapability::DetectHostCompatibility));
        assert!(!plugin.has_capability(PluginCapability::PreUpdateHook));
        assert!(!plugin.has_capability(PluginCapability::PostUpdateHook));
    }

    #[test]
    fn host_compatibility_incompatible_carries_message() {
        let compat = HostCompatibility::Incompatible("apt-get not found".to_string());
        match compat {
            HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "apt-get not found");
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
        }
    }
}
