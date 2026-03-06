use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the Proxmox VE plugin.
#[derive(Debug, Error)]
pub enum ProxmoxError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("Proxmox API error: {status} {message}")]
    ApiError {
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("failed to parse Proxmox API response: {0}")]
    ParseResponse(String),

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("database error: {0}")]
    Database(String),
}

/// Result type alias for Proxmox plugin operations.
pub type Result<T> = std::result::Result<T, Report<ProxmoxError>>;

impl_report_conversion!(reqwest::Error => ProxmoxError, |e| ProxmoxError::Request(e.to_string()));
impl_report_conversion!(PluginError => ProxmoxError, |e| ProxmoxError::Configuration(e.to_string()));
impl_report_conversion!(ProxmoxError => PluginError, |e| PluginError::PluginInternal(e.to_string()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_api_error() {
        let err = ProxmoxError::ApiError {
            status: reqwest::StatusCode::FORBIDDEN,
            message: "permission denied".to_string(),
        };
        assert!(err.to_string().contains("403"));
        assert!(err.to_string().contains("permission denied"));
    }

    #[test]
    fn display_configuration() {
        let err = ProxmoxError::Configuration("invalid api_url".to_string());
        assert_eq!(err.to_string(), "configuration error: invalid api_url");
    }
}
