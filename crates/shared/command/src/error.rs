use rootcause::prelude::*;
use thiserror::Error;

/// Errors that can occur during command execution.
#[derive(Debug, Error)]
pub enum CommandError {
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

    /// PTY allocation failed during interactive command execution.
    #[error("PTY allocation failed: {0}")]
    PtyAllocationFailed(#[source] std::io::Error),
}

/// Result type alias for command operations.
pub type Result<T> = std::result::Result<T, Report<CommandError>>;
