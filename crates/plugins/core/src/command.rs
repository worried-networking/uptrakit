//! Command execution utilities for plugin operations.
//!
//! Re-exports the [`CommandExecutor`] abstraction from `uptrakit_command` so
//! that plugin crates access everything through `uptrakit_plugin_core`.

use crate::error::PluginError;

// Direct re-exports (no error conversion needed)
pub use uptrakit_command::{send_output, shell_escape};

// Executor abstraction re-exports
pub use uptrakit_command::{
    CommandExecutor, CommandMode, CommandOutput, CommandSpec, LocalCommandExecutor,
};

// Error conversion: CommandError -> PluginError
uptrakit_shared_macros::impl_report_conversion!(
    uptrakit_command::CommandError => PluginError, |e| {
        match e {
            uptrakit_command::CommandError::CommandSpawn(io) => PluginError::CommandSpawn(io),
            uptrakit_command::CommandError::CaptureFailed(s) => PluginError::CaptureFailed(s),
            uptrakit_command::CommandError::CommandFailed(code) => PluginError::CommandFailed(code),
            uptrakit_command::CommandError::CommandWait(io) => PluginError::CommandWait(io),
            uptrakit_command::CommandError::TimedOut => PluginError::TimedOut,
            uptrakit_command::CommandError::UnsupportedShell(s) => PluginError::UnsupportedShell(s),
        }
    }
);
