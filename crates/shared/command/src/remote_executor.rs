//! Remote command execution abstraction.
//!
//! Defines [`RemoteExecutor`] for running commands on remote machines without
//! assuming a specific transport. Both the SSH agent (`SshSession`) and the
//! Proxmox guest executor (`PveGuestExecutor`) implement this trait, allowing
//! bootstrap and sudoers logic to be transport-agnostic.

use async_trait::async_trait;

/// Result of executing a command on a remote machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommandResult {
    /// Standard output (may be empty).
    pub stdout: String,
    /// Standard error (may be empty).
    pub stderr: String,
    /// Process exit code (`0` typically means success).
    pub exit_code: u32,
}

/// Trait for executing commands on a remote machine.
///
/// Implementors wrap transport-specific mechanisms (SSH sessions,
/// `pct exec`, `qm guest exec`, etc.) and expose a uniform interface.
///
/// All implementations must be `Send + Sync` to allow use behind `Arc<dyn RemoteExecutor>`.
#[async_trait]
pub trait RemoteExecutor: Send + Sync {
    /// Execute a shell command on the remote machine and return the result.
    ///
    /// The `command` string is passed to the remote shell (typically `bash -c`
    /// or the default login shell). The caller is responsible for escaping.
    async fn exec_command(&self, command: &str) -> crate::Result<RemoteCommandResult>;
}
