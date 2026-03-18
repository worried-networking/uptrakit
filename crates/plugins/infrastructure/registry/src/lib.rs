//! Plugin Registry for Uptrakit
//!
//! This crate provides the plugin catalog and descriptor assembly:
//!
//! - **Catalog construction**: Build a `PluginCatalog` from all compiled-in descriptors
//! - **Descriptor-based creation**: Create plugin role instances via descriptor function pointers
//! - **Sudo command collection**: Gather sudo requirements from all plugins

pub mod error;
pub mod registry;

pub use error::{PluginRegistryError, Result};
pub use registry::{
    all_descriptors, all_required_sudo_commands, compatible_sudo_commands_for_host, get_descriptor,
};

// Re-export commonly used types for convenience
pub use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ControllerRuntime, HostRuntime, PluginCapability, PluginCatalog,
    PluginDescriptor, PluginMeta, SoftwareItemCreatedEvent, SoftwareItemPatch, SudoCommandEntry,
    SudoHelperScript,
};
pub use uptrakit_shared_types::{PluginType, PluginTypeId, plugin_ids};

// Re-export PluginOps traits
pub use uptrakit_plugin_infrastructure_core::{
    NotificationOps, PluginConfigOps, PluginExtensionOps, PluginMetadataOps, PluginOps,
    PluginOpsError, SoftwareItemLifecycleOps,
};

// Re-export descriptor's ExtensionActionContext (dyn Any version)
pub use uptrakit_plugin_infrastructure_core::ExtensionActionContext;

// Re-export executor types for downstream convenience
pub use uptrakit_command::{CommandExecutor, LocalCommandExecutor};

/// Build a `PluginCatalog` from all compiled-in descriptors.
///
/// This is the primary entry point for controller startup. The `config`
/// carries deployment-level settings (SSRF policy, shared HTTP client, etc.).
pub fn build_catalog(
    config: &CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<PluginCatalog> {
    PluginCatalog::new(all_descriptors(), config)
}
