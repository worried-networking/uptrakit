use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum CargoError {
    #[error("command execution failed with exit code {0}")]
    CommandFailed(i32),
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("request error: {0}")]
    Request(String),
    #[error("API error (status {status}): {message}")]
    ApiError {
        status: reqwest::StatusCode,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, Report<CargoError>>;

impl_report_conversion!(reqwest::Error => CargoError, |e| CargoError::Request(e.to_string()));
impl_report_conversion!(CargoError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
impl_report_conversion!(PluginError => CargoError, |e| CargoError::Configuration(e.to_string()));
