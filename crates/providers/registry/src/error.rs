use rootcause::ReportConversion;
use rootcause::prelude::*;
use thiserror::Error;

/// Errors that can occur in the provider registry.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Unknown provider type.
    #[error("unknown provider type: {0}")]
    UnknownProviderType(String),

    /// Failed to parse provider configuration.
    #[error("failed to parse config")]
    ConfigParse(#[from] serde_json::Error),

    /// Provider configuration validation failed.
    #[error("config validation failed: {0}")]
    ConfigValidation(String),

    /// Failed to instantiate provider.
    #[error("failed to instantiate provider: {0}")]
    Instantiation(String),
}

/// Result type for registry operations.
pub type Result<T> = std::result::Result<T, Report<RegistryError>>;

impl<T> ReportConversion<serde_json::Error, markers::Mutable, T> for RegistryError
where
    RegistryError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<serde_json::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(RegistryError::ConfigParse)
    }
}
