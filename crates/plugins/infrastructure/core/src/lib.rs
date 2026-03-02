pub mod batch_update;
pub mod command;
pub mod error;
pub mod secrets;
pub mod serde_helpers;
pub mod traits;
pub mod types;
pub mod version;

pub use batch_update::{BatchUpdateItem, BatchUpdateResult};
pub use error::{PluginError, Result};
pub use secrets::SecretMasking;
pub use traits::{
    HostCompatibility, Plugin, PreUpdateHookResult, SudoCommandEntry, SudoHelperScript,
    UpdateHookContext,
};
pub use types::{
    DiscoveredSoftware, DiscoveryTarget, PluginCapability, PluginRole, PluginType, ReleaseAsset,
    ReleaseInfo, TrackingSystem, UpdateCategory, UpstreamRelease,
};
pub use version::Version;

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
