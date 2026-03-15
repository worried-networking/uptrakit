#[cfg(feature = "agent-infra")]
pub mod agent_infra;
#[cfg(feature = "http-client")]
pub mod http_client;
#[cfg(feature = "http-client")]
pub use http_client::{PluginHttpClientConfig, SsrfMode, build_plugin_http_client};
pub mod batch_detect;
pub mod batch_fetch;
pub mod batch_update;
pub mod command;
pub mod error;
pub mod form_schema;
pub mod plugin_base;
#[cfg(feature = "plugin-ops")]
pub mod plugin_ops;
pub mod secrets;
pub mod serde_helpers;
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
    ReleaseFetcherPlugin, UpdateExecutorPlugin, UpdateLifecyclePlugin, VersionDetectorPlugin,
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

#[cfg(feature = "plugin-ops")]
pub use plugin_ops::{ExtensionActionContext, PluginOps, PluginOpsError};

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
///
/// Prefer this alias over `mpsc::Sender<UpdateOutputLine>` directly so that
/// plugin code remains decoupled from the concrete channel implementation.
pub type UpdateOutputSender = mpsc::Sender<UpdateOutputLine>;

/// Typed receiver for consuming update output lines produced by the executor.
///
/// Prefer this alias over `mpsc::Receiver<UpdateOutputLine>` directly.
pub type UpdateOutputReceiver = mpsc::Receiver<UpdateOutputLine>;
