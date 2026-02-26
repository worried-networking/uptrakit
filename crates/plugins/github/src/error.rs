use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the GitHub Releases plugin.
#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("GitHub API error: {status} {message}")]
    ApiError { status: u16, message: String },

    #[error("GitHub API rate limit exceeded (resets at {reset_at})")]
    RateLimited { reset_at: String },

    #[error("failed to parse GitHub API response: {0}")]
    ParseResponse(String),

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("invalid asset pattern: {0}")]
    InvalidPattern(String),
}

/// Result type alias for GitHub plugin operations.
pub type Result<T> = std::result::Result<T, Report<GitHubError>>;

impl_report_conversion!(reqwest::Error => GitHubError, |e| GitHubError::Request(e.to_string()));
impl_report_conversion!(PluginError => GitHubError, |e| GitHubError::Configuration(e.to_string()));
impl_report_conversion!(GitHubError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
