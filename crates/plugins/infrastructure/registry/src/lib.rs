//! Plugin Registry for Uptrakit
//!
//! This crate provides the plugin catalog and descriptor assembly:
//!
//! - **Catalog construction**: Build a `PluginCatalog` from all compiled-in descriptors
//! - **Descriptor-based creation**: Create plugin role instances via descriptor function pointers
//! - **Sudo command collection**: Gather sudo requirements from all plugins

#[cfg(feature = "agent-infra")]
pub mod agent_infra;
pub mod error;
pub mod registry;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use error::{PluginRegistryError, Result};
pub use registry::{
    all_descriptors, all_required_sudo_commands, compatible_sudo_commands_for_host, get_descriptor,
    is_interactive_dispatch_plugin, is_package_manager_plugin, plugin_family,
};

// Re-export commonly used types for convenience
pub use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ConfigModel, ControllerPostUpdateContext, ControllerProtectionContext,
    ControllerProtectionDecision, ControllerRuntime, ControllerUpdateProtection,
    GlobalProviderLookup, HostRuntime, MetadataAwareHostRuntime, NotificationTransport,
    PluginCapability, PluginCatalog, PluginConfigValidationError, PluginDescriptor, PluginMeta,
    PostUpdateOutcome, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemLifecycleContext, SoftwareItemPatch, SudoCommandEntry, SudoHelperScript,
    SurfaceActionController, SurfaceActionError, UpdateProtectionController,
};
pub use uptrakit_shared_types::{PluginTypeId, plugin_ids};

// Re-export PluginOps traits
pub use uptrakit_plugin_infrastructure_core::{
    ControllerUpdateProtectionOps, NotificationOps, PluginConfigOps, PluginMetadataOps, PluginOps,
    PluginOpsError, PluginSurfaceActionOps, PluginSurfaceOps, SoftwareItemLifecycleOps,
};

// Re-export descriptor surface-action context (typed controller boundary).
pub use uptrakit_plugin_infrastructure_core::SurfaceActionContext;

// Re-export executor types for downstream convenience
pub use uptrakit_command::{CommandExecutor, LocalCommandExecutor};

// --- Additive re-exports (DESIGN-0001 / ST-0015) ---

pub use uptrakit_plugin_infrastructure_core::host_requirements::RoleKey;
pub use uptrakit_plugin_infrastructure_core::roles::ReleaseFetcher;
pub use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchFetchItem, BatchFetchResult, BatchUpdateItem, ExecuteUpdateResult,
    HostCapabilities, HostCompatibility, InfraBundle, PluginError, PluginFamily,
    ServiceMetadataProvider, UpdateLifecycleContext, construct_host_runtime,
};
pub use uptrakit_plugin_infrastructure_core::{
    FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor, FormSelectSourceDescriptor,
    SurfaceActionDescriptor, SurfaceActionLibrary, SurfaceActionUi, SurfaceFormDescriptor,
    SurfaceRowCondition, SurfaceRowVisibleWhen, SurfaceWorkflowStep,
};

/// Canonical plugin-result alias re-exported by the registry.
///
/// Source: expands to
/// `std::result::Result<T, rootcause::Report<uptrakit_plugin_infrastructure_core::PluginError>>`.
/// The underlying `PluginError` originates in `uptrakit-plugin-infrastructure-core`; the alias
/// itself is defined and owned by `uptrakit-plugin-infrastructure-registry` (this crate).
///
/// Intended usage: downstream consumers should import this alias through the registry-qualified
/// path `uptrakit_plugin_infrastructure_registry::PluginResult` rather than spelling
/// `Result<_, rootcause::Report<PluginError>>` by hand or re-importing `rootcause` and
/// `PluginError` independently. Consumers must not conflate this alias with the crate-local
/// `Result<T>` alias from `error.rs`, which wraps `PluginRegistryError` and is unrelated.
pub type PluginResult<T> = std::result::Result<T, rootcause::Report<PluginError>>;

pub use uptrakit_plugin_infrastructure_core::{
    PluginHttpClientBuildError, PluginHttpClientConfig, SsrfMode, build_plugin_http_client,
};

pub use uptrakit_notification_plugin_core::{
    DeliveryMessage, MessageAction, NotificationPluginError, escape_html,
};

/// Build a `PluginCatalog` from all compiled-in descriptors.
///
/// This is the primary entry point for controller startup. The `config`
/// carries deployment-level settings (SSRF policy, shared HTTP client, etc.).
pub fn build_catalog(
    config: &CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<PluginCatalog> {
    PluginCatalog::new(all_descriptors(), config)
}
