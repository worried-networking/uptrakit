use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;

use crate::batch_detect::{BatchDetectItem, BatchDetectResult};
use crate::batch_fetch::{BatchFetchItem, BatchFetchResult};
use crate::batch_update::{BatchUpdateItem, BatchUpdateResult};
use crate::error::{PluginError, Result};
use crate::types::{
    DiscoveredSoftware, PluginCapability, PluginType, ReleaseInfo, UpstreamRelease,
};
use crate::version::Version;
use uptrakit_command::UpdateOutputLine;

/// Empty capabilities slice for plugins that have no special capabilities.
const NO_CAPABILITIES: &[PluginCapability] = &[];

/// Whether a plugin is applicable to the current host.
#[non_exhaustive]
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

/// A helper script installed by the bootstrap process on the managed host.
///
/// Enables argument-validated sudo commands — something sudoers wildcards
/// cannot express safely, because `*` in sudoers matches `/`, making
/// path-based wildcard restrictions ineffective (e.g. `/usr/bin/cat /root/.*`
/// would still allow reading `/root/.ssh/id_rsa`).
///
/// The script must validate its own arguments before acting, making the
/// corresponding sudoers entry unconditional (no argument wildcard needed):
///
/// ```text
/// uptrakit ALL=(root) NOPASSWD: /usr/local/bin/my-helper
/// ```
///
/// # Contract
///
/// - `install_path` must be an absolute path (e.g. `/usr/local/bin/my-helper`).
/// - `content` must be a complete, self-contained shell script that validates
///   its arguments and exits non-zero on invalid input.
/// - The script is installed with mode `0755` and owned by root.
pub struct SudoHelperScript {
    /// Absolute path where the script is installed on the managed host.
    ///
    /// Used directly as the sudoers command (no `command -v` resolution).
    pub install_path: &'static str,
    /// Complete shell script content installed verbatim at `install_path`.
    pub content: &'static str,
}

/// Describes a single command that a plugin needs to run with passwordless sudo.
///
/// Plugins return a [`Vec<SudoCommandEntry>`] from
/// [`Plugin::required_sudo_commands`] to declare which commands they need
/// elevated privileges for. The bootstrap process and `update-sudoers` command
/// use these declarations to generate minimal, specific sudoers entries instead
/// of a blanket `NOPASSWD: ALL` rule.
///
/// # Contract
///
/// When `helper_script` is `None`:
/// - `command` must be a **bare command name** (e.g. `"apt-get"`), never an
///   absolute path. The agent resolves it to an absolute path on the target
///   host at sudoers-generation time using `command -v`.
///
/// When `helper_script` is `Some`:
/// - `command` is used only as a display name (not resolved via `command -v`).
/// - Bootstrap installs the script at `helper_script.install_path`, sets
///   permissions to `0755`, and uses that path as the sudoers command.
///   The script's own argument validation enforces restrictions that sudoers
///   wildcards cannot safely express.
///
/// In both cases `explanation` is shown as a comment in the generated sudoers
/// file and in CLI output for human reviewers.
pub struct SudoCommandEntry {
    /// Bare command name (e.g. `"apt-get"`) or a short display identifier for
    /// helper scripts.
    ///
    /// When `helper_script` is `None`, this is resolved to an absolute path
    /// via `command -v` on the target host. When `helper_script` is `Some`,
    /// this field is used only for logging and display purposes; the sudoers
    /// entry uses `helper_script.install_path`.
    pub command: String,
    /// Human-readable explanation shown in sudoers comments and CLI output.
    pub explanation: String,
    /// Optional helper script to install on the managed host during bootstrap.
    ///
    /// When `Some`, bootstrap installs this script before writing the sudoers
    /// entry. The sudoers entry uses the install path directly. The script must
    /// validate its own arguments to enforce the least-privilege contract.
    pub helper_script: Option<SudoHelperScript>,
    /// When `true`, the sudoers entry is generated with the `SETENV:` tag, which
    /// allows the agent to pass inline `NAME=VALUE` env var assignments before
    /// the program name (e.g. `sudo DEBIAN_FRONTEND=noninteractive apt-get …`).
    ///
    /// Set this to `true` only when the plugin invokes the command with
    /// [`CommandSpec::with_env`] in combination with `.privileged()`.
    /// Defaults to `false` for commands that don't need env var forwarding.
    pub needs_setenv: bool,
}

/// A unified plugin trait for both remote and local operations.
///
/// This trait abstracts over both controller-side (remote) and agent-side (local)
/// plugin operations. Each plugin may declare its capabilities, and all
/// methods have default implementations that return appropriate errors,
/// empty results, or no capabilities.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Returns the plugin type for this instance.
    ///
    /// Used for logging, telemetry, and debugging after a plugin is boxed
    /// as `Box<dyn Plugin>` (which erases the concrete type).
    fn plugin_type(&self) -> PluginType;

    /// Returns the capabilities supported by this plugin instance.
    ///
    /// Default implementation returns an empty slice (no capabilities).
    /// Plugins with capabilities should override this to return their
    /// inherent `CAPABILITIES` constant.
    fn capabilities(&self) -> &'static [PluginCapability] {
        NO_CAPABILITIES
    }

    /// Check if the plugin has a specific capability.
    fn has_capability(&self, capability: PluginCapability) -> bool {
        self.capabilities().contains(&capability)
    }

    /// Fetch available releases from the upstream source (remote operation).
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn fetch_releases(&self, _package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        Err(report!(PluginError::Configuration(
            "fetch_releases not supported by this plugin".to_string()
        )))
    }

    /// Detect the currently installed version (local operation).
    ///
    /// Default implementation returns an error indicating the operation is not supported.
    async fn detect_installed_version(&self, _package_identifier: &str) -> Result<Option<Version>> {
        Err(report!(PluginError::Configuration(
            "detect_installed_version not supported by this plugin".to_string()
        )))
    }

    /// Execute an update with full context (local operation).
    ///
    /// Plugins implement this to perform the actual update. Output is streamed
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
            "execute_update not supported by this plugin".to_string()
        )))
    }

    /// Execute updates for multiple packages in a single operation.
    ///
    /// Package managers like APT, Homebrew, and npm can update multiple packages
    /// in a single command, which is more efficient than per-package calls.
    ///
    /// The default implementation falls back to calling [`execute_update`](Self::execute_update)
    /// sequentially for each item. Plugins that support native batch operations
    /// should override this for efficiency.
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            match self
                .execute_update(
                    &item.package_identifier,
                    &item.to_version,
                    item.release_info.as_ref(),
                    output_tx,
                )
                .await
            {
                Ok(output) => results.push(BatchUpdateResult {
                    package_identifier: item.package_identifier.clone(),
                    success: true,
                    output,
                }),
                Err(e) => results.push(BatchUpdateResult {
                    package_identifier: item.package_identifier.clone(),
                    success: false,
                    output: format!("{e}"),
                }),
            }
        }
        Ok(results)
    }

    /// Detect installed versions for multiple packages in one operation.
    ///
    /// Default falls back to sequential [`detect_installed_version`](Self::detect_installed_version)
    /// calls, capturing per-item errors without failing the whole batch.
    /// Plugins backed by commands that accept multiple packages (APT, Homebrew, npm)
    /// should override this for significant efficiency gains.
    ///
    /// The default implementation never returns `Err` at the batch level —
    /// individual item failures are recorded in [`BatchDetectResult::error`].
    /// An empty `items` slice always returns an empty `Vec`.
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            match self.detect_installed_version(&item.package_identifier).await {
                Ok(v) => results.push(BatchDetectResult {
                    package_identifier: item.package_identifier.clone(),
                    installed_version: v,
                    error: None,
                }),
                Err(e) => results.push(BatchDetectResult {
                    package_identifier: item.package_identifier.clone(),
                    installed_version: None,
                    error: Some(e.to_string()),
                }),
            }
        }
        Ok(results)
    }

    /// Fetch releases for multiple packages in one operation.
    ///
    /// Default falls back to sequential [`fetch_releases`](Self::fetch_releases)
    /// calls, capturing per-item errors without failing the whole batch.
    /// Plugins that can query multiple packages in one command (APT, Homebrew)
    /// should override this.
    ///
    /// The default implementation never returns `Err` at the batch level —
    /// individual item failures are recorded in [`BatchFetchResult::error`].
    /// An empty `items` slice always returns an empty `Vec`.
    async fn batch_fetch_releases(
        &self,
        items: &[BatchFetchItem],
    ) -> Result<Vec<BatchFetchResult>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            match self.fetch_releases(&item.package_identifier).await {
                Ok(releases) => results.push(BatchFetchResult {
                    package_identifier: item.package_identifier.clone(),
                    releases,
                    error: None,
                }),
                Err(e) => results.push(BatchFetchResult {
                    package_identifier: item.package_identifier.clone(),
                    releases: vec![],
                    error: Some(e.to_string()),
                }),
            }
        }
        Ok(results)
    }

    /// Discover software that this plugin can manage on the local system.
    ///
    /// Returns a list of discovered software with their identifiers and optionally
    /// detected installed versions. Plugins that do not support discovery return
    /// an error via the default implementation.
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        Err(report!(PluginError::Configuration(
            "discover_software not supported by this plugin".to_string()
        )))
    }

    /// Refresh the local package index from remote sources.
    ///
    /// This is the equivalent of `apt update` or `brew update` — it syncs the local
    /// package database without installing or upgrading packages. Default implementation
    /// returns an error indicating the operation is not supported.
    async fn refresh_package_index(&self) -> Result<()> {
        Err(report!(PluginError::Configuration(
            "refresh_package_index not supported by this plugin".to_string()
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
    use crate::batch_detect::BatchDetectItem;
    use crate::batch_fetch::BatchFetchItem;

    /// Minimal plugin implementation that relies on all defaults.
    struct StubPlugin;

    #[async_trait]
    impl Plugin for StubPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::ReleasesGithub
        }
    }

    /// Plugin with DiscoverLocalSoftware capability.
    struct DiscoveryPlugin;

    #[async_trait]
    impl Plugin for DiscoveryPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::ReleasesGithub
        }

        fn capabilities(&self) -> &'static [PluginCapability] {
            &[PluginCapability::DiscoverLocalSoftware]
        }
    }

    /// Plugin with RefreshPackageIndex capability.
    struct RefreshPlugin;

    #[async_trait]
    impl Plugin for RefreshPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::ReleasesGithub
        }

        fn capabilities(&self) -> &'static [PluginCapability] {
            &[PluginCapability::RefreshPackageIndex]
        }
    }

    #[tokio::test]
    async fn default_fetch_releases_returns_error() {
        let plugin = StubPlugin;
        let result = plugin.fetch_releases("example").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_detect_installed_version_returns_error() {
        let plugin = StubPlugin;
        let result = plugin.detect_installed_version("example").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_execute_update_returns_error() {
        let plugin = StubPlugin;
        let (tx, _rx) = mpsc::channel(10);
        let result = plugin.execute_update("test", "1.0.0", None, &tx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn default_discover_software_returns_error() {
        let plugin = StubPlugin;
        let result = plugin.discover_software().await;
        assert!(result.is_err());
    }

    #[test]
    fn has_capability_returns_false_for_stub() {
        let plugin = StubPlugin;
        assert!(!plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn has_capability_returns_true_for_discovery_plugin() {
        let plugin = DiscoveryPlugin;
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
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
        let plugin = StubPlugin;
        let result = plugin.refresh_package_index().await;
        assert!(result.is_err());
    }

    #[test]
    fn stub_and_discovery_plugins_lack_refresh_capability() {
        let stub = StubPlugin;
        assert!(!stub.has_capability(PluginCapability::RefreshPackageIndex));

        let discovery = DiscoveryPlugin;
        assert!(!discovery.has_capability(PluginCapability::RefreshPackageIndex));
    }

    #[test]
    fn refresh_plugin_has_refresh_but_not_discover() {
        let refresh = RefreshPlugin;
        assert!(refresh.has_capability(PluginCapability::RefreshPackageIndex));
        assert!(!refresh.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    /// Plugin with multiple capabilities.
    struct MultiCapabilityPlugin;

    #[async_trait]
    impl Plugin for MultiCapabilityPlugin {
        fn plugin_type(&self) -> PluginType {
            PluginType::ReleasesGithub
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
        let plugin = MultiCapabilityPlugin;

        // First in slice
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        // Last in slice
        assert!(plugin.has_capability(PluginCapability::RefreshPackageIndex));
    }

    #[test]
    fn capabilities_returns_correct_count_for_multi() {
        let plugin = MultiCapabilityPlugin;
        assert_eq!(plugin.capabilities().len(), 2);
    }

    #[tokio::test]
    async fn default_error_messages_contain_operation_name() {
        let plugin = StubPlugin;

        let err = plugin.fetch_releases("pkg").await.unwrap_err();
        assert!(
            format!("{}", err.current_context()).contains("fetch_releases"),
            "fetch_releases error should mention the operation"
        );

        let err = plugin.detect_installed_version("pkg").await.unwrap_err();
        assert!(
            format!("{}", err.current_context()).contains("detect_installed_version"),
            "detect_installed_version error should mention the operation"
        );

        let err = plugin.discover_software().await.unwrap_err();
        assert!(
            format!("{}", err.current_context()).contains("discover_software"),
            "discover_software error should mention the operation"
        );

        let err = plugin.refresh_package_index().await.unwrap_err();
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

    // ── batch_detect_installed_version default impl ───────────────────────

    #[tokio::test]
    async fn default_batch_detect_empty_slice_returns_empty() {
        let plugin = StubPlugin;
        let results = plugin
            .batch_detect_installed_version(&[])
            .await
            .expect("batch detect ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn default_batch_detect_records_per_item_error() {
        // StubPlugin's detect_installed_version always returns Err.
        let plugin = StubPlugin;
        let items = vec![
            BatchDetectItem {
                package_identifier: "pkg-a".to_string(),
            },
            BatchDetectItem {
                package_identifier: "pkg-b".to_string(),
            },
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("batch never fails at batch level");
        assert_eq!(results.len(), 2);
        // Per-item errors are recorded; batch call itself succeeds.
        assert!(results[0].error.is_some());
        assert!(results[0].installed_version.is_none());
        assert!(results[1].error.is_some());
        assert!(results[1].installed_version.is_none());
        assert_eq!(results[0].package_identifier, "pkg-a");
        assert_eq!(results[1].package_identifier, "pkg-b");
    }

    // ── batch_fetch_releases default impl ────────────────────────────────

    #[tokio::test]
    async fn default_batch_fetch_empty_slice_returns_empty() {
        let plugin = StubPlugin;
        let results = plugin
            .batch_fetch_releases(&[])
            .await
            .expect("batch fetch ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn default_batch_fetch_records_per_item_error() {
        // StubPlugin's fetch_releases always returns Err.
        let plugin = StubPlugin;
        let items = vec![
            BatchFetchItem {
                package_identifier: "pkg-a".to_string(),
            },
            BatchFetchItem {
                package_identifier: "pkg-b".to_string(),
            },
        ];
        let results = plugin
            .batch_fetch_releases(&items)
            .await
            .expect("batch never fails at batch level");
        assert_eq!(results.len(), 2);
        // Per-item errors are recorded; batch call itself succeeds.
        assert!(results[0].error.is_some());
        assert!(results[0].releases.is_empty());
        assert!(results[1].error.is_some());
        assert!(results[1].releases.is_empty());
        assert_eq!(results[0].package_identifier, "pkg-a");
        assert_eq!(results[1].package_identifier, "pkg-b");
    }
}
