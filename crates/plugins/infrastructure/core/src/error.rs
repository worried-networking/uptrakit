use rootcause::prelude::*;
use thiserror::Error;

/// Errors that can occur within plugin operations.
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("version parse error: {0}")]
    VersionParse(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("missing plugin config field: {0}")]
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

    #[error("command timed out")]
    TimedOut,

    #[error("unsupported shell variant: {0}")]
    UnsupportedShell(String),

    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("install command failed: {0}")]
    InstallFailed(String),

    #[error("plugin internal error: {0}")]
    PluginInternal(String),
}

/// Result type alias for plugin operations.
pub type Result<T> = std::result::Result<T, Report<PluginError>>;
