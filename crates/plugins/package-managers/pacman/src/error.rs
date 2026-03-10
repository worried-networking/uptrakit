use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum PacmanError {
    #[error("command execution failed with exit code {0}")]
    CommandFailed(i32),
    #[error("configuration error: {0}")]
    Configuration(String),
}

pub type Result<T> = std::result::Result<T, Report<PacmanError>>;

impl_report_conversion!(PluginError => PacmanError, |e| PacmanError::Configuration(e.to_string()));
impl_report_conversion!(PacmanError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
