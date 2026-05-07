use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RouterOsError {
    #[error("SSH exec failed: {0}")]
    SshExec(String),
    #[error("failed to parse RouterOS output field '{field}' from: {context}")]
    ParseFailure {
        field: &'static str,
        context: String,
    },
    #[error("version not available: {0}")]
    VersionUnavailable(String),
}

pub type Result<T> = std::result::Result<T, Report<RouterOsError>>;

impl_report_conversion!(
    RouterOsError => PluginError,
    |e: RouterOsError| PluginError::PluginInternal(e.to_string())
);
