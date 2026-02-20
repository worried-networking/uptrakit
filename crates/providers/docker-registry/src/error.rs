use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_provider_core::ProviderError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the Docker Registry provider.
#[derive(Debug, Error)]
pub enum DockerRegistryError {
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

    #[error("provider error: {0}")]
    Provider(String),

    #[error("invalid tag pattern: {0}")]
    InvalidPattern(String),

    #[error("registry rate limit exceeded")]
    RateLimited,

    #[error("unsupported registry: {0}")]
    UnsupportedRegistry(String),
}

/// Result type alias for Docker Registry provider operations.
pub type Result<T> = std::result::Result<T, Report<DockerRegistryError>>;

impl_report_conversion!(reqwest::Error => DockerRegistryError, |e| DockerRegistryError::Request(e.to_string()));
impl_report_conversion!(ProviderError => DockerRegistryError, |e| DockerRegistryError::Configuration(e.to_string()));
impl_report_conversion!(DockerRegistryError => ProviderError, |e| ProviderError::ProviderInternal(e.to_string()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_request_error() {
        let err = DockerRegistryError::Request("connection refused".to_string());
        assert_eq!(err.to_string(), "HTTP request failed: connection refused");
    }

    #[test]
    fn display_api_error() {
        let err = DockerRegistryError::ApiError {
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
        let err = DockerRegistryError::AuthFailed("invalid credentials".to_string());
        assert_eq!(
            err.to_string(),
            "authentication failed: invalid credentials"
        );
    }

    #[test]
    fn display_parse_response() {
        let err = DockerRegistryError::ParseResponse("unexpected JSON".to_string());
        assert_eq!(
            err.to_string(),
            "failed to parse registry API response: unexpected JSON"
        );
    }

    #[test]
    fn display_configuration() {
        let err = DockerRegistryError::Configuration("image is empty".to_string());
        assert_eq!(err.to_string(), "configuration error: image is empty");
    }

    #[test]
    fn display_provider() {
        let err = DockerRegistryError::Provider("upstream failure".to_string());
        assert_eq!(err.to_string(), "provider error: upstream failure");
    }

    #[test]
    fn display_invalid_pattern() {
        let err = DockerRegistryError::InvalidPattern("[bad regex".to_string());
        assert_eq!(err.to_string(), "invalid tag pattern: [bad regex");
    }

    #[test]
    fn display_rate_limited() {
        let err = DockerRegistryError::RateLimited;
        assert_eq!(err.to_string(), "registry rate limit exceeded");
    }

    #[test]
    fn display_unsupported_registry() {
        let err = DockerRegistryError::UnsupportedRegistry("custom.io".to_string());
        assert_eq!(err.to_string(), "unsupported registry: custom.io");
    }
}
