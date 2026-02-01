use rootcause::Report;
use thiserror::Error;

/// Errors that can occur within provider operations.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("version parse error: {0}")]
    VersionParse(String),

    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Result type alias for provider operations.
pub type Result<T> = std::result::Result<T, Report<ProviderError>>;
