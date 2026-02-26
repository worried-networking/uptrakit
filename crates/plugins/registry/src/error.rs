use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors that can occur in the plugin registry.
#[derive(Debug, Error)]
pub enum PluginRegistryError {
    /// Unknown plugin type.
    #[error("unknown plugin type: {0}")]
    UnknownPluginType(String),

    /// Failed to parse plugin configuration.
    #[error("failed to parse config")]
    ConfigParse(#[from] serde_json::Error),

    /// Plugin configuration validation failed.
    #[error("config validation failed: {0}")]
    ConfigValidation(String),

    /// Failed to instantiate plugin.
    #[error("failed to instantiate plugin: {0}")]
    Instantiation(String),
}

/// Result type for registry operations.
pub type Result<T> = std::result::Result<T, Report<PluginRegistryError>>;

impl_report_conversion!(serde_json::Error => PluginRegistryError::ConfigParse);
