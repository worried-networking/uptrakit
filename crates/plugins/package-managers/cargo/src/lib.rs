pub mod config;
pub mod detection;
pub mod error;
pub mod plugin;
pub mod releases;
pub mod update;

pub use config::CargoConfig;
pub use error::{CargoError, Result};
pub use plugin::{CargoPlugin, DESCRIPTOR, validate_identifier};
