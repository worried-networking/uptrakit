//! Remote executor implementations for the SSH agent.
//!
//! - [`SshRemoteExecutor`]: wraps an [`SshSession`] to implement
//!   [`RemoteExecutor`] for direct SSH command execution.
//! - [`PveGuestExecutor`]: wraps an SSH session to a PVE node and executes
//!   commands inside a guest (LXC/QEMU) via `pct exec` / `qm guest exec`.
//! - [`PveGuestCommandExecutor`]: adapts [`PveGuestExecutor`] to the
//!   [`CommandExecutor`] interface so plugin compatibility probes run against
//!   the guest (not the PVE host).

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_command::{
    CommandError, CommandExecutor, CommandOutput, CommandSpec, RemoteCommandResult, RemoteExecutor,
    StdioTunnel, UpdateOutputLine,
};
use uptrakit_plugin_infrastructure_proxmox::guest_exec::{self, PveGuestType};

use crate::ssh_executor::build_remote_command_string;
use crate::ssh_transport::SshSession;

// ── SshRemoteExecutor ──────────────────────────────────────────────────

/// Implements [`RemoteExecutor`] by delegating to an [`SshSession`].
pub struct SshRemoteExecutor {
    session: Arc<SshSession>,
}

impl SshRemoteExecutor {
    pub fn new(session: Arc<SshSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl RemoteExecutor for SshRemoteExecutor {
    async fn exec_command(&self, command: &str) -> uptrakit_command::Result<RemoteCommandResult> {
        let result = self.session.exec_command(command).await.map_err(|e| {
            rootcause::report!(uptrakit_command::CommandError::CommandSpawn(
                std::io::Error::other(e.to_string())
            ))
        })?;
        Ok(RemoteCommandResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        })
    }
}

// ── PveGuestExecutor ───────────────────────────────────────────────────

/// Executes commands inside a PVE guest by SSHing to the PVE node and
/// running `pct exec` (LXC) or `qm guest exec` (QEMU).
#[allow(dead_code)] // Used by bootstrap-proxmox action (Phase 6)
pub struct PveGuestExecutor {
    /// SSH session to the PVE node.
    pve_session: Arc<SshSession>,
    /// VMID of the target guest.
    vmid: u32,
    /// Guest type (LXC or QEMU).
    guest_type: PveGuestType,
}

impl PveGuestExecutor {
    #[allow(dead_code)] // Used by bootstrap-proxmox action (Phase 6)
    pub fn new(pve_session: Arc<SshSession>, vmid: u32, guest_type: PveGuestType) -> Self {
        Self {
            pve_session,
            vmid,
            guest_type,
        }
    }
}

#[async_trait]
impl RemoteExecutor for PveGuestExecutor {
    async fn exec_command(&self, command: &str) -> uptrakit_command::Result<RemoteCommandResult> {
        // Build an SshRemoteExecutor for the PVE node.
        let pve_executor = SshRemoteExecutor::new(Arc::clone(&self.pve_session));
        let result = guest_exec::exec_in_guest(&pve_executor, self.vmid, self.guest_type, command)
            .await
            .map_err(|e| {
                rootcause::report!(uptrakit_command::CommandError::CommandSpawn(
                    std::io::Error::other(e.to_string())
                ))
            })?;
        Ok(RemoteCommandResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: u32::try_from(result.exit_code).unwrap_or(u32::MAX),
        })
    }
}

// ── PveGuestCommandExecutor ────────────────────────────────────────────

/// Adapts [`PveGuestExecutor`] to the [`CommandExecutor`] interface.
///
/// Enables plugin compatibility probes (e.g.,
/// `PluginRegistry::compatible_sudo_commands_for_host`) to run against the
/// **guest** rather than the PVE host. The executor converts a [`CommandSpec`]
/// to a shell-safe string and delegates to [`PveGuestExecutor::exec_command`].
/// Streaming output is not supported (PVE exec runs commands
/// non-interactively); `execute` behaves identically to `execute_quiet`.
pub struct PveGuestCommandExecutor(pub PveGuestExecutor);

#[async_trait]
impl CommandExecutor for PveGuestCommandExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        // PVE exec does not support line-by-line streaming; delegate to
        // execute_quiet and collect the full output.
        self.execute_quiet(spec).await
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        let cmd = build_remote_command_string(spec)?;
        let result = self.0.exec_command(&cmd).await.map_err(|e| {
            report!(CommandError::CommandSpawn(std::io::Error::other(
                e.to_string()
            )))
        })?;

        if result.exit_code != 0 {
            let exit_code = i32::try_from(result.exit_code).unwrap_or(-1);
            bail!(CommandError::CommandFailed(exit_code));
        }

        let mut output = result.stdout;
        output.push_str(&result.stderr);
        let exit_code = i32::try_from(result.exit_code).unwrap_or(0);
        Ok(CommandOutput { output, exit_code })
    }

    fn supports_stdio_tunnel(&self) -> bool {
        false
    }

    async fn open_stdio_tunnel(
        &self,
        _command: &str,
    ) -> uptrakit_command::Result<Box<dyn StdioTunnel>> {
        bail!(CommandError::UnsupportedShell(
            "PVE guest exec does not support stdio tunnels".to_string()
        ))
    }
}
