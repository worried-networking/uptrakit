use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the npm package manager plugin.
#[derive(Debug, Error)]
pub enum NpmError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("npm registry error: {status} {message}")]
    ApiError {
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("npm registry rate limit exceeded")]
    RateLimited,

    #[error("failed to parse npm registry response: {0}")]
    ParseResponse(String),

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("invalid package identifier: {0}")]
    InvalidIdentifier(String),

    #[error("invalid version: {0}")]
    InvalidVersion(String),

    #[error("npm command failed with exit code {0}")]
    CommandFailed(i32),

    #[error("package not found: {0}")]
    PackageNotFound(String),
}

/// Result type alias for npm plugin operations.
pub type Result<T> = std::result::Result<T, Report<NpmError>>;

impl_report_conversion!(reqwest::Error => NpmError, |e| NpmError::Request(e.to_string()));
impl_report_conversion!(PluginError => NpmError, |e| NpmError::Configuration(e.to_string()));
impl_report_conversion!(NpmError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
