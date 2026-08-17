//! Remote command execution abstraction.
//!
//! Defines [`RemoteExecutor`] for running commands on remote machines without
//! assuming a specific transport. Both the SSH agent (`SshSession`) and the
//! Proxmox guest executor (`PveGuestExecutor`) implement this trait, allowing
//! bootstrap and sudoers logic to be transport-agnostic.

use async_trait::async_trait;
use rootcause::prelude::*;

use crate::error::CommandError;

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

/// FHS binary directories prepended to the child's `PATH` so `command -v`
/// resolution is deterministic (`/usr/bin` ahead of `/bin` on merged-`/usr`
/// systems), regardless of whether the caller runs from a console, cron, or
/// a systemd unit.
const FHS_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

fn fhs_first_path() -> String {
    // Prepend, don't replace: the inherited tail keeps non-FHS hosts (NixOS)
    // and version-manager shims resolvable, and never introduces an empty
    // PATH entry (which would mean the current directory).
    match std::env::var("PATH") {
        Ok(inherited) if !inherited.is_empty() => format!("{FHS_PATH}:{inherited}"),
        _ => FHS_PATH.to_string(),
    }
}

/// [`RemoteExecutor`] that runs commands on the local host via `sh -c`.
///
/// Lets host-provisioning logic written against the SSH executor seam
/// (sudoers, helper scripts) run unchanged on the machine itself.
///
/// Commands run with stdin closed and with the FHS directories prepended to
/// `PATH` — output of `command -v` here feeds sudoers grant paths, which must
/// not vary with the caller's environment. Not a general-purpose local shell:
/// reuse for anything env-sensitive should account for both.
#[derive(Debug, Default)]
pub struct LocalRemoteExecutor;

#[async_trait]
impl RemoteExecutor for LocalRemoteExecutor {
    async fn exec_command(&self, command: &str) -> crate::Result<RemoteCommandResult> {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            // Provisioning runs unattended (PVEHS `/usr/bin/update` calls it);
            // an inherited stdin would let a prompting child block forever.
            .stdin(std::process::Stdio::null())
            .env("PATH", fhs_first_path())
            .output()
            .await
            .map_err(|e| report!(CommandError::CommandSpawn(e)))?;
        Ok(RemoteCommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output
                .status
                .code()
                .and_then(|c| u32::try_from(c).ok())
                .unwrap_or(u32::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_executor_captures_stdout_and_exit_zero() {
        let result = LocalRemoteExecutor
            .exec_command("echo hello")
            .await
            .expect("exec");
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn local_executor_captures_stderr_and_nonzero_exit() {
        let result = LocalRemoteExecutor
            .exec_command("echo oops >&2; exit 3")
            .await
            .expect("exec");
        assert_eq!(result.stderr.trim(), "oops");
        assert_eq!(result.exit_code, 3);
    }

    #[tokio::test]
    async fn local_executor_prepends_fhs_path() {
        let result = LocalRemoteExecutor
            .exec_command("printf '%s' \"$PATH\"")
            .await
            .expect("exec");
        assert!(
            result.stdout.starts_with("/usr/local/sbin:"),
            "{}",
            result.stdout
        );
    }
}
