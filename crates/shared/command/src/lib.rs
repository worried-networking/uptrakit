pub mod command;
pub mod error;
pub mod executor;
#[cfg(feature = "interactive")]
pub mod interactive;
pub mod remote_executor;
pub mod sudo;
pub mod types;

pub use command::{
    run_command, run_command_exec, run_command_exec_quiet, run_command_quiet,
    run_command_with_shell, run_command_with_shell_quiet, send_output, shell_escape,
};
pub use error::{CommandError, Result};
#[cfg(feature = "interactive")]
pub use executor::InteractiveHandle;
pub use executor::{
    CommandExecutor, CommandMode, CommandOutput, CommandSpec, LocalCommandExecutor,
    NoopCommandExecutor, StdioTunnel, build_remote_command_string,
};
pub use remote_executor::{RemoteCommandResult, RemoteExecutor};
pub use sudo::{ParseSudoPolicyError, SudoAwareCommandExecutor, SudoContext, SudoPolicy};
pub use types::UpdateOutputLine;
