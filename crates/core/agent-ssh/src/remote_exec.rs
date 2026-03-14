//! Remote executor implementations for the SSH agent.
//!
//! - [`SshRemoteExecutor`]: wraps an [`SshSession`] to implement
//!   [`RemoteExecutor`] for direct SSH command execution.

use std::sync::Arc;

use async_trait::async_trait;
use uptrakit_command::{RemoteCommandResult, RemoteExecutor};

use crate::ssh_transport::SshSession;

// ── SshRemoteExecutor ──────────────────────────────────────────────────

/// Implements [`RemoteExecutor`] by delegating to an [`SshSession`].
pub(crate) struct SshRemoteExecutor {
    session: Arc<SshSession>,
}

impl SshRemoteExecutor {
    pub(crate) fn new(session: Arc<SshSession>) -> Self {
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
