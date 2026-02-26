use thiserror::Error;

/// Errors specific to the Shell plugin.
#[derive(Debug, Error)]
pub enum ShellError {
    /// Plugin configuration is invalid.
    #[error("shell plugin configuration error: {0}")]
    Configuration(String),

    /// The requested operation is not supported because the relevant command
    /// field is absent in the configuration.
    #[error("shell plugin operation not configured: {0}")]
    NotConfigured(String),

    /// The shell command returned a non-zero exit code or could not be spawned.
    #[error("shell command failed: {0}")]
    CommandFailed(String),
}

/// Convenience alias.
pub type Result<T> = rootcause::Result<T, ShellError>;
