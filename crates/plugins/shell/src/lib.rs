pub mod config;
pub mod error;
pub mod plugin;

pub use config::ShellConfig;
pub use error::{ShellError, Result};
pub use plugin::ShellPlugin;
