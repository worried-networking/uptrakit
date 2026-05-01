#[cfg(feature = "agent-infra")]
pub mod agent_infra;
#[cfg(feature = "http-client")]
pub mod http_client;
#[cfg(feature = "http-client")]
pub use http_client::{
    PluginHttpClientBuildError, PluginHttpClientConfig, SsrfMode, build_plugin_http_client,
};
pub mod batch_detect;
pub mod batch_fetch;
pub mod batch_update;
#[cfg(feature = "catalog")]
pub mod catalog;
pub mod command;
pub mod descriptor;
pub mod error;
pub mod form_schema;
pub mod helpers;
pub mod host_requirements;
pub mod host_runtime;
pub mod macros;
pub mod plugin_config;
pub mod plugin_ops;
pub mod roles;
pub mod serde_helpers;
mod surface_form_authoring;
#[cfg(feature = "testing")]
pub mod testing;
pub mod traits;
pub mod types;
pub mod version;

pub use batch_detect::{BatchDetectItem, BatchDetectResult};
pub use batch_fetch::{BatchFetchItem, BatchFetchResult};
pub use batch_update::{BatchUpdateItem, BatchUpdateResult};
pub use error::{PluginError, Result};
pub use roles::{
    ControllerPostUpdateContext, ControllerProtectionContext, ControllerProtectionDecision,
    PostUpdateOutcome, SoftwareItemCreatedEvent, SoftwareItemLifecycleContext, SoftwareItemPatch,
};
pub use traits::{
    HostCompatibility, PreUpdateHookResult, SudoCommandEntry, SudoHelperScript,
    UpdateLifecycleContext,
};
pub use types::{
    AttestationStatus, DiscoveredSoftware, DiscoveryTarget, PluginCapability, PluginRole,
    ReleaseAsset, ReleaseInfo, UpdateCategory, UpstreamRelease,
};
pub use version::Version;

// New plugin_ops: always available (no feature gate)
pub use plugin_ops::{
    ControllerUpdateProtectionOps, NotificationOps, PluginConfigOps, PluginMetadataOps, PluginOps,
    PluginOpsError, PluginSurfaceActionOps, PluginSurfaceOps, SoftwareItemLifecycleOps,
};

// Catalog (feature-gated)
#[cfg(feature = "catalog")]
pub use catalog::PluginCatalog;

// Re-export the shared command-capture helper so plugin crates access it through this crate
pub use command::execute_and_capture;

// Re-export shared package-manager helpers
pub use helpers::{
    BatchNamesParams, BatchVersionedParams, CommandUpdateParams, ValidatorFn,
    execute_batch_names_command, execute_batch_versioned_command, execute_command_update,
    refresh_package_index_command, require_package_identifier,
};

// Re-export command crate types (keeps existing imports working for plugin crates)
pub use uptrakit_command::UpdateOutputLine;

// Re-export shared-types enums used by plugins
pub use uptrakit_shared_types::{HookShell, OutputStreamType};

// Re-export executor types for plugin crate convenience
pub use uptrakit_command::{
    CommandExecutor, CommandMode, CommandOutput, CommandSpec, LocalCommandExecutor,
    SudoAwareCommandExecutor, SudoContext, SudoPolicy,
};

// Re-export SecretString so plugin crates use it via plugin-core
pub use uptrakit_shared_types::SecretString;

// Re-export tokio::sync::mpsc so plugin crates don't need a direct tokio dependency
pub use tokio::sync::mpsc;

/// Typed sender for streaming update output lines to the executor.
pub type UpdateOutputSender = mpsc::Sender<UpdateOutputLine>;

/// Typed receiver for consuming update output lines produced by the executor.
pub type UpdateOutputReceiver = mpsc::Receiver<UpdateOutputLine>;

// ── New framework re-exports ────────────────────────────────────────────────

#[cfg(feature = "catalog")]
pub use descriptor::ControllerRuntime;
pub use descriptor::{
    ApiSubmitDescriptor, CatalogConfig, ConfigModel, ConfigOps, ConfigTestOps,
    CreateControllerProtectionFn, CreateEnhancementFn, CreateRoleFn, CreateTransportFn,
    GlobalProviderConsumerDecl, GlobalProviderLookup, PluginDescriptor, PluginFamily, RoleCreators,
    RoleSlot, SurfaceActionContext, SurfaceActionDescriptor, SurfaceActionError,
    SurfaceActionHandler, SurfaceActionLibrary, SurfaceActionUi, SurfaceFormDescriptor,
    SurfaceManifest, SurfacePlacement, SurfaceRegistrationOps, SurfaceRowCondition,
    SurfaceRowVisibleWhen, SurfaceTableColumn, SurfaceTargeting, SurfaceUiDefinition,
    SurfaceWorkflowStep, TypeSettingsOps,
};
pub use descriptor::{InfraBundle, InfraSlot, MigrationsFn};
pub use form_schema::{
    FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor, FormSelectSourceDescriptor,
};
pub use host_requirements::{HostCompatibilityError, HostRequirements, RoleKey};

// Re-export ConfigTestKind so plugin crates don't need a direct internal-wire dependency
pub use host_runtime::{HostRuntime, StandardHostRuntime, construct_host_runtime};
pub use plugin_config::{PluginConfig, PluginConfigValidationError, TypeSettings};
pub use roles::ExecuteUpdateResult;
pub use roles::{
    ControllerUpdateProtection, Discoverer, DockerItemHostRequest, DockerSurfaceStore,
    DockerSwitchTagRequest, EmailSmtpSettings, EmailSmtpSettingsPatch, EmailSmtpSettingsStore,
    LifecycleHook, NotificationActionTokenRecord, NotificationChannelListItem,
    NotificationChannelListPage, NotificationChannelListRequest, NotificationChannelStore,
    NotificationTransport, PackageIndexer, PluginMeta, ProxmoxApproveMatchRequest,
    ProxmoxGlobalDefaultsSaveRequest, ProxmoxHostInfoRequest, ProxmoxHostMappingRecord,
    ProxmoxHostMappingsRequest, ProxmoxItemOverridePreloadRequest, ProxmoxItemOverrideSaveRequest,
    ProxmoxManualMatchRequest, ProxmoxMappingRequest, ProxmoxPluginConfigRequest,
    ProxmoxProtectionAuditRecord, ProxmoxProtectionMode, ProxmoxProtectionPolicyRecord,
    ProxmoxProtectionStore, ProxmoxScopeSelectionRequest, ProxmoxSurfaceStore,
    ProxmoxUnmatchedGuestsRequest, ReleaseFetcher, SoftwareItemLifecycle, SurfaceActionController,
    TelegramGlobalSettingsStore, UpdateExecutor, UpdateProtectionController, VersionDetector,
};
#[cfg(feature = "agent-infra")]
pub use roles::{GuestExec, HostLifecycle, HostReport};
pub use uptrakit_shared_types::ConfigTestKind;
pub use uptrakit_surfaces as surfaces;

// Re-export shared-types for convenience
pub use uptrakit_shared_types::{
    HostCapabilities, HostFeature, OsFamily, PluginTypeId, host_features, plugin_ids,
};

#[cfg(test)]
#[test]
fn plugin_config_validation_error_formats_for_display() {
    let err = PluginConfigValidationError::invalid_field("url", "must be https");
    assert_eq!(err.field(), Some("url"));
    assert_eq!(err.to_string(), "url: must be https");
}
