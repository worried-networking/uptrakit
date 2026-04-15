#[cfg(feature = "agent-infra")]
pub mod agent_infra;
#[cfg(feature = "http-client")]
pub mod http_client;
#[cfg(feature = "http-client")]
pub use http_client::{PluginHttpClientConfig, SsrfMode, build_plugin_http_client};
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
mod legacy_extension;
pub mod macros;
pub mod plugin_config;
pub mod plugin_ops;
pub mod roles;
pub mod serde_helpers;
pub mod surface_contract;
#[cfg(feature = "testing")]
pub mod testing;
pub mod traits;
pub mod types;
pub mod version;

pub use batch_detect::{BatchDetectItem, BatchDetectResult};
pub use batch_fetch::{BatchFetchItem, BatchFetchResult};
pub use batch_update::{BatchUpdateItem, BatchUpdateResult};
pub use error::{PluginError, Result};
pub use roles::{SoftwareItemCreatedEvent, SoftwareItemLifecycleContext, SoftwareItemPatch};
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
    NotificationOps, PluginConfigOps, PluginExtensionOps, PluginMetadataOps, PluginOps,
    PluginOpsError, PluginSurfaceOps, SoftwareItemLifecycleOps,
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
    CatalogConfig, ConfigModel, ConfigOps, ConfigTestOps, CreateEnhancementFn, CreateRoleFn,
    CreateTransportFn, ExtensionActionContext, ExtensionActionHandler, ExtensionOps,
    PluginDescriptor, PluginFamily, RoleCreators, RoleSlot, SurfaceRegistrationOps,
    TypeSettingsOps,
};
pub use descriptor::{InfraBundle, InfraSlot, MigrationsFn};
pub use host_requirements::{HostCompatibilityError, HostRequirements, RoleKey};
pub use legacy_extension::{
    ActionDef, ActionUi, ApiSubmitDef, ContextSelectorDef, ContextSelectorSource,
    ExtensionManifest, ExtensionPlacement, ExtensionRequestPayload, ExtensionResponsePayload,
    ExtensionTargeting, ExtensionUi, FieldDef, FieldType, FormDef, PanelPosition, RowCondition,
    RowVisibleWhen, SelectOption, SelectSource, TableColumn, WizardStep,
};

// Re-export ConfigTestKind so plugin crates don't need a direct internal-wire dependency
pub use host_runtime::{HostRuntime, StandardHostRuntime, construct_host_runtime};
pub use plugin_config::{PluginConfig, TypeSettings};
pub use roles::{
    Discoverer, LifecycleHook, NotificationTransport, PackageIndexer, PluginMeta, ReleaseFetcher,
    SoftwareItemLifecycle, UpdateExecutor, VersionDetector,
};
#[cfg(feature = "agent-infra")]
pub use roles::{GuestExec, HostLifecycle, HostReport};
pub use surface_contract::build_plugin_surface_registrations_from_extensions;
pub use uptrakit_internal_wire::ConfigTestKind;
pub use uptrakit_internal_wire::surfaces;

// Re-export shared-types for convenience
pub use uptrakit_shared_types::{
    HostCapabilities, HostFeature, OsFamily, PluginTypeId, host_features, plugin_ids,
};
