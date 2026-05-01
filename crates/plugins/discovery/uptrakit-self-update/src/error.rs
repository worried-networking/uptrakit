use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the uptrakit self-update discovery plugin.
#[derive(Debug, Error)]
pub enum SelfUpdateError {
    #[error("metadata provider not available")]
    NoMetadataProvider,

    #[error("binary path not available for UnixBinary topology")]
    NoBinaryPath,

    #[error("pid file not configured for UnixBinary topology with reuseport")]
    NoPidFile,
}

/// Result type alias for self-update plugin operations.
pub type Result<T> = std::result::Result<T, Report<SelfUpdateError>>;

impl_report_conversion!(PluginError => SelfUpdateError, |_e| SelfUpdateError::NoMetadataProvider);
impl_report_conversion!(SelfUpdateError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
