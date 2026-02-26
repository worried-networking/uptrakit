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
//! // Fetch releases (owner/repo is the package_identifier, not config)
//! let releases = plugin.fetch_releases("octocat/hello-world").await?;
//! ```

pub mod error;
pub mod registry;

pub use error::{PluginRegistryError, Result};
pub use registry::PluginRegistry;

// Re-export commonly used types for plugin crate convenience
pub use uptrakit_plugin_infrastructure_core::{Plugin, PluginCapability, SudoCommandEntry};
pub use uptrakit_shared_types::PluginType;

// Re-export executor types for downstream convenience
pub use uptrakit_command::{CommandExecutor, LocalCommandExecutor};

/// Abstraction over the plugin registry operations needed by the web API.
///
/// Defines the three operations used when persisting and returning plugin
/// configurations over the REST API: config validation, secret masking for
/// API responses, and secret restoration on update. Implemented by
/// [`PluginRegistry`].
///
/// Storing this trait in `AppState` as `Arc<dyn PluginOps>` rather than
/// referencing `PluginRegistry` directly decouples route handlers and query
/// helpers from the concrete registry, making them testable in isolation.
pub trait PluginOps: Send + Sync + 'static {
    /// Validate plugin configuration JSON for the given string plugin type.
    fn validate_config_str(&self, plugin_type: &str, config: &serde_json::Value) -> Result<()>;

    /// Mask secrets in plugin configuration JSON for an API response.
    ///
    /// Returns the config with all secret fields replaced by `"***"`.
    /// Unknown plugin types are returned unchanged.
    fn mask_config_secrets_str(
        &self,
        plugin_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value;

    /// Restore masked secrets from an existing configuration.
    ///
    /// Fields in `incoming` that equal `"***"` are replaced with the
    /// corresponding values from `existing`. Non-masked fields are left
    /// untouched.
    fn restore_config_secrets_str(
        &self,
        plugin_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    );

    /// Returns all plugin types that have the `DiscoverLocalSoftware` capability.
    fn discovery_plugins(&self) -> Vec<PluginType>;

    /// Validate a package identifier for the given string plugin type.
    ///
    /// Returns `Ok(())` for unknown plugin types (no constraints apply) and for
    /// plugin types that impose no identifier constraints. Returns `Err(message)`
    /// when the identifier violates plugin-specific rules.
    fn validate_package_identifier_str(
        &self,
        plugin_type: &str,
        value: &str,
    ) -> std::result::Result<(), String>;
}

impl PluginOps for PluginRegistry {
    fn validate_config_str(&self, plugin_type: &str, config: &serde_json::Value) -> Result<()> {
        PluginRegistry::validate_config_str(plugin_type, config)
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

    fn discovery_plugins(&self) -> Vec<PluginType> {
        PluginRegistry::discovery_plugins()
    }

    fn validate_package_identifier_str(
        &self,
        plugin_type: &str,
        value: &str,
    ) -> std::result::Result<(), String> {
        let Ok(pt) = plugin_type.parse::<PluginType>() else {
            return Ok(());
        };
        PluginRegistry::validate_package_identifier(pt, value)
    }
}
