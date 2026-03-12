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

// Re-export agent-infra types when the feature is enabled.
#[cfg(feature = "agent-infra")]
pub use uptrakit_plugin_infrastructure_core::agent_infra::AgentInfraRegistry;

// Re-export notification types when the feature is enabled.
#[cfg(feature = "notifications")]
pub use uptrakit_notification_plugin_registry::NotificationRegistryConfig;

/// Create an [`AgentInfraRegistry`] populated with all known infrastructure plugins.
#[cfg(feature = "agent-infra")]
pub fn create_agent_infra_registry() -> AgentInfraRegistry {
    let mut registry = AgentInfraRegistry::new();
    registry.register(std::sync::Arc::new(
        uptrakit_plugin_infrastructure_proxmox::agent::ProxmoxAgentPlugin::new(),
    ));
    registry
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
            for plugin in self.notification_registry.plugins() {
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
            for plugin in self.notification_registry.plugins() {
                actions.extend(plugin.extension_actions());
            }
        }
        actions
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
        PluginRegistry::handle_extension_action(ctx, extension_id, action_id, params)
    }

    fn notification_plugin(
        &self,
        _channel_type: &str,
    ) -> Option<std::sync::Arc<dyn uptrakit_notification_plugin_core::NotificationPlugin>> {
        #[cfg(feature = "notifications")]
        {
            self.notification_registry.get(_channel_type)
        }
        #[cfg(not(feature = "notifications"))]
        {
            None
        }
    }

    fn notification_supported_types(&self) -> Vec<&'static str> {
        #[cfg(feature = "notifications")]
        {
            self.notification_registry.supported_types()
        }
        #[cfg(not(feature = "notifications"))]
        {
            vec![]
        }
    }

    fn notification_validate_config(
        &self,
        _channel_type: &str,
        _config: &serde_json::Value,
    ) -> uptrakit_notification_plugin_core::Result<()> {
        #[cfg(feature = "notifications")]
        {
            let Some(plugin) = self.notification_registry.get(_channel_type) else {
                return Err(rootcause::report!(
                    uptrakit_notification_plugin_core::NotificationPluginError::InvalidConfig(
                        format!("unknown channel type: {_channel_type}")
                    )
                ));
            };
            plugin.validate_config(_config)
        }
        #[cfg(not(feature = "notifications"))]
        {
            Ok(())
        }
    }

    fn notification_mask_config_secrets(
        &self,
        _channel_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        #[cfg(feature = "notifications")]
        {
            if let Some(plugin) = self.notification_registry.get(_channel_type) {
                return plugin.mask_config_secrets(config);
            }
        }
        config.clone()
    }

    fn notification_restore_config_secrets(
        &self,
        _channel_type: &str,
        incoming: &serde_json::Value,
        _stored: &serde_json::Value,
    ) -> serde_json::Value {
        #[cfg(feature = "notifications")]
        {
            if let Some(plugin) = self.notification_registry.get(_channel_type) {
                return plugin.restore_config_secrets(incoming, _stored);
            }
        }
        incoming.clone()
    }
}
