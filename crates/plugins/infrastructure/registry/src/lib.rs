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
pub use uptrakit_plugin_infrastructure_core::{
    Plugin, PluginCapability, SudoCommandEntry, SudoHelperScript,
};
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

    /// Returns all plugin types registered in the registry.
    ///
    /// This is the authoritative list — no hardcoded lists should exist outside
    /// the registry. Use this to populate plugin-type selectors dynamically.
    fn known_plugin_types(&self) -> Vec<PluginType>;

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

    /// Returns the capabilities declared by the given plugin type.
    ///
    /// Returns an empty vec for unknown plugin types.
    fn capabilities_for_str(&self, plugin_type: &str) -> Vec<PluginCapability>;

    /// Returns a sample/default configuration JSON for the given plugin type string.
    ///
    /// Serializes the `Default` implementation of the plugin's config type.
    /// Returns an empty JSON object `{}` for unknown plugin types.
    fn sample_config_for_str(&self, plugin_type: &str) -> serde_json::Value;

    /// Returns UI extension manifests provided by all registered plugins.
    ///
    /// Default returns empty — no plugin provides extensions yet. Override
    /// when a plugin declares compile-time UI extensions.
    fn extension_manifests(&self) -> Vec<uptrakit_internal_wire::extension::ExtensionManifest> {
        vec![]
    }

    /// Returns the action library for all registered plugins.
    ///
    /// Actions are referenced by `action_id` from the extension manifests.
    /// Default returns empty.
    fn extension_actions(&self) -> Vec<uptrakit_internal_wire::extension::ActionDef> {
        vec![]
    }

    /// Handle an extension action invocation for a plugin-backed extension.
    ///
    /// The controller calls this when an action is invoked on an extension
    /// owned by `ExtensionOwner::Plugin`. The plugin registry dispatches to
    /// the appropriate plugin based on the extension ID prefix.
    ///
    /// Returns `Ok(json)` on success or `Err(message)` on failure.
    /// The route handler maps these to HTTP 200/422 respectively.
    fn handle_extension_action<'a>(
        &'a self,
        _ctx: &'a ExtensionActionContext<'a>,
        _extension_id: &'a str,
        _action_id: &'a str,
        _params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<serde_json::Value, String>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err("plugin-backed extension actions not supported".to_string()) })
    }
}

/// Context passed to plugin extension action handlers.
///
/// Provides access to the database connection and tenant/user context
/// from the authenticated HTTP request.
pub struct ExtensionActionContext<'a> {
    /// Database connection for queries.
    pub db: &'a sea_orm::DatabaseConnection,
    /// Tenant ID from the authenticated request (if available).
    pub tenant_id: Option<uuid::Uuid>,
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
            return Ok(());
        };
        PluginRegistry::validate_package_identifier(pt, value)
    }

    fn capabilities_for_str(&self, plugin_type: &str) -> Vec<PluginCapability> {
        PluginRegistry::capabilities_for_str(plugin_type)
    }

    fn sample_config_for_str(&self, plugin_type: &str) -> serde_json::Value {
        PluginRegistry::sample_config_str(plugin_type)
    }

    fn extension_manifests(&self) -> Vec<uptrakit_internal_wire::extension::ExtensionManifest> {
        uptrakit_plugin_infrastructure_proxmox::extensions::extension_manifests()
    }

    fn extension_actions(&self) -> Vec<uptrakit_internal_wire::extension::ActionDef> {
        uptrakit_plugin_infrastructure_proxmox::extensions::extension_actions()
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
}
