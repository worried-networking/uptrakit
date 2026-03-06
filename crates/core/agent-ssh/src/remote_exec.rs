//! Remote executor implementations for the SSH agent.
//!
//! - [`SshRemoteExecutor`]: wraps an [`SshSession`] to implement
//!   [`RemoteExecutor`] for direct SSH command execution.
//! - [`PveGuestExecutor`]: wraps an SSH session to a PVE node and executes
//!   commands inside a guest (LXC/QEMU) via `pct exec` / `qm guest exec`.

use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_command::{RemoteCommandResult, RemoteExecutor};
use uptrakit_plugin_infrastructure_proxmox::guest_exec::{self, PveGuestType};

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
