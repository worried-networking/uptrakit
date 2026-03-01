use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the Forgejo Releases plugin.
#[derive(Debug, Error)]
pub enum ForgejoError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("Forgejo API error: {status} {message}")]
    ApiError { status: u16, message: String },

    #[error("Forgejo API rate limit exceeded")]
    RateLimited,

    #[error("failed to parse Forgejo API response: {0}")]
    ParseResponse(String),

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("invalid asset pattern: {0}")]
    InvalidPattern(String),
}

/// Result type alias for Forgejo plugin operations.
pub type Result<T> = std::result::Result<T, Report<ForgejoError>>;

impl_report_conversion!(reqwest::Error => ForgejoError, |e| ForgejoError::Request(e.to_string()));
impl_report_conversion!(PluginError => ForgejoError, |e| ForgejoError::Configuration(e.to_string()));
impl_report_conversion!(ForgejoError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
