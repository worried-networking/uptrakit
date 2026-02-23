pub mod command;
pub mod error;
pub mod secrets;
pub mod serde_helpers;
pub mod traits;
pub mod types;
pub mod version;

pub use error::{ProviderError, Result};
pub use secrets::SecretMasking;
pub use traits::Provider;
pub use types::{
    DiscoveredSoftware, ProviderCapability, ProviderType, ReleaseAsset, ReleaseInfo,
    UpstreamRelease,
};
pub use version::Version;

// Re-export command crate types (keeps existing imports working for provider crates)
pub use uptrakit_command::UpdateOutputLine;

// Re-export shared-types enums used by providers
pub use uptrakit_shared_types::{HookShell, OutputStreamType};

// Re-export executor types for provider crate convenience
pub use uptrakit_command::{
    CommandExecutor, CommandMode, CommandOutput, CommandSpec, LocalCommandExecutor,
};

// Re-export SecretString so provider crates use it via provider-core
pub use uptrakit_shared_types::SecretString;

// Re-export tokio::sync::mpsc so provider crates don't need a direct tokio dependency
pub use tokio::sync::mpsc;

/// Typed sender for streaming update output lines to the executor.
///
/// Prefer this alias over `mpsc::Sender<UpdateOutputLine>` directly so that
/// provider code remains decoupled from the concrete channel implementation.
pub type UpdateOutputSender = mpsc::Sender<UpdateOutputLine>;

/// Typed receiver for consuming update output lines produced by the executor.
///
/// Prefer this alias over `mpsc::Receiver<UpdateOutputLine>` directly.
pub type UpdateOutputReceiver = mpsc::Receiver<UpdateOutputLine>;
