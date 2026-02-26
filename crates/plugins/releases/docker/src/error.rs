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
}

/// Result type alias for Docker plugin operations.
pub type Result<T> = std::result::Result<T, Report<DockerError>>;

impl_report_conversion!(reqwest::Error => DockerError, |e| DockerError::Request(e.to_string()));
impl_report_conversion!(PluginError => DockerError, |e| DockerError::Configuration(e.to_string()));
impl_report_conversion!(DockerError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
impl_report_conversion!(bollard::errors::Error => DockerError, |e| DockerError::DaemonConnection(e.to_string()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_request_error() {
        let err = DockerError::Request("connection refused".to_string());
        assert_eq!(err.to_string(), "HTTP request failed: connection refused");
    }

    #[test]
    fn display_api_error() {
        let err = DockerError::ApiError {
            status: reqwest::StatusCode::NOT_FOUND,
            message: "not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "registry API error: 404 Not Found not found"
        );
    }

    #[test]
    fn display_auth_failed() {
        let err = DockerError::AuthFailed("invalid credentials".to_string());
        assert_eq!(
            err.to_string(),
            "authentication failed: invalid credentials"
        );
    }

    #[test]
    fn display_parse_response() {
        let err = DockerError::ParseResponse("unexpected JSON".to_string());
        assert_eq!(
            err.to_string(),
            "failed to parse registry API response: unexpected JSON"
        );
    }

    #[test]
    fn display_configuration() {
        let err = DockerError::Configuration("page_size is zero".to_string());
        assert_eq!(err.to_string(), "configuration error: page_size is zero");
    }

    #[test]
    fn display_plugin() {
        let err = DockerError::Plugin("upstream failure".to_string());
        assert_eq!(err.to_string(), "plugin error: upstream failure");
    }

    #[test]
    fn display_invalid_pattern() {
        let err = DockerError::InvalidPattern("[bad regex".to_string());
        assert_eq!(err.to_string(), "invalid tag pattern: [bad regex");
    }

    #[test]
    fn display_rate_limited() {
        let err = DockerError::RateLimited;
        assert_eq!(err.to_string(), "registry rate limit exceeded");
    }

    #[test]
    fn display_unsupported_registry() {
        let err = DockerError::UnsupportedRegistry("custom.io".to_string());
        assert_eq!(err.to_string(), "unsupported registry: custom.io");
    }

    #[test]
    fn display_daemon_connection() {
        let err = DockerError::DaemonConnection("no such socket".to_string());
        assert_eq!(
            err.to_string(),
            "docker daemon connection failed: no such socket"
        );
    }

    #[test]
    fn display_pull_failed() {
        let err = DockerError::PullFailed("manifest not found".to_string());
        assert_eq!(err.to_string(), "docker pull failed: manifest not found");
    }
}
