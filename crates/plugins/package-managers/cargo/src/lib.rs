pub mod config;
pub mod error;
pub mod plugin;

pub use config::CargoConfig;
pub use error::{CargoError, Result};
pub use plugin::{CargoPlugin, validate_identifier};
