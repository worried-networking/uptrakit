//! Plugin Registry for Uptrakit
//!
//! This crate provides a centralized registry for plugin operations:
//!
//! - **Plugin creation**: Create plugin instances from configuration and executor
//! - **Configuration validation**: Validate plugin-specific configuration JSON
//! - **Secret management**: Mask and restore sensitive fields in configuration
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use uptrakit_plugin_infrastructure_registry::{PluginRegistry, PluginType, LocalCommandExecutor};
//!
//! // Validate configuration (all fields optional — empty config is valid)
//! let config = serde_json::json!({});
//! PluginRegistry::validate_config(PluginType::ReleasesGithub, &config)?;
//!
//! // Create a plugin with a local executor
//! let executor = Arc::new(LocalCommandExecutor);
//! let plugin = PluginRegistry::create_plugin(
//!     PluginType::ReleasesGithub,
//!     &config,
//!     executor,
//! )?;
//!
//! // Fetch releases via subtrait accessor
//! let fetcher = plugin.as_release_fetcher().expect("plugin supports fetching");
//! let releases = fetcher.fetch_releases("octocat/hello-world").await?;
//! ```

pub mod error;
pub mod registry;

pub use error::{PluginRegistryError, Result};
pub use registry::PluginRegistry;

// Re-export commonly used types for plugin crate convenience
pub use uptrakit_plugin_infrastructure_core::{
    PluginBase, PluginCapability, SudoCommandEntry, SudoHelperScript,
};
pub use uptrakit_shared_types::PluginType;

// Re-export executor types for downstream convenience
pub use uptrakit_command::{CommandExecutor, LocalCommandExecutor};

/// Configuration for the notification plugins in the registry.
///
/// Carries deployment-level settings that affect plugin behaviour.
#[cfg(feature = "notifications")]
#[derive(Clone, Debug, Default)]
pub struct NotificationRegistryConfig {
    /// When `true`, the webhook plugin allows URLs pointing to private /
    /// loopback / link-local addresses. Intended for single-tenant or
    /// self-hosted deployments where internal webhook targets are legitimate.
    ///
    /// Default: `false` (private URLs are blocked).
    pub allow_private_urls: bool,
}

/// Returns the exact set of raw settings keys owned by compiled-in notification plugins.
///
/// Notification plugins store their configuration in the shared `settings` and
/// `global_settings` DB tables via the raw-key settings store
/// (`uptrakit_shared_db::raw_settings`). Because these keys are not part of the
/// typed `SettingKey` enum, the controller would otherwise log a spurious
/// "unrecognised setting key" warning for each one on startup.
///
/// This function returns the complete list of exact raw keys that are legitimately
/// owned by compiled-in notification plugins. The set is determined at compile
/// time by the active feature flags — enabling a new notification plugin
/// automatically contributes its keys here via its `extensions::RAW_SETTINGS_KEYS`
/// constant.
///
/// The slice is computed once and cached in a [`std::sync::OnceLock`].
///
/// # Adding a new notification plugin
///
/// Declare `pub const RAW_SETTINGS_KEYS: &[&str]` in the plugin's `extensions`
/// module, then add a `#[cfg(feature = "notifications-<name>")]` extend call
/// below.
pub fn all_plugin_raw_settings_keys() -> Vec<&'static str> {
    // `mut` is only exercised when at least one notification plugin feature is
    // enabled (`notifications-email` / `notifications-telegram`). The suppression
    // is required because additive feature flags prohibit `#[cfg(not(feature))]`.
    #[allow(unused_mut)]
    let mut keys: Vec<&'static str> = Vec::new();
    #[cfg(feature = "notifications-email")]
    keys.extend_from_slice(uptrakit_notification_plugin_email::extensions::RAW_SETTINGS_KEYS);
    #[cfg(feature = "notifications-telegram")]
    keys.extend_from_slice(uptrakit_notification_plugin_telegram::extensions::RAW_SETTINGS_KEYS);
    keys
}

/// Return all controller-side database migrations contributed by plugins.
///
/// The controller's migration runner appends these after the core migrations
/// from `crates/shared/db` so that plugin-owned tables are created after the
/// core schema is in place. Each migration has a unique name tracked in
/// `seaql_migrations`, so already-applied migrations are skipped.
#[cfg(feature = "migrations")]
pub fn all_controller_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    let mut migrations = Vec::new();
    migrations.extend(uptrakit_plugin_infrastructure_proxmox::controller_migration::migrations());
    migrations
}

/// Create a list of all known agent-side infrastructure plugins.
///
/// Returns `Arc<dyn PluginBase>` instances that implement the infrastructure
/// subtraits (`HostLifecyclePlugin`, `HostReportPlugin`, `GuestExecPlugin`).
/// The agent uses subtrait accessors (e.g. `as_host_lifecycle()`) to call
/// specific plugin hooks.
#[cfg(feature = "agent-infra")]
pub fn create_agent_infra_plugins()
-> Vec<std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::PluginBase>> {
    vec![std::sync::Arc::new(
        uptrakit_plugin_infrastructure_proxmox::agent::ProxmoxAgentPlugin::new(),
    )]
}

// Re-export `PluginOps` trait and associated types from `infrastructure-core`.
// The trait definition lives in `core` so that lightweight consumers (e.g.
// `web-api-queries`) can depend on `core` alone without pulling in all plugin
// crate implementations.
pub use uptrakit_plugin_infrastructure_core::plugin_ops::{
    ExtensionActionContext, PluginOps, PluginOpsError,
};
/// Result type for [`PluginOps`] trait methods.
///
/// Re-exported from `infrastructure-core` for convenience.
pub type PluginOpsResult<T> = std::result::Result<T, rootcause::Report<PluginOpsError>>;

impl PluginOps for PluginRegistry {
    fn validate_config_str(
        &self,
        plugin_type: &str,
        config: &serde_json::Value,
    ) -> uptrakit_plugin_infrastructure_core::plugin_ops::Result<()> {
        PluginRegistry::validate_config_str(plugin_type, config).map_err(|e| {
            rootcause::report!(
                uptrakit_plugin_infrastructure_core::PluginOpsError::ConfigValidation(
                    e.to_string()
                )
            )
        })
    }

    fn mask_config_secrets_str(
        &self,
        plugin_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        PluginRegistry::mask_config_secrets_str(plugin_type, config)
    }

    fn restore_config_secrets_str(
        &self,
        plugin_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        PluginRegistry::restore_config_secrets_str(plugin_type, incoming, existing);
    }

    fn known_plugin_types(&self) -> Vec<PluginType> {
        PluginRegistry::known_plugin_types()
    }

    fn discovery_plugins(&self) -> Vec<PluginType> {
        PluginRegistry::discovery_plugins()
    }

    fn validate_package_identifier_str(
        &self,
        plugin_type: &str,
        value: &str,
    ) -> std::result::Result<(), String> {
        let Ok(pt) = plugin_type.parse::<PluginType>() else {
            return Err(format!("unknown plugin type: {plugin_type}"));
        };
        PluginRegistry::validate_package_identifier(pt, value)
    }

    fn capabilities_for_str(&self, plugin_type: &str) -> Vec<PluginCapability> {
        PluginRegistry::capabilities_for_str(plugin_type)
    }

    fn sample_config_for_str(&self, plugin_type: &str) -> serde_json::Value {
        PluginRegistry::sample_config_str(plugin_type)
    }

    fn config_form_schema_str(
        &self,
        plugin_type: &str,
    ) -> Option<Vec<uptrakit_extension_framework::FieldDef>> {
        PluginRegistry::config_form_schema_str(plugin_type)
    }

    fn type_settings_form_schema_str(
        &self,
        plugin_type: &str,
    ) -> Option<Vec<uptrakit_extension_framework::FieldDef>> {
        PluginRegistry::type_settings_form_schema_str(plugin_type)
    }

    fn type_settings_sample_for_str(&self, plugin_type: &str) -> serde_json::Value {
        PluginRegistry::type_settings_sample_str(plugin_type)
    }

    fn extension_manifests(&self) -> Vec<uptrakit_extension_framework::ExtensionManifest> {
        #[allow(unused_mut)]
        let mut manifests =
            uptrakit_plugin_infrastructure_proxmox::extensions::extension_manifests();
        #[cfg(feature = "notifications")]
        {
            for plugin in self.notification_plugins.values() {
                manifests.extend(plugin.extension_manifests());
            }
        }
        manifests
    }

    fn extension_actions(&self) -> Vec<uptrakit_extension_framework::ActionDef> {
        #[allow(unused_mut)]
        let mut actions = uptrakit_plugin_infrastructure_proxmox::extensions::extension_actions();
        #[cfg(feature = "notifications")]
        {
            for plugin in self.notification_plugins.values() {
                actions.extend(plugin.extension_actions());
            }
        }
        actions
    }

    fn extension_manifests_and_actions(
        &self,
    ) -> Vec<(
        uptrakit_extension_framework::ExtensionManifest,
        Vec<uptrakit_extension_framework::ActionDef>,
    )> {
        let mut result = Vec::new();

        // Proxmox plugin: pair each proxmox manifest with proxmox-specific actions.
        let proxmox_manifests =
            uptrakit_plugin_infrastructure_proxmox::extensions::extension_manifests();
        let proxmox_actions =
            uptrakit_plugin_infrastructure_proxmox::extensions::extension_actions();
        for manifest in proxmox_manifests {
            result.push((manifest, proxmox_actions.clone()));
        }

        // Notification plugins: pair each plugin's manifests with that plugin's
        // own actions so that `resolveAction("create")` on one notification
        // extension (e.g. telegram) does not return another plugin's action
        // (e.g. webhook's "Add Webhook").
        #[cfg(feature = "notifications")]
        {
            for plugin in self.notification_plugins.values() {
                let plugin_manifests = plugin.extension_manifests();
                let plugin_actions = plugin.extension_actions();
                for manifest in plugin_manifests {
                    result.push((manifest, plugin_actions.clone()));
                }
            }
        }

        result
    }

    fn handle_extension_action<'a>(
        &'a self,
        ctx: &'a ExtensionActionContext<'a>,
        extension_id: &'a str,
        action_id: &'a str,
        params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<serde_json::Value, String>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            // First try the macro-generated dispatch (proxmox, etc.)
            let result = PluginRegistry::handle_extension_action(
                ctx,
                extension_id,
                action_id,
                params.clone(),
            )
            .await;
            if result.is_ok()
                || !result
                    .as_ref()
                    .is_err_and(|e| e.starts_with("no plugin handles extension"))
            {
                return result;
            }

            // Then check notification plugins by extension prefix
            #[cfg(feature = "notifications")]
            {
                #[cfg(feature = "notifications-webhook")]
                if extension_id.starts_with("notifications.webhook") {
                    return uptrakit_notification_plugin_webhook::extensions::handle_action(
                        ctx,
                        extension_id,
                        action_id,
                        params,
                    )
                    .await;
                }

                #[cfg(feature = "notifications-telegram")]
                if extension_id.starts_with("notifications.telegram") {
                    return uptrakit_notification_plugin_telegram::extensions::handle_action(
                        ctx,
                        extension_id,
                        action_id,
                        params,
                    )
                    .await;
                }

                #[cfg(feature = "notifications-email")]
                if extension_id.starts_with("notifications.email") {
                    return uptrakit_notification_plugin_email::extensions::handle_action(
                        ctx,
                        extension_id,
                        action_id,
                        params,
                    )
                    .await;
                }
            }

            Err(format!("no plugin handles extension '{extension_id}'"))
        })
    }

    fn notification_transport(
        &self,
        // Only used by the `#[cfg(feature = "notifications")]` path below
        // (`self.notification_plugin_ref()`). Cannot be removed: defined by `PluginOps` trait.
        #[allow(unused_variables)] channel_type: &str,
    ) -> Option<std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::PluginBase>> {
        #[cfg(feature = "notifications")]
        {
            self.notification_plugin_ref(channel_type).cloned()
        }
        #[cfg(not(feature = "notifications"))]
        {
            None
        }
    }

    fn notification_supported_types(&self) -> Vec<&'static str> {
        #[cfg(feature = "notifications")]
        {
            self.notification_plugins.keys().copied().collect()
        }
        #[cfg(not(feature = "notifications"))]
        {
            vec![]
        }
    }

    fn notification_validate_config(
        &self,
        // Only used by the `#[cfg(feature = "notifications")]` path below.
        // Cannot be removed: defined by `PluginOps` trait.
        #[allow(unused_variables)] channel_type: &str,
        // Only used by the `#[cfg(feature = "notifications")]` path below.
        // Cannot be removed: defined by `PluginOps` trait.
        #[allow(unused_variables)] config: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        #[cfg(feature = "notifications")]
        {
            let Some(plugin) = self.notification_plugin_ref(channel_type) else {
                return Err(format!("unknown channel type: {channel_type}"));
            };
            plugin.validate_config(config)
        }
        #[cfg(not(feature = "notifications"))]
        {
            Ok(())
        }
    }

    fn notification_mask_config_secrets(
        &self,
        // Only used by the `#[cfg(feature = "notifications")]` path below.
        // Cannot be removed: defined by `PluginOps` trait.
        #[allow(unused_variables)] channel_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        #[cfg(feature = "notifications")]
        {
            if let Some(plugin) = self.notification_plugin_ref(channel_type) {
                return plugin.mask_config_secrets(config);
            }
        }
        config.clone()
    }

    fn notification_restore_config_secrets(
        &self,
        // Only used by the `#[cfg(feature = "notifications")]` path below.
        // Cannot be removed: defined by `PluginOps` trait.
        #[allow(unused_variables)] channel_type: &str,
        incoming: &serde_json::Value,
        // Only used by the `#[cfg(feature = "notifications")]` path below.
        // Cannot be removed: defined by `PluginOps` trait.
        #[allow(unused_variables)] stored: &serde_json::Value,
    ) -> serde_json::Value {
        #[cfg(feature = "notifications")]
        {
            if let Some(plugin) = self.notification_plugin_ref(channel_type) {
                return plugin.restore_config_secrets(incoming, stored);
            }
        }
        incoming.clone()
    }
}
