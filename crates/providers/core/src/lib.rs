pub mod command;
pub mod error;
pub mod serde_helpers;
pub mod traits;
pub mod types;
pub mod version;

pub use error::{ProviderError, Result};
pub use traits::Provider;
pub use types::{
    DiscoveredSoftware, ProviderCapability, ProviderType, ReleaseAsset, ReleaseInfo, UpdateContext,
    UpstreamRelease,
};
pub use version::Version;

// Re-export command crate types (keeps existing imports working for provider crates)
pub use uptrakit_command::{ShellType, UpdateOutputLine, UpdateOutputStream};
