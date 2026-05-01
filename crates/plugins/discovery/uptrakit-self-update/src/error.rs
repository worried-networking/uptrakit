use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the uptrakit self-update discovery plugin.
#[derive(Debug, Error)]
pub enum SelfUpdateError {
    #[error("metadata provider not available")]
    NoMetadataProvider,

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),
}

/// Result type alias for self-update plugin operations.
pub type Result<T> = std::result::Result<T, Report<SelfUpdateError>>;

impl_report_conversion!(PluginError => SelfUpdateError, |e| SelfUpdateError::Configuration(e.to_string()));
impl_report_conversion!(SelfUpdateError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
