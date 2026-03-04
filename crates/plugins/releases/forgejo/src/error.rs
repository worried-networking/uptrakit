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
    ApiError {
        status: reqwest::StatusCode,
        message: String,
    },

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_api_error() {
        let err = ForgejoError::ApiError {
            status: reqwest::StatusCode::UNAUTHORIZED,
            message: "token required".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Forgejo API error: 401 Unauthorized token required"
        );
    }

    #[test]
    fn display_rate_limited() {
        let err = ForgejoError::RateLimited;
        assert_eq!(err.to_string(), "Forgejo API rate limit exceeded");
    }

    #[test]
    fn display_configuration() {
        let err = ForgejoError::Configuration("api_base_url required".to_string());
        assert_eq!(
            err.to_string(),
            "configuration error: api_base_url required"
        );
    }
}
