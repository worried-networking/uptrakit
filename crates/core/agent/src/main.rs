mod cli;
mod client;
mod host_info;

use clap::Parser;
use rootcause::prelude::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{Capability, ControllerMessage, ReportHostsPayload, ServiceMessage};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, default_resolve_shutdown,
};

use cli::Args;

/// Minimum interval between consecutive update executions on this agent.
///
/// Rapid-fire update messages from a compromised controller are rejected
/// with a `security_audit:` warning. Legitimate orchestration always waits
/// for the previous update to finish before sending the next one.
const UPDATE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(5);

struct AgentHandler {
    /// Local machine ID, collected once on connect and used to validate
    /// incoming `host_machine_id` fields as a defensive sanity check.
    machine_id: String,
    in_flight_update: Option<client::InFlightUpdate>,
    /// Path to the operator-controlled freeze file.
    ///
    /// When this file exists, the agent rejects all `ExecuteUpdate` and
    /// `ExecuteBatchHostPackageUpdate` messages without executing them.
    /// Operators can create the file with `touch <path>` to halt update
    /// execution from the agent side, independent of the controller.
    ///
    /// Default path: `<state-dir>/update-freeze`.
    freeze_file_path: PathBuf,
    /// Timestamp of the last accepted update execution, for rate limiting.
    last_update_accepted: Option<std::time::Instant>,
    /// Shared command executor, created once and reused across all message handlers.
    executor: Arc<dyn CommandExecutor>,
}

#[async_trait::async_trait]
impl ServiceHandler for AgentHandler {
    const DIR_NAME: &'static str = "agent";
    const SERVICE_LABEL: &'static str = "uptrakit-agent service";
    const SERVICE_APP_NAME: &'static str = env!("CARGO_PKG_NAME");

    type ServiceEvent = client::UpdateEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let host_info = crate::host_info::collect_host_info();
        // Capture and store the machine_id for use in on_message() validation.
        self.machine_id = host_info.machine_id.clone();
        conn.send_auto_paginate(ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts: vec![host_info],
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: agent_capabilities(),
        }))
        .await
        .context_to::<LoopError>()?;
        tracing::debug!(
            "sent ReportHosts with agent_version={}",
            env!("CARGO_PKG_VERSION")
        );
        Ok(())
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match msg {
            ControllerMessage::CheckVersions(payload) => {
                if payload.host_machine_id != self.machine_id {
                    tracing::warn!(
                        expected = %self.machine_id,
                        received = %payload.host_machine_id,
                        "host_machine_id mismatch on CheckVersions; ignoring message"
                    );
                    return Ok(None);
                }
                Ok(client::handle_check_versions(payload, self.executor.clone(), conn).await)
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                if payload.host_machine_id != self.machine_id {
                    tracing::warn!(
                        expected = %self.machine_id,
                        received = %payload.host_machine_id,
                        "host_machine_id mismatch on ExecuteUpdate; ignoring message"
                    );
                    return Ok(None);
                }
                if is_frozen(&self.freeze_file_path).await {
                    tracing::warn!(
                        freeze_file = %self.freeze_file_path.display(),
                        "update execution is frozen; ignoring ExecuteUpdate message. \
                         Remove the freeze file to re-enable update execution."
                    );
                    return Ok(None);
                }
                if let Some(last) = self.last_update_accepted
                    && last.elapsed() < UPDATE_COOLDOWN
                {
                    tracing::warn!(
                        cooldown_secs = UPDATE_COOLDOWN.as_secs(),
                        elapsed_ms = last.elapsed().as_millis() as u64,
                        "security_audit: update rate limit exceeded; ignoring ExecuteUpdate"
                    );
                    return Ok(None);
                }
                self.last_update_accepted = Some(std::time::Instant::now());
                client::handle_execute_update(
                    *payload,
                    self.executor.clone(),
                    &mut self.in_flight_update,
                    conn,
                )
                .await;
                Ok(None)
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                if payload.host_machine_id != self.machine_id {
                    tracing::warn!(
                        expected = %self.machine_id,
                        received = %payload.host_machine_id,
                        "host_machine_id mismatch on DiscoverSoftware; ignoring message"
                    );
                    return Ok(None);
                }
                Ok(client::handle_discover_software(payload, self.executor.clone(), conn).await)
            }
            ControllerMessage::ExecuteBatchHostPackageUpdate(payload) => {
                if payload.host_machine_id != self.machine_id {
                    tracing::warn!(
                        expected = %self.machine_id,
                        received = %payload.host_machine_id,
                        "host_machine_id mismatch on ExecuteBatchHostPackageUpdate; ignoring message"
                    );
                    return Ok(None);
                }
                if is_frozen(&self.freeze_file_path).await {
                    tracing::warn!(
                        freeze_file = %self.freeze_file_path.display(),
                        "update execution is frozen; ignoring ExecuteBatchHostPackageUpdate message. \
                         Remove the freeze file to re-enable update execution."
                    );
                    return Ok(None);
                }
                if let Some(last) = self.last_update_accepted
                    && last.elapsed() < UPDATE_COOLDOWN
                {
                    tracing::warn!(
                        cooldown_secs = UPDATE_COOLDOWN.as_secs(),
                        elapsed_ms = last.elapsed().as_millis() as u64,
                        "security_audit: update rate limit exceeded; ignoring ExecuteBatchHostPackageUpdate"
                    );
                    return Ok(None);
                }
                self.last_update_accepted = Some(std::time::Instant::now());
                Ok(client::handle_execute_batch_host_package_update(
                    *payload,
                    self.executor.clone(),
                    conn,
                )
                .await)
            }
            ControllerMessage::SetUpdateFreeze(payload) => {
                handle_set_update_freeze(&self.freeze_file_path, payload).await;
                Ok(None)
            }
            _ => {
                tracing::debug!("ignoring unrecognized message in authenticated loop");
                Ok(None)
            }
        }
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        if let Some(ref mut update) = self.in_flight_update {
            tokio::select! {
                biased;
                Some(output_msg) = update.output_rx.recv() => {
                    client::UpdateEvent::Output(output_msg)
                }
                result = &mut update.handle => {
                    client::UpdateEvent::Completed(result)
                }
            }
        } else {
            std::future::pending::<Self::ServiceEvent>().await
        }
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        let Some(ref update) = self.in_flight_update else {
            tracing::error!("received update event but no in-flight update exists");
            return Ok(None);
        };
        let update_history_id = update.update_history_id;

        match event {
            client::UpdateEvent::Output(output_msg) => {
                client::send_update_output(conn, update_history_id, output_msg).await;
            }
            client::UpdateEvent::Completed(result) => {
                if let Err(e) = client::send_update_result(conn, update_history_id, result).await {
                    tracing::error!(error = %e, "failed to send UpdateResult; disconnecting");
                    self.in_flight_update = None;
                    return Ok(Some(LoopOutcome::Disconnected));
                }
                self.in_flight_update = None;
            }
        }
        Ok(None)
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        agent_capabilities()
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        shutdown_timeout: std::time::Duration,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
        client::handle_graceful_shutdown(
            conn,
            self.in_flight_update.take(),
            shutdown_timeout,
            disconnect_reason,
            outcome,
        )
        .await
    }
}

/// Returns `true` if the freeze file exists on the filesystem.
///
/// When the freeze file is present, the agent refuses to process any
/// `ExecuteUpdate` or `ExecuteBatchHostPackageUpdate` messages.  Operators
/// can create the file with `touch <path>` to halt update execution from the
/// agent side without stopping the agent process or losing connectivity.
///
/// I/O errors are treated conservatively as *not frozen* so a transient
/// filesystem error does not permanently halt the agent.
async fn is_frozen(freeze_file_path: &std::path::Path) -> bool {
    tokio::fs::try_exists(freeze_file_path)
        .await
        .unwrap_or(false)
}

/// Handle a remote `SetUpdateFreeze` message by creating or removing the
/// freeze file on the local filesystem.
///
/// This piggybacks on the existing freeze-file mechanism: local `touch` still
/// works, and the freeze persists across agent restarts.
async fn handle_set_update_freeze(
    freeze_file_path: &std::path::Path,
    payload: uptrakit_internal_wire::SetUpdateFreezePayload,
) {
    let reason = payload.reason.as_deref().unwrap_or("(no reason given)");
    if payload.enabled {
        match tokio::fs::write(freeze_file_path, "").await {
            Ok(()) => {
                tracing::warn!(
                    freeze_file = %freeze_file_path.display(),
                    reason = reason,
                    "security_audit: update freeze enabled via remote command"
                );
            }
            Err(e) => {
                tracing::error!(
                    freeze_file = %freeze_file_path.display(),
                    error = %e,
                    "failed to create freeze file"
                );
            }
        }
    } else {
        match tokio::fs::remove_file(freeze_file_path).await {
            Ok(()) => {
                tracing::warn!(
                    freeze_file = %freeze_file_path.display(),
                    reason = reason,
                    "security_audit: update freeze disabled via remote command"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(
                    freeze_file = %freeze_file_path.display(),
                    "freeze file did not exist; no action taken"
                );
            }
            Err(e) => {
                tracing::error!(
                    freeze_file = %freeze_file_path.display(),
                    error = %e,
                    "failed to remove freeze file"
                );
            }
        }
    }
}

/// Capabilities advertised by the agent service.
fn agent_capabilities() -> BTreeSet<Capability> {
    [
        Capability::SoftwareDiscovery,
        Capability::UpdateHooks,
        Capability::GracefulShutdown,
    ]
    .into_iter()
    .collect()
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            "uptrakit-agent",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    uptrakit_service_sdk::init_tracing("uptrakit_agent", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    // Resolve the freeze file path early so we can pass it to the handler.
    // The lifecycle will resolve dirs again internally; this is a cheap
    // second call.  The default path is <state-dir>/update-freeze.
    let freeze_file_path = args
        .common
        .resolve_dirs("agent")
        .map(|dirs| dirs.state_dir().join("update-freeze"))
        .unwrap_or_else(|_| PathBuf::from("update-freeze"));

    let mut handler = AgentHandler {
        machine_id: String::new(),
        in_flight_update: None,
        freeze_file_path,
        last_update_accepted: None,
        executor: client::make_executor(),
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent",
        &args.common,
        &mut handler,
    )
    .await;
}
