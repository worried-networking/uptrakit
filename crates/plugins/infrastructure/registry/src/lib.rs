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
    DockerItemHostRequest, DockerSurfaceStore, DockerSwitchTagRequest, EmailSmtpSettings,
    EmailSmtpSettingsPatch, EmailSmtpSettingsStore, GlobalProviderLookup, HostRuntime,
    NotificationActionTokenRecord, NotificationChannelListItem, NotificationChannelListPage,
    NotificationChannelListRequest, NotificationChannelStore, NotificationTransport,
    PluginCapability, PluginCatalog, PluginConfigValidationError, PluginDescriptor, PluginMeta,
    PostUpdateOutcome, ProxmoxApproveMatchRequest, ProxmoxGlobalDefaultsSaveRequest,
    ProxmoxHostInfoRequest, ProxmoxHostMappingRecord, ProxmoxHostMappingsRequest,
    ProxmoxItemOverridePreloadRequest, ProxmoxItemOverrideSaveRequest, ProxmoxManualMatchRequest,
    ProxmoxMappingRequest, ProxmoxPluginConfigRequest, ProxmoxProtectionAuditRecord,
    ProxmoxProtectionMode, ProxmoxProtectionPolicyRecord, ProxmoxProtectionStore,
    ProxmoxScopeSelectionRequest, ProxmoxSurfaceStore, ProxmoxUnmatchedGuestsRequest,
    SoftwareItemCreatedEvent, SoftwareItemLifecycle, SoftwareItemLifecycleContext,
    SoftwareItemPatch, SudoCommandEntry, SudoHelperScript, SurfaceActionController,
    SurfaceActionError, TelegramGlobalSettingsStore, UpdateProtectionController,
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
    BatchDetectItem, BatchFetchItem, BatchFetchResult, BatchUpdateItem, HostCapabilities,
    HostCompatibility, InfraBundle, PluginError, PluginFamily, UpdateLifecycleContext,
    construct_host_runtime,
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
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_approve_match as execute_proxmox_controller_approve_match;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_discover_hosts as execute_proxmox_controller_discover_hosts;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_get_host_info as execute_proxmox_controller_get_host_info;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_list_all_unmatched as execute_proxmox_controller_list_all_unmatched;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_list_host_mappings as execute_proxmox_controller_list_host_mappings;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_load_backup_target_options as execute_proxmox_controller_load_backup_target_options;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_manual_match as execute_proxmox_controller_manual_match;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_preload_global_defaults as execute_proxmox_controller_preload_global_defaults;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_preload_item_overrides as execute_proxmox_controller_preload_item_overrides;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_save_global_defaults as execute_proxmox_controller_save_global_defaults;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_save_item_overrides as execute_proxmox_controller_save_item_overrides;
/// Legacy compatibility export for string-routed Proxmox controller actions.
///
/// Prefer the typed `execute_proxmox_controller_*` functions above for any new
/// call sites.
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_surface_action as execute_proxmox_controller_surface_action;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_test_connection as execute_proxmox_controller_test_connection;
pub use uptrakit_plugin_infrastructure_proxmox::surfaces::execute_controller_unmatch_host as execute_proxmox_controller_unmatch_host;

pub use uptrakit_notification_plugin_core::{DeliveryMessage, MessageAction, escape_html};

/// Build a `PluginCatalog` from all compiled-in descriptors.
///
/// This is the primary entry point for controller startup. The `config`
/// carries deployment-level settings (SSRF policy, shared HTTP client, etc.).
pub fn build_catalog(
    config: &CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<PluginCatalog> {
    PluginCatalog::new(all_descriptors(), config)
}
