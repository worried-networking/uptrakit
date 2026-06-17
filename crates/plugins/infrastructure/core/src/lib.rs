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
#[cfg(feature = "migrations")]
pub(crate) mod db_migrate;
pub mod descriptor;
pub mod error;
pub mod form_schema;
pub mod helpers;
pub mod host_requirements;
pub mod host_runtime;
pub mod service_metadata;
pub use service_metadata::{DeploymentTopology, ServiceMetadata, ServiceMetadataProvider};
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
    InstalledVersionDisplay, InstalledVersionEnricher, InstalledVersionEnrichmentContext,
    InstalledVersionItem, PostUpdateOutcome, SoftwareItemCreatedEvent,
    SoftwareItemLifecycleContext, SoftwareItemPatch,
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
    ControllerUpdateHookOps, ControllerUpdateProtectionOps, NotificationOps, PluginConfigOps,
    PluginMetadataOps, PluginOps, PluginOpsError, PluginSurfaceActionOps, PluginSurfaceOps,
    SoftwareItemLifecycleOps, TransactionalEmailError,
};

// Catalog (feature-gated)
#[cfg(feature = "catalog")]
pub use catalog::{InstancePluginStates, PluginCatalog};

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
#[cfg(feature = "migrations")]
pub use descriptor::PluginTableDescriptor;
pub use descriptor::{
    ApiSubmitDescriptor, CatalogConfig, ConfigModel, ConfigOps, ConfigTestOps,
    CreateControllerProtectionFn, CreateEnhancementFn, CreateReleaseFetcherFn, CreateRoleFn,
    CreateTransportFn, GlobalProviderConsumerDecl, GlobalProviderLookup, InstanceConfigOps,
    PluginDescriptor, PluginFamily, PluginScope, ReleaseFetcherSlot, RoleCreators, RoleSlot,
    SurfaceActionContext, SurfaceActionDescriptor, SurfaceActionError, SurfaceActionHandler,
    SurfaceActionLibrary, SurfaceActionUi, SurfaceFormDescriptor, SurfaceManifest,
    SurfacePlacement, SurfaceRegistrationOps, SurfaceRowCondition, SurfaceRowVisibleWhen,
    SurfaceTableColumn, SurfaceTargeting, SurfaceUiDefinition, SurfaceWorkflowStep,
    TypeSettingsOps,
};
pub use descriptor::{DbMigrateTablesFn, InfraBundle, InfraSlot, MigrationsFn};
pub use form_schema::{
    FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor, FormSelectSourceDescriptor,
};
pub use host_requirements::{HostCompatibilityError, HostRequirements, RoleKey};
pub use roles::ReleaseFetchContext;

// Re-export ConfigTestKind so plugin crates don't need a direct internal-wire dependency
pub use host_runtime::{
    HostRuntime, MetadataAwareHostRuntime, RouterOsExecutor, RouterOsHostRuntime,
    StandardHostRuntime, construct_host_runtime, construct_routeros_host_runtime,
};
pub use plugin_config::{PluginConfig, PluginConfigValidationError, TypeSettings};
#[cfg(feature = "plugin-ops")]
pub use roles::{
    ControllerUpdateHook, UpdateHookController, UpdateHookPostContext, UpdateHookPreContext,
};
pub use roles::{
    ControllerUpdateProtection, Discoverer, ExecuteUpdateResult, LifecycleHook,
    NotificationTransport, PackageIndexer, PluginMeta, ReleaseFetcher, SoftwareItemLifecycle,
    SurfaceActionController, UpdateExecutor, UpdateProtectionController, VersionDetector,
};
#[cfg(feature = "agent-infra")]
pub use roles::{GuestExec, HostLifecycle, HostReport};
pub use uptrakit_shared_types::ConfigTestKind;
pub use uptrakit_surfaces as surfaces;

// Re-export shared-types for convenience
pub use uptrakit_shared_types::{
    HostCapabilities, HostFeature, OsFamily, PluginTypeId, host_features, plugin_ids,
};
