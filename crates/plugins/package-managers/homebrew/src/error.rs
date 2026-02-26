use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum HomebrewError {
    #[error("command execution failed: {0}")]
    CommandFailed(String),
    #[error("failed to parse brew output: {0}")]
    ParseOutput(String),
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("package not found: {0}")]
    PackageNotFound(String),
}

pub type Result<T> = std::result::Result<T, Report<HomebrewError>>;

impl_report_conversion!(PluginError => HomebrewError, |e| HomebrewError::CommandFailed(e.to_string()));
impl_report_conversion!(HomebrewError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
