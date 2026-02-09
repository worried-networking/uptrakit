use rootcause::prelude::*;
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

    #[error("missing provider config field: {0}")]
    MissingConfig(String),

    #[error("no release info provided")]
    MissingReleaseInfo,

    #[error("command spawn failed: {0}")]
    CommandSpawn(#[source] std::io::Error),

    #[error("failed to capture {0}")]
    CaptureFailed(String),

    #[error("command exited with code {0}")]
    CommandFailed(i32),

    #[error("command execution failed: {0}")]
    CommandWait(#[source] std::io::Error),

    #[error("install command failed: {0}")]
    InstallFailed(String),
}

/// Result type alias for provider operations.
pub type Result<T> = std::result::Result<T, Report<ProviderError>>;
