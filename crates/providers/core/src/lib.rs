pub mod error;
pub mod serde_helpers;
pub mod traits;
pub mod types;
pub mod version;

pub use error::{ProviderError, Result};
pub use traits::{LocalProvider, RemoteProvider};
pub use types::{DiscoveredSoftware, ProviderType, ReleaseAsset, UpstreamRelease};
pub use version::Version;
