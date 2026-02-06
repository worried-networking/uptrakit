use rootcause::ReportConversion;
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_provider_core::ProviderError;

/// Errors specific to the GitHub Releases provider.
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

    #[error("provider error: {0}")]
    Provider(String),

    #[error("invalid asset pattern: {0}")]
    InvalidPattern(String),
}

/// Result type alias for GitHub provider operations.
pub type Result<T> = std::result::Result<T, Report<GitHubError>>;

impl<T> ReportConversion<reqwest::Error, markers::Mutable, T> for GitHubError
where
    GitHubError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<reqwest::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(|e| GitHubError::Request(e.to_string()))
    }
}

impl<T> ReportConversion<ProviderError, markers::Mutable, T> for GitHubError
where
    GitHubError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<ProviderError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(|e| GitHubError::Provider(e.to_string()))
    }
}
