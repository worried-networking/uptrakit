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
pub use uptrakit_command::{ShellType, UpdateOutputLine, UpdateOutputStream};

// Re-export SecretString so provider crates use it via provider-core
pub use uptrakit_shared_types::SecretString;
