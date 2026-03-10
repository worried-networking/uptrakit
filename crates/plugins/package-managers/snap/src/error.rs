use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum SnapError {
    #[error("command execution failed with exit code {0}")]
    CommandFailed(i32),
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("package not found: {0}")]
    PackageNotFound(String),
}

pub type Result<T> = std::result::Result<T, Report<SnapError>>;

impl_report_conversion!(PluginError => SnapError, |e| SnapError::Configuration(e.to_string()));
impl_report_conversion!(SnapError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
