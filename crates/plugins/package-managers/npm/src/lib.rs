pub mod config;
pub mod detection;
pub mod discovery;
pub mod error;
pub mod plugin;
pub mod releases;
pub mod update;

pub use config::NpmConfig;
pub use error::*;
pub use plugin::{NpmPlugin, validate_identifier, validate_version};
