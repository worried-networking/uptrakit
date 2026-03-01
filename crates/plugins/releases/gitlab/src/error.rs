use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the GitLab Releases plugin.
#[derive(Debug, Error)]
pub enum GitLabError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("GitLab API error: {status} {message}")]
    ApiError { status: u16, message: String },

    #[error("GitLab API rate limit exceeded")]
    RateLimited,

    #[error("failed to parse GitLab API response: {0}")]
    ParseResponse(String),

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("invalid asset pattern: {0}")]
    InvalidPattern(String),
}

/// Result type alias for GitLab plugin operations.
pub type Result<T> = std::result::Result<T, Report<GitLabError>>;

impl_report_conversion!(reqwest::Error => GitLabError, |e| GitLabError::Request(e.to_string()));
impl_report_conversion!(PluginError => GitLabError, |e| GitLabError::Configuration(e.to_string()));
impl_report_conversion!(GitLabError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
