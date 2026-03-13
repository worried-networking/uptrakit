//! Unified plugin trait hierarchy.
//!
//! [`PluginBase`] is the common trait for all plugins (software, notification,
//! infrastructure). Fine-grained capability subtraits allow plugins to declare
//! exactly which operations they support. The `register_plugins!` macro
//! generates accessor methods (`as_discovery()`, `as_version_detector()`, etc.)
//! that downcast to the appropriate subtrait.
//!
//! # Subtrait overview
//!
//! | Subtrait | Capability | Purpose |
//! |----------|------------|---------|
//! | [`DiscoveryPlugin`] | `DiscoverLocalSoftware` | Discover software on the local system |
//! | [`VersionDetectorPlugin`] | `VersionDetection` | Detect installed version |
//! | [`ReleaseFetcherPlugin`] | `ReleaseFetching` | Fetch upstream releases |
//! | [`PackageIndexPlugin`] | `RefreshPackageIndex` | Sync local package database |
//! | [`UpdateExecutorPlugin`] | `UpdateExecution` | Execute updates |
//! | [`UpdateHooksPlugin`] | `PreUpdateHook`/`PostUpdateHook` | Pre/post update lifecycle hooks |
//! | [`NotificationTransportPlugin`] | `NotificationDelivery` | Deliver notification messages |
//! | [`HostLifecyclePlugin`] | `HostLifecycle` | Infrastructure host bootstrap/sync |
//! | [`HostReportPlugin`] | `HostReport` | Post-report-hosts callbacks |
//! | [`GuestExecPlugin`] | `GuestExec` | Execute commands inside infrastructure guests |

#[cfg(feature = "agent-infra")]
use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_extension_framework::{ActionDef, ExtensionManifest, FieldDef};

use crate::batch_detect::{BatchDetectItem, BatchDetectResult};
use crate::batch_fetch::{BatchFetchItem, BatchFetchResult};
use crate::batch_update::{BatchUpdateItem, BatchUpdateResult};
use crate::error::Result;
use crate::traits::{HostCompatibility, PreUpdateHookResult, SudoCommandEntry, UpdateHookContext};
use crate::types::{DiscoveredSoftware, PluginCapability, ReleaseInfo, UpstreamRelease};
use crate::version::Version;
use uptrakit_command::UpdateOutputLine;

// ── PluginBase ──────────────────────────────────────────────────────────────

/// Common trait for all plugins in the unified registry.
///
/// Every plugin (software, notification, infrastructure) implements this trait.
/// It provides config operations, form schema, extension declarations, and
/// accessor methods for downcasting to fine-grained capability subtraits.
///
/// The `register_plugins!` macro generates the `as_*()` accessor
/// implementations, returning `Some(self)` for subtraits declared in the
/// macro entry and `None` for the rest.
#[async_trait]
pub trait PluginBase: Send + Sync {
    /// Returns the plugin type identifier string (e.g. `"package_manager_apt"`,
    /// `"webhook"`, `"infrastructure_proxmox"`).
    fn plugin_type_id(&self) -> &str;

    /// Returns the capabilities supported by this plugin.
    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![]
    }

    /// Check if the plugin has a specific capability.
    fn has_capability(&self, capability: PluginCapability) -> bool {
        self.capabilities().contains(&capability)
    }

    // ── Config operations ────────────────────────────────────────────────

    /// Validate plugin configuration JSON.
    fn validate_config(&self, _config: &serde_json::Value) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Return a copy of the config with secrets replaced by `"***"`.
    #[must_use]
    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        config.clone()
    }

    /// Restore masked secrets from an existing configuration.
    ///
    /// Fields in `incoming` that equal `"***"` are replaced with the
    /// corresponding values from `stored`.
    fn restore_config_secrets(
        &self,
        incoming: &serde_json::Value,
        stored: &serde_json::Value,
    ) -> serde_json::Value {
        let Some(incoming_obj) = incoming.as_object() else {
            return incoming.clone();
        };
        let stored_obj = stored.as_object();

        let mut result = incoming_obj.clone();
        for (key, value) in &mut result {
            if value.as_str() == Some("***")
                && let Some(stored_value) = stored_obj.and_then(|o| o.get(key.as_str()))
            {
                *value = stored_value.clone();
            }
        }
        serde_json::Value::Object(result)
    }

    // ── Form schema ──────────────────────────────────────────────────────

    /// Returns form field definitions for the plugin config form.
    fn form_schema(&self) -> Vec<FieldDef> {
        vec![]
    }

    /// Returns form field definitions for the plugin type settings form.
    fn type_settings_form_schema(&self) -> Vec<FieldDef> {
        vec![]
    }

    /// Returns a sample/default JSON for type settings.
    fn type_settings_sample(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    /// Returns a sample/default configuration JSON.
    fn sample_config(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    // ── Package identifier validation ────────────────────────────────────

    /// Validate a package identifier for this plugin type.
    fn validate_package_identifier(&self, _value: &str) -> std::result::Result<(), String> {
        Ok(())
    }

    // ── Sudo commands ────────────────────────────────────────────────────

    /// Returns commands this plugin needs to run with passwordless sudo.
    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![]
    }

    // ── Extensions ───────────────────────────────────────────────────────

    /// Return UI extension manifests for this plugin.
    fn extension_manifests(&self) -> Vec<ExtensionManifest> {
        vec![]
    }

    /// Return action definitions for this plugin's extensions.
    fn extension_actions(&self) -> Vec<ActionDef> {
        vec![]
    }

    /// Return action IDs that appear in the hosts data table.
    fn primary_action_ids(&self) -> Vec<String> {
        vec![]
    }

    /// Handle a service-side extension action request.
    ///
    /// Infrastructure plugins override this to handle their UI-driven actions
    /// (e.g., Proxmox guest list, bootstrap). Returns `Some(response)` if
    /// handled, `None` to let the agent try the next plugin.
    #[cfg(feature = "agent-infra")]
    async fn handle_service_extension_action(
        &self,
        _ctx: &crate::agent_infra::InfraPluginContext<'_>,
        _request: &uptrakit_extension_framework::ExtensionRequestPayload,
    ) -> Option<uptrakit_extension_framework::ExtensionResponsePayload> {
        None
    }

    // ── Migrations ───────────────────────────────────────────────────────

    /// Return service-side (agent-local) database migrations.
    ///
    /// Plugins with [`PluginCapability::ServiceMigrations`] override this.
    #[cfg(feature = "migrations")]
    fn service_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![]
    }

    /// Return controller-side database migrations.
    ///
    /// Plugins with [`PluginCapability::ControllerMigrations`] override this.
    #[cfg(feature = "migrations")]
    fn controller_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![]
    }

    // ── Subtrait accessors ───────────────────────────────────────────────
    //
    // Defaults return `None`. The `register_plugins!` macro overrides
    // these for subtraits declared in the plugin's entry.

    /// Downcast to [`DiscoveryPlugin`], if implemented.
    fn as_discovery(&self) -> Option<&dyn DiscoveryPlugin> {
        None
    }

    /// Downcast to [`VersionDetectorPlugin`], if implemented.
    fn as_version_detector(&self) -> Option<&dyn VersionDetectorPlugin> {
        None
    }

    /// Downcast to [`ReleaseFetcherPlugin`], if implemented.
    fn as_release_fetcher(&self) -> Option<&dyn ReleaseFetcherPlugin> {
        None
    }

    /// Downcast to [`PackageIndexPlugin`], if implemented.
    fn as_package_index(&self) -> Option<&dyn PackageIndexPlugin> {
        None
    }

    /// Downcast to [`UpdateExecutorPlugin`], if implemented.
    fn as_update_executor(&self) -> Option<&dyn UpdateExecutorPlugin> {
        None
    }

    /// Downcast to [`UpdateHooksPlugin`], if implemented.
    fn as_update_hooks(&self) -> Option<&dyn UpdateHooksPlugin> {
        None
    }

    /// Downcast to [`NotificationTransportPlugin`], if implemented.
    fn as_notification_transport(&self) -> Option<&dyn NotificationTransportPlugin> {
        None
    }

    /// Downcast to [`HostLifecyclePlugin`], if implemented.
    #[cfg(feature = "agent-infra")]
    fn as_host_lifecycle(&self) -> Option<&dyn HostLifecyclePlugin> {
        None
    }

    /// Downcast to [`HostReportPlugin`], if implemented.
    #[cfg(feature = "agent-infra")]
    fn as_host_report(&self) -> Option<&dyn HostReportPlugin> {
        None
    }

    /// Downcast to [`GuestExecPlugin`], if implemented.
    #[cfg(feature = "agent-infra")]
    fn as_guest_exec(&self) -> Option<&dyn GuestExecPlugin> {
        None
    }
}

// ── Software subtraits ──────────────────────────────────────────────────────

/// Discover software that a plugin can manage on the local system.
#[async_trait]
pub trait DiscoveryPlugin: PluginBase {
    /// Discover manageable software on the local system.
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>>;

    /// Detect whether this plugin is applicable to the current host.
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        Ok(HostCompatibility::Compatible)
    }
}

/// Detect installed versions of software packages.
#[async_trait]
pub trait VersionDetectorPlugin: PluginBase {
    /// Detect the currently installed version of a package.
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>>;

    /// Detect installed versions for multiple packages in one operation.
    ///
    /// Default falls back to sequential calls. Override for batch efficiency.
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            match self
                .detect_installed_version(&item.package_identifier)
                .await
            {
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
}

/// Fetch upstream releases for software packages.
#[async_trait]
pub trait ReleaseFetcherPlugin: PluginBase {
    /// Fetch available releases from the upstream source.
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>>;

    /// Fetch releases for multiple packages in one operation.
    ///
    /// Default falls back to sequential calls. Override for batch efficiency.
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
}

/// Refresh/sync the local package index from remote sources.
#[async_trait]
pub trait PackageIndexPlugin: PluginBase {
    /// Refresh the local package index (e.g. `apt update`, `brew update`).
    async fn refresh_package_index(&self) -> Result<()>;
}

/// Execute software updates.
#[async_trait]
pub trait UpdateExecutorPlugin: PluginBase {
    /// Execute a single package update with streaming output.
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        release_info: Option<&ReleaseInfo>,
        output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String>;

    /// Execute updates for multiple packages in a single operation.
    ///
    /// Default falls back to sequential [`execute_update`](Self::execute_update) calls.
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
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
}

/// Pre/post update lifecycle hooks.
///
/// Any plugin can implement this — it is not tied to package managers.
/// For example, APT uses `pre_update_hook` to run `apt-get update` and
/// `post_update_hook` to check for reboot-required.
#[async_trait]
pub trait UpdateHooksPlugin: PluginBase {
    /// Run before an update is applied. May abort the update.
    async fn pre_update_hook(
        &self,
        ctx: &UpdateHookContext,
        output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<PreUpdateHookResult>;

    /// Run after an update has been applied. Errors are logged, not fatal.
    async fn post_update_hook(
        &self,
        ctx: &UpdateHookContext,
        output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
    ) -> Result<()>;
}

// ── Notification subtrait ───────────────────────────────────────────────────

/// Notification delivery channel (webhook, Telegram, email, etc.).
#[async_trait]
pub trait NotificationTransportPlugin: PluginBase {
    /// Returns the channel type identifier (e.g. `"webhook"`, `"telegram"`).
    fn channel_type(&self) -> &'static str;

    /// Deliver a pre-built message using the given channel-specific config.
    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &uptrakit_notification_plugin_core::DeliveryMessage,
    ) -> uptrakit_notification_plugin_core::Result<()>;
}

// ── Infrastructure subtraits ────────────────────────────────────────────────

/// Infrastructure host lifecycle hooks (bootstrap, sync).
///
/// Called by the SSH agent during host provisioning and sync operations.
#[cfg(feature = "agent-infra")]
#[async_trait]
pub trait HostLifecyclePlugin: PluginBase {
    /// Detect infrastructure during host bootstrap.
    async fn on_host_bootstrapped(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
        executor: &dyn uptrakit_command::RemoteExecutor,
        host_id: uuid::Uuid,
        host_name: &str,
    ) -> Result<crate::agent_infra::BootstrapInfraResult>;

    /// Sync infrastructure state during host sync.
    async fn on_host_synced(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
        executor: &dyn uptrakit_command::RemoteExecutor,
        host_id: uuid::Uuid,
    ) -> Result<crate::agent_infra::SyncInfraResult>;

    /// Check whether this plugin has infrastructure state for the given host.
    async fn has_infra_state(&self, db: &sea_orm::DatabaseConnection, host_id: uuid::Uuid) -> bool;
}

/// Post-report-hosts callbacks from the agent.
#[cfg(feature = "agent-infra")]
#[async_trait]
pub trait HostReportPlugin: PluginBase {
    /// Called after `ReportHosts` has been sent to the controller.
    async fn on_post_report_hosts(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
    ) -> Result<()>;

    /// Called when the controller responds to a `ReportPluginConfig` request.
    async fn on_plugin_config_reported(
        &self,
        db: &sea_orm::DatabaseConnection,
        plugin_config_id: uuid::Uuid,
        request_id: &str,
    ) -> Result<()>;
}

/// Guest execution capabilities for infrastructure plugins.
#[cfg(feature = "agent-infra")]
#[async_trait]
pub trait GuestExecPlugin: PluginBase {
    /// Return a [`GuestExecProvider`](crate::agent_infra::GuestExecProvider)
    /// for executing commands inside infrastructure guests, or `None` if not supported.
    fn guest_exec_provider(&self) -> Option<Arc<dyn crate::agent_infra::GuestExecProvider>>;
}

// ── Helper macro for config delegation ──────────────────────────────────────

/// Generates `PluginBase` config delegation methods for a plugin + config pair.
///
/// The plugin struct must have access to its config type which implements
/// [`ConfigFormSchema`], [`SecretMasking`], `Default`, and `Serialize`.
///
/// # Usage
///
/// Basic (all defaults):
/// ```ignore
/// impl_plugin_base_config!(AptPlugin, AptConfig, "package_manager_apt");
/// ```
///
/// With optional method overrides (capabilities, sudo commands, etc.),
/// wrapped in braces to prevent `cargo fmt` from inserting commas:
/// ```ignore
/// impl_plugin_base_config!(AptPlugin, AptConfig, "package_manager_apt", {
///     fn capabilities(&self) -> Vec<$crate::PluginCapability> {
///         Self::CAPABILITIES.to_vec()
///     }
///     fn required_sudo_commands(&self) -> Vec<$crate::SudoCommandEntry> {
///         vec![$crate::SudoCommandEntry::new("apt-get", "...")]
///     }
/// });
/// ```
#[macro_export]
macro_rules! impl_plugin_base_config {
    // With extra method overrides wrapped in braces.
    ($plugin:ty, $config:ty, $type_id:expr, { $($extra_methods:item)* }) => {
        $crate::impl_plugin_base_config!(@inner $plugin, $config, $type_id, $($extra_methods)*);
    };
    // No extra methods.
    ($plugin:ty, $config:ty, $type_id:expr) => {
        $crate::impl_plugin_base_config!(@inner $plugin, $config, $type_id,);
    };
    // Internal rule that generates the impl block.
    (@inner $plugin:ty, $config:ty, $type_id:expr, $($extra_methods:item)*) => {
        impl $crate::PluginBase for $plugin {
            fn plugin_type_id(&self) -> &str {
                $type_id
            }

            fn validate_config(
                &self,
                config: &serde_json::Value,
            ) -> std::result::Result<(), String> {
                let typed: $config = serde_json::from_value(config.clone())
                    .map_err(|e| format!("failed to parse config: {e}"))?;
                typed.validate().map_err(|e| e.to_string())
            }

            fn mask_config_secrets(
                &self,
                config: &serde_json::Value,
            ) -> serde_json::Value {
                let Ok(cfg) = serde_json::from_value::<$config>(config.clone()) else {
                    return config.clone();
                };
                use $crate::SecretMasking;
                match serde_json::to_value(cfg.with_secrets_masked()) {
                    Ok(masked) => masked,
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "failed to serialize masked plugin config"
                        );
                        config.clone()
                    }
                }
            }

            fn restore_config_secrets(
                &self,
                incoming: &serde_json::Value,
                stored: &serde_json::Value,
            ) -> serde_json::Value {
                let (Ok(mut inc), Ok(ex)) = (
                    serde_json::from_value::<$config>(incoming.clone()),
                    serde_json::from_value::<$config>(stored.clone()),
                ) else {
                    return incoming.clone();
                };
                use $crate::SecretMasking;
                inc.restore_secrets_from(&ex);
                serde_json::to_value(&inc).unwrap_or_else(|_| incoming.clone())
            }

            fn form_schema(&self) -> Vec<$crate::form_schema::FieldDef> {
                <$config as $crate::ConfigFormSchema>::form_schema()
            }

            fn type_settings_form_schema(&self) -> Vec<$crate::form_schema::FieldDef> {
                <$config as $crate::ConfigFormSchema>::type_settings_form_schema()
            }

            fn type_settings_sample(&self) -> serde_json::Value {
                <$config as $crate::ConfigFormSchema>::type_settings_sample()
            }

            fn sample_config(&self) -> serde_json::Value {
                serde_json::to_value(<$config>::default())
                    .unwrap_or_else(|_| serde_json::json!({}))
            }

            fn validate_package_identifier(
                &self,
                value: &str,
            ) -> std::result::Result<(), String> {
                <$config>::validate_identifier(value)
            }

            $($extra_methods)*
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal plugin implementing only PluginBase.
    struct StubPlugin;

    #[async_trait]
    impl PluginBase for StubPlugin {
        fn plugin_type_id(&self) -> &str {
            "stub"
        }
    }

    #[test]
    fn stub_has_no_capabilities() {
        let plugin = StubPlugin;
        assert!(plugin.capabilities().is_empty());
        assert!(!plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn stub_accessors_return_none() {
        let plugin = StubPlugin;
        assert!(plugin.as_discovery().is_none());
        assert!(plugin.as_version_detector().is_none());
        assert!(plugin.as_release_fetcher().is_none());
        assert!(plugin.as_package_index().is_none());
        assert!(plugin.as_update_executor().is_none());
        assert!(plugin.as_update_hooks().is_none());
        assert!(plugin.as_notification_transport().is_none());
    }

    #[test]
    fn stub_config_defaults() {
        let plugin = StubPlugin;
        assert!(plugin.validate_config(&serde_json::json!({})).is_ok());
        assert_eq!(
            plugin.mask_config_secrets(&serde_json::json!({"key": "val"})),
            serde_json::json!({"key": "val"})
        );
        assert!(plugin.form_schema().is_empty());
        assert!(plugin.extension_manifests().is_empty());
        assert!(plugin.extension_actions().is_empty());
        assert!(plugin.primary_action_ids().is_empty());
        assert!(plugin.required_sudo_commands().is_empty());
    }

    #[test]
    fn restore_config_secrets_replaces_masked_values() {
        let plugin = StubPlugin;
        let incoming = serde_json::json!({"token": "***", "url": "https://example.com"});
        let stored = serde_json::json!({"token": "secret123", "url": "https://old.com"});
        let result = plugin.restore_config_secrets(&incoming, &stored);
        assert_eq!(
            result,
            serde_json::json!({"token": "secret123", "url": "https://example.com"})
        );
    }

    #[test]
    fn validate_package_identifier_defaults_to_ok() {
        let plugin = StubPlugin;
        assert!(plugin.validate_package_identifier("anything").is_ok());
    }
}
