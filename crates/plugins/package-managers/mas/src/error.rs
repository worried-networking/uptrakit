use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum MasError {
    #[error("command exited with code {0}")]
    CommandFailed(i32),
    #[error("failed to parse mas output: {0}")]
    ParseOutput(String),
    #[error("package not found: {0}")]
    PackageNotFound(String),
}

pub type Result<T> = std::result::Result<T, Report<MasError>>;

impl_report_conversion!(PluginError => MasError, |e| MasError::ParseOutput(e.to_string()));
impl_report_conversion!(MasError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
