use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum ApkError {
    #[error("command execution failed with exit code {0}")]
    CommandFailed(i32),
    #[error("output parse error: {0}")]
    ParseOutput(String),
    #[error("package not found: {0}")]
    PackageNotFound(String),
}

pub type Result<T> = std::result::Result<T, Report<ApkError>>;

impl_report_conversion!(PluginError => ApkError, |e| ApkError::ParseOutput(e.to_string()));
impl_report_conversion!(ApkError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
