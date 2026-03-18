pub mod config;
pub mod error;
pub mod plugin;

pub use config::{PkgConfig, PkgDiscoveryFilter};
pub use error::{PkgError, Result};
pub use plugin::{DESCRIPTOR, PkgPlugin, validate_identifier};
