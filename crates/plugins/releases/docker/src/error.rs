use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the Docker plugin.
#[derive(Debug, Error)]
pub enum DockerError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("registry API error: {status} {message}")]
    ApiError {
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("failed to parse registry API response: {0}")]
    ParseResponse(String),

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("invalid tag pattern: {0}")]
    InvalidPattern(String),

    #[error("registry rate limit exceeded")]
    RateLimited,

    #[error("unsupported registry: {0}")]
    UnsupportedRegistry(String),

    #[error("docker daemon connection failed: {0}")]
    DaemonConnection(String),

    #[error("docker pull failed: {0}")]
    PullFailed(String),

    #[error("platform {platform} is not available for image {image}:{tag}")]
    PlatformNotAvailable {
        platform: String,
        image: String,
        tag: String,
    },
}

/// Result type alias for Docker plugin operations.
pub type Result<T> = std::result::Result<T, Report<DockerError>>;

impl_report_conversion!(reqwest::Error => DockerError, |e| DockerError::Request(e.to_string()));
impl_report_conversion!(PluginError => DockerError, |e| DockerError::Configuration(e.to_string()));
impl_report_conversion!(DockerError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
#[cfg(feature = "daemon")]
impl_report_conversion!(bollard::errors::Error => DockerError, |e| DockerError::DaemonConnection(e.to_string()));
