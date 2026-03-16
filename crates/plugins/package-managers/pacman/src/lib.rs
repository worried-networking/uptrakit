pub mod config;
pub mod detection;
pub mod discovery;
pub mod error;
pub mod index;
pub mod plugin;
pub mod releases;
pub mod update;

pub use config::{PacmanConfig, PacmanDiscoveryFilter};
pub use error::{PacmanError, Result};
pub use plugin::{PacmanPlugin, validate_identifier, validate_version};
