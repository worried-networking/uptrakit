use uptrakit_shared_types::{HookShell, OutputStreamType};

use crate::command::{get_shell_args, wrap_command_for_shell};

/// A single line of output from a command execution.
#[derive(Clone, Debug)]
pub struct UpdateOutputLine {
    /// The text content of the output line.
    pub text: String,
    /// Which output stream this line came from.
    pub stream: OutputStreamType,
}

/// Handle for an interactive command session with PTY support.
///
/// Returned by [`crate::executor::CommandExecutor::execute_interactive`]. The caller uses the
/// channels to forward stdin data and signals to the running process, and
/// receives notifications when the process appears to be waiting for input.
#[cfg(feature = "interactive")]
pub struct InteractiveHandle {
    /// Send raw bytes to the process stdin (PTY master write end).
    pub stdin_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    /// Send a signal number to the process group (e.g., 2 = SIGINT, 15 = SIGTERM).
    pub signal_tx: tokio::sync::mpsc::Sender<i32>,
    /// Await process completion. Resolves with accumulated output and exit code.
    pub completion: tokio::task::JoinHandle<crate::Result<CommandOutput>>,
    /// Receives a notification when the process appears to be waiting for stdin
    /// (no output for ~10 seconds while still running).
    pub attention_rx: tokio::sync::mpsc::Receiver<()>,
}

/// How a command is invoked.
#[derive(Clone, Debug)]
pub enum CommandMode {
    /// Direct program execution (no shell interpretation).
    Exec {
        /// The program to run.
        program: String,
        /// Arguments to pass to the program.
        args: Vec<String>,
    },
    /// Shell-interpreted command with fail-early settings.
    Shell {
        /// The shell command string.
        command: String,
        /// Which shell to use.
        shell: HookShell,
    },
}

/// Specification for a command to execute.
///
/// Plugins build a `CommandSpec` describing *what* to run, and the injected
/// [`crate::executor::CommandExecutor`] decides *how* to run it (locally, over SSH, etc.).
///
/// The `privileged` flag marks commands that require root privileges. When
/// `true`, a [`crate::sudo::SudoAwareCommandExecutor`] will prepend `sudo`
/// to the command if the active [`crate::sudo::SudoContext`] indicates that
/// sudo is available and the current user is not already root. Shell-mode
/// commands are always passed through unchanged regardless of this flag.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    /// How the command should be invoked.
    pub mode: CommandMode,
    /// Optional working directory for the command.
    pub working_dir: Option<String>,
    /// Maximum time to wait for the command. `None` means no timeout is applied.
    pub timeout: Option<std::time::Duration>,
    /// Whether this command requires root privileges.
    ///
    /// When `true` and the active executor is a [`crate::sudo::SudoAwareCommandExecutor`],
    /// `sudo` is prepended to `Exec`-mode commands if the current user is
    /// non-root and passwordless sudo is available. Has no effect on `Shell`
    /// mode — shell commands must handle privilege escalation themselves.
    pub privileged: bool,
    /// Extra environment variables to set for the process.
    ///
    /// Each entry is a `(name, value)` pair. For local execution these are set
    /// directly on the spawned process. For SSH execution they are prepended
    /// as `NAME='VALUE'` assignments in the remote command string. When sudo
    /// is in use they are forwarded as inline `NAME=VALUE` assignments before
    /// the program name (`sudo NAME=VALUE PROG …`); the sudoers entry must
    /// carry `SETENV:` for sudo to accept this form.
    pub envs: Vec<(String, String)>,
}

/// Output captured from a command execution.
#[derive(Clone, Debug)]
pub struct CommandOutput {
    /// The accumulated stdout followed by stderr output.
    ///
    /// Stdout content always precedes stderr content, regardless of the actual
    /// temporal interleaving of the two streams. This is a fundamental limitation
    /// of reading from separate pipes.
    pub output: String,
    /// The process exit code.
    pub exit_code: i32,
}

impl CommandSpec {
    /// Create a spec for direct program execution (no shell).
    pub fn exec(program: impl Into<String>, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            mode: CommandMode::Exec {
                program: program.into(),
                args: args.into_iter().collect(),
            },
            working_dir: None,
            timeout: None,
            privileged: false,
            envs: vec![],
        }
    }

    /// Create a spec for a shell command using Bash.
    pub fn shell(command: impl Into<String>) -> Self {
        Self {
            mode: CommandMode::Shell {
                command: command.into(),
                shell: HookShell::Bash,
            },
            working_dir: None,
            timeout: None,
            privileged: false,
            envs: vec![],
        }
    }

    /// Create a spec for a shell command using the specified shell.
    pub fn shell_with(command: impl Into<String>, shell: HookShell) -> Self {
        Self {
            mode: CommandMode::Shell {
                command: command.into(),
                shell,
            },
            working_dir: None,
            timeout: None,
            privileged: false,
            envs: vec![],
        }
    }

    /// Mark this command as requiring root privileges (builder pattern).
    ///
    /// A [`crate::sudo::SudoAwareCommandExecutor`] will prepend `sudo` when
    /// the host context indicates that sudo is available and the current user
    /// is non-root. This flag has **no effect** on Shell-mode commands.
    #[must_use]
    pub fn privileged(mut self) -> Self {
        self.privileged = true;
        self
    }

    /// Set the working directory (builder pattern).
    #[must_use]
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Add an environment variable to the command (builder pattern).
    ///
    /// For local execution the variable is set directly on the spawned process.
    /// For SSH execution it is prepended as a `NAME='VALUE'` assignment.
    /// When sudo is used it is forwarded via `sudo env NAME=VALUE …`.
    #[must_use]
    pub fn with_env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.push((name.into(), value.into()));
        self
    }

    /// Set a maximum execution time for the command (builder pattern).
    ///
    /// When the deadline is reached the executor returns
    /// [`CommandError::TimedOut`]. The child process is sent `SIGKILL` via
    /// `kill_on_drop(true)`, which fires automatically when the
    /// `tokio::process::Child` handle is dropped as the timed-out future is
    /// cancelled.
    #[must_use]
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Resolve to (program, args) for process execution.
    ///
    /// - **Exec** mode: returns `(program, args)` unchanged.
    /// - **Shell** mode: wraps the command with fail-early settings and returns
    ///   `(shell_executable, [flag, wrapped_command])`.
    ///
    /// Returns [`crate::error::CommandError::UnsupportedShell`] if the shell
    /// variant is not recognized by this version of the agent.
    pub fn resolve(&self) -> crate::Result<(String, Vec<String>)> {
        match &self.mode {
            CommandMode::Exec { program, args } => Ok((program.clone(), args.clone())),
            CommandMode::Shell { command, shell } => {
                let wrapped = wrap_command_for_shell(command, *shell)?;
                let (shell_exec, shell_arg) = get_shell_args(*shell);
                Ok((shell_exec.to_string(), vec![shell_arg.to_string(), wrapped]))
            }
        }
    }
}
