pub mod config;
pub mod error;
pub mod plugin;

pub use config::ShellConfig;
pub use error::{Result, ShellError};
pub use plugin::ShellPlugin;
