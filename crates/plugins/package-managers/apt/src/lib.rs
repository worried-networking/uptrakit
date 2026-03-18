pub mod config;
pub mod detection;
pub mod discovery;
pub mod error;
pub mod plugin;
pub mod releases;
pub mod update;

pub use config::{AptConfig, AptDiscoveryFilter};
pub use error::{AptError, Result};
pub use plugin::{AptPlugin, DESCRIPTOR, validate_identifier, validate_version};
