pub mod config;
pub mod discovery;
pub mod error;
pub mod plugin;

pub use config::ProxmoxHelperScriptsConfig;
pub use error::*;
pub use plugin::{DESCRIPTOR, ProxmoxHelperScriptsPlugin};
