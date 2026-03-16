pub mod config;
pub mod detection;
pub mod discovery;
pub mod error;
pub mod plugin;
pub mod releases;
pub mod update;

pub use config::SnapConfig;
pub use error::{Result, SnapError};
pub use plugin::{SnapPlugin, validate_identifier};
