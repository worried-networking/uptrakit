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
pub mod host_requirements;
pub mod host_runtime;
pub mod macros;
pub mod plugin_base;
pub mod plugin_config;
pub mod plugin_ops;
pub mod roles;
pub mod secrets;
pub mod serde_helpers;
#[cfg(feature = "testing")]
pub mod testing;
pub mod traits;
pub mod types;
pub mod version;

pub use batch_detect::{BatchDetectItem, BatchDetectResult};
pub use batch_fetch::{BatchFetchItem, BatchFetchResult};
pub use batch_update::{BatchUpdateItem, BatchUpdateResult};
pub use error::{PluginError, Result};
pub use form_schema::ConfigFormSchema;
pub use plugin_base::{
    DiscoveryPlugin, NotificationTransportPlugin, PackageIndexPlugin, PluginBase,
    ReleaseFetcherPlugin, SoftwareItemCreatedEvent, SoftwareItemLifecyclePlugin, SoftwareItemPatch,
    UpdateExecutorPlugin, UpdateLifecyclePlugin, VersionDetectorPlugin,
};
#[cfg(feature = "agent-infra")]
pub use plugin_base::{GuestExecPlugin, HostLifecyclePlugin, HostReportPlugin};
pub use secrets::SecretMasking;
pub use traits::{
    HostCompatibility, PreUpdateHookResult, SudoCommandEntry, SudoHelperScript,
    UpdateLifecycleContext,
};
pub use types::{
    AttestationStatus, DiscoveredSoftware, DiscoveryTarget, PluginCapability, PluginRole,
    PluginType, ReleaseAsset, ReleaseInfo, UpdateCategory, UpstreamRelease,
};
pub use version::Version;

// New plugin_ops: always available (no feature gate)
pub use plugin_ops::{
    NotificationOps, PluginConfigOps, PluginExtensionOps, PluginMetadataOps, PluginOps,
    PluginOpsError, SoftwareItemLifecycleOps,
};

// Catalog (feature-gated)
#[cfg(feature = "catalog")]
pub use catalog::PluginCatalog;

// Re-export the shared command-capture helper so plugin crates access it through this crate
pub use command::execute_and_capture;

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
    PluginDescriptor, PluginFamily, RoleCreators, RoleSlot, TypeSettingsOps,
};
pub use descriptor::{InfraBundle, InfraSlot, MigrationsFn};
pub use host_requirements::{HostCompatibilityError, HostRequirements, RoleKey};

// Re-export ConfigTestKind so plugin crates don't need a direct internal-wire dependency
pub use host_runtime::{
    HostRuntime, PosixHostRuntime, construct_host_runtime, require_posix_executor,
};
pub use plugin_config::{PluginConfig, TypeSettings};
pub use roles::{
    Discoverer, LifecycleHook, NotificationTransport, PackageIndexer, PluginMeta, ReleaseFetcher,
    SoftwareItemLifecycle, UpdateExecutor, VersionDetector,
};
#[cfg(feature = "agent-infra")]
pub use roles::{GuestExec, HostLifecycle, HostReport};
pub use uptrakit_internal_wire::ConfigTestKind;

// Re-export shared-types for convenience
pub use uptrakit_shared_types::{
    HostCapabilities, HostFeature, OsFamily, PluginTypeId, plugin_ids,
};
