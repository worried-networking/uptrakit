use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the GitHub Releases plugin.
#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("GitHub API error: {status} {message}")]
    ApiError {
        status: reqwest::StatusCode,
        message: String,
    },

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

    #[error("asset download failed: {0}")]
    DownloadFailed(String),

    #[error("SHA-256 checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("no matching asset found: {0}")]
    NoMatchingAsset(String),

    #[error("multiple assets matched ({count}): {names}")]
    AmbiguousAsset { count: usize, names: String },

    #[error("file operation failed: {0}")]
    FileOperation(String),
}

/// Result type alias for GitHub plugin operations.
pub type Result<T> = std::result::Result<T, Report<GitHubError>>;

impl_report_conversion!(reqwest::Error => GitHubError, |e| GitHubError::Request(e.to_string()));
impl_report_conversion!(PluginError => GitHubError, |e| GitHubError::Configuration(e.to_string()));
impl_report_conversion!(GitHubError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
impl_report_conversion!(std::io::Error => GitHubError, |e| GitHubError::FileOperation(e.to_string()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_api_error() {
        let err = GitHubError::ApiError {
            status: reqwest::StatusCode::NOT_FOUND,
            message: "Not Found".to_string(),
        };
        assert_eq!(err.to_string(), "GitHub API error: 404 Not Found Not Found");
    }

    #[test]
    fn display_rate_limited() {
        let err = GitHubError::RateLimited {
            reset_at: "1234567890".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "GitHub API rate limit exceeded (resets at 1234567890)"
        );
    }

    #[test]
    fn display_configuration() {
        let err = GitHubError::Configuration("invalid owner".to_string());
        assert_eq!(err.to_string(), "configuration error: invalid owner");
    }
}
