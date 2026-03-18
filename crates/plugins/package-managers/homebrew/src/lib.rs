pub mod config;
pub mod detection;
pub mod discovery;
pub mod error;
pub mod plugin;
pub mod releases;
pub mod update;

pub use config::{HomebrewConfig, HomebrewPackageType};
pub use error::{HomebrewError, Result};
pub use plugin::{DESCRIPTOR, HomebrewPlugin, validate_identifier};
