//! [`GuestExecProvider`] implementation for Proxmox VE.
//!
//! Wraps the existing `guest_exec` module to implement the generic
//! `GuestExecProvider` trait, allowing the SSH agent to execute commands
//! inside PVE guests without depending on this crate directly.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use uptrakit_command::{
    CommandError, CommandExecutor, CommandOutput, CommandSpec, RemoteCommandResult, RemoteExecutor,
    StdioTunnel, UpdateOutputLine, build_remote_command_string,
};
use uptrakit_plugin_infrastructure_core::agent_infra::{GuestExecProvider, GuestIpError};

use crate::guest_exec::{self, PveGuestType};

// ── Type conversion ───────────────────────────────────────────────────────────

fn parse_guest_type(guest_type: &str) -> PveGuestType {
    match guest_type {
        "qemu" => PveGuestType::Qemu,
        _ => PveGuestType::Lxc,
    }
}

// ── ProxmoxGuestRemoteExecutor ───────────────────────────────────────────────

/// Implements [`RemoteExecutor`] by routing commands through a PVE host
/// into a guest using `pct exec` (LXC) or `qm guest exec` (QEMU).
struct ProxmoxGuestRemoteExecutor {
    gateway: Arc<dyn RemoteExecutor>,
    guest_id: u32,
    guest_type: PveGuestType,
}

#[async_trait]
impl RemoteExecutor for ProxmoxGuestRemoteExecutor {
    async fn exec_command(&self, command: &str) -> uptrakit_command::Result<RemoteCommandResult> {
        let result = guest_exec::exec_in_guest(
            self.gateway.as_ref(),
            self.guest_id,
            self.guest_type,
            command,
        )
        .await
        .map_err(|e| {
            rootcause::report!(CommandError::CommandSpawn(std::io::Error::other(
                e.to_string()
            )))
        })?;

        Ok(RemoteCommandResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: u32::try_from(result.exit_code).unwrap_or(u32::MAX),
        })
    }
}

// ── ProxmoxGuestCommandExecutor ──────────────────────────────────────────────

/// Adapts [`ProxmoxGuestRemoteExecutor`] to the [`CommandExecutor`] interface.
///
/// Converts a [`CommandSpec`] to a shell-safe string and delegates to the
/// guest remote executor. Streaming is not supported — `execute` behaves
/// identically to `execute_quiet`.
struct ProxmoxGuestCommandExecutor(ProxmoxGuestRemoteExecutor);

#[async_trait]
impl CommandExecutor for ProxmoxGuestCommandExecutor {
    async fn execute(
        &self,
        spec: &CommandSpec,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        self.execute_quiet(spec).await
    }

    async fn execute_quiet(&self, spec: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        let cmd = build_remote_command_string(spec)?;
        let result = self.0.exec_command(&cmd).await.map_err(|e| {
            rootcause::report!(CommandError::CommandSpawn(std::io::Error::other(
                e.to_string()
            )))
        })?;

        if result.exit_code != 0 {
            let exit_code = i32::try_from(result.exit_code).unwrap_or(-1);
            rootcause::bail!(CommandError::CommandFailed(exit_code));
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
        rootcause::bail!(CommandError::UnsupportedShell(
            "PVE guest exec does not support stdio tunnels".to_string()
        ))
    }
}

// ── ProxmoxGuestExecProvider ─────────────────────────────────────────────────

/// [`GuestExecProvider`] for Proxmox VE.
///
/// Creates executors that run commands inside PVE guests via the PVE host's
/// SSH connection using `pct exec` (LXC) or `qm guest exec` (QEMU).
pub struct ProxmoxGuestExecProvider;

#[async_trait]
impl GuestExecProvider for ProxmoxGuestExecProvider {
    fn create_guest_remote_executor(
        &self,
        gateway: Arc<dyn RemoteExecutor>,
        guest_id: u32,
        guest_type: &str,
    ) -> Arc<dyn RemoteExecutor> {
        Arc::new(ProxmoxGuestRemoteExecutor {
            gateway,
            guest_id,
            guest_type: parse_guest_type(guest_type),
        })
    }

    fn create_guest_command_executor(
        &self,
        gateway: Arc<dyn RemoteExecutor>,
        guest_id: u32,
        guest_type: &str,
    ) -> Arc<dyn CommandExecutor> {
        Arc::new(ProxmoxGuestCommandExecutor(ProxmoxGuestRemoteExecutor {
            gateway,
            guest_id,
            guest_type: parse_guest_type(guest_type),
        }))
    }

    async fn get_guest_ip(
        &self,
        gateway: &dyn RemoteExecutor,
        guest_id: u32,
        guest_type: &str,
    ) -> std::result::Result<String, GuestIpError> {
        guest_exec::get_guest_ip(gateway, guest_id, parse_guest_type(guest_type))
            .await
            .map_err(|e| GuestIpError::from(e.to_string()))
    }
}
