//! Command execution utilities for provider operations.
//!
//! Re-exports the [`CommandExecutor`] abstraction from `uptrakit_command` so
//! that provider crates access everything through `uptrakit_provider_core`.

use crate::error::ProviderError;

// Direct re-exports (no error conversion needed)
pub use uptrakit_command::{send_output, shell_escape};

// Executor abstraction re-exports
pub use uptrakit_command::{
    CommandExecutor, CommandMode, CommandOutput, CommandSpec, LocalCommandExecutor,
};

// Error conversion: CommandError -> ProviderError
uptrakit_shared_macros::impl_report_conversion!(
    uptrakit_command::CommandError => ProviderError, |e| {
        match e {
            uptrakit_command::CommandError::CommandSpawn(io) => ProviderError::CommandSpawn(io),
            uptrakit_command::CommandError::CaptureFailed(s) => ProviderError::CaptureFailed(s),
            uptrakit_command::CommandError::CommandFailed(code) => ProviderError::CommandFailed(code),
            uptrakit_command::CommandError::CommandWait(io) => ProviderError::CommandWait(io),
            uptrakit_command::CommandError::TimedOut => ProviderError::TimedOut,
        }
    }
);
