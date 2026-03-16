//! Embedded agent service for single-tenant controller deployments.
//!
//! When the `embedded-agent` feature is enabled, the controller can run a
//! local agent inside its own process. This eliminates the need for a separate
//! `uptrakit-agent` binary in single-tenant deployments.
//!
//! The embedded agent:
//! - Manages the host the controller runs on (software discovery, updates)
//! - Yields to an external agent with the same `machine_id` (coexistence)
//! - Uses in-process mpsc channels instead of WebSocket for transport
//! - Reuses all business logic from `uptrakit-agent-core`

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use uptrakit_agent_core::ConnectionContext;
use uptrakit_agent_core::client::{InFlightUpdate, UpdateEvent};
#[cfg(feature = "interactive")]
use uptrakit_internal_wire::StdinAttentionPayload;
use uptrakit_internal_wire::{
    Capability, ControllerMessage, DisconnectReason, RegisterPayload, ReportHostsPayload,
    ServiceMessage, ServiceTransport,
};

use crate::embedded::types::EmbeddedTransport;

/// Minimum interval between accepted `ExecuteUpdate` / `ExecuteBatchUpdate`
/// messages. Prevents runaway update loops.
const UPDATE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(5);

/// Timeout for graceful shutdown: how long to wait for an in-flight update
/// before abandoning it.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Build the agent capabilities set.
pub(crate) fn agent_capabilities() -> BTreeSet<Capability> {
    let mut caps = BTreeSet::new();
    caps.insert(Capability::SoftwareDiscovery);
    caps.insert(Capability::UpdateHooks);
    caps.insert(Capability::GracefulShutdown);
    #[cfg(feature = "interactive")]
    caps.insert(Capability::InteractiveUpdates);
    caps
}

/// Build the command executor for the embedded agent.
///
/// Uses `LocalCommandExecutor` wrapped in `SudoAwareCommandExecutor` with
/// default sudo context — identical to the standalone agent binary.
fn make_executor() -> Arc<dyn uptrakit_command::CommandExecutor> {
    let raw: Arc<dyn uptrakit_command::CommandExecutor> =
        Arc::new(uptrakit_command::LocalCommandExecutor);
    Arc::new(uptrakit_command::SudoAwareCommandExecutor::new(
        raw,
        uptrakit_command::SudoContext::default(),
    ))
}

/// Check whether the freeze file exists.
async fn is_frozen(freeze_file_path: &std::path::Path) -> bool {
    tokio::fs::try_exists(freeze_file_path)
        .await
        .unwrap_or(false)
}

/// Handle a `SetUpdateFreeze` message by creating or removing the freeze file.
async fn handle_set_update_freeze(
    freeze_file_path: &std::path::Path,
    payload: uptrakit_internal_wire::SetUpdateFreezePayload,
) {
    if payload.enabled {
        if let Some(parent) = freeze_file_path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            tracing::error!(error = %e, "failed to create freeze file directory");
            return;
        }
        if let Err(e) = tokio::fs::write(freeze_file_path, b"").await {
            tracing::error!(error = %e, "failed to create update freeze file");
        } else {
            tracing::warn!(
                reason = ?payload.reason,
                "update freeze enabled via controller"
            );
        }
    } else {
        match tokio::fs::remove_file(freeze_file_path).await {
            Ok(()) => {
                tracing::warn!("update freeze disabled via controller");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("freeze file did not exist; nothing to remove");
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to remove update freeze file");
            }
        }
    }
}

/// Forward stdin data or a signal from the controller to the in-flight update.
#[cfg(feature = "interactive")]
fn handle_update_stdin_data(
    payload: uptrakit_internal_wire::UpdateStdinDataPayload,
    in_flight_update: &Option<InFlightUpdate>,
) {
    let Some(update) = in_flight_update else {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "received UpdateStdinData but no in-flight update exists; ignoring"
        );
        return;
    };
    if update.update_history_id != payload.update_history_id {
        tracing::debug!(
            expected = %update.update_history_id,
            received = %payload.update_history_id,
            "UpdateStdinData update_history_id mismatch; ignoring"
        );
        return;
    }

    if let Some(signal) = payload.signal {
        if let Some(ref signal_tx) = update.signal_tx {
            if signal_tx.try_send(signal).is_err() {
                tracing::warn!("signal channel full or closed; dropping signal {signal}");
            }
        } else {
            tracing::debug!("signal_tx not available for this update; ignoring signal");
        }
    } else if let Some(ref stdin_tx) = update.stdin_tx {
        use base64::Engine as _;
        match base64::engine::general_purpose::STANDARD.decode(&payload.data) {
            Ok(bytes) => {
                if stdin_tx.try_send(bytes).is_err() {
                    tracing::warn!("stdin channel full or closed; dropping stdin data");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to decode base64 stdin data");
            }
        }
    } else {
        tracing::debug!("stdin_tx not available for this update; ignoring stdin data");
    }
}

/// Events produced by the embedded agent's in-flight update polling.
enum AgentEvent {
    /// Progress from an in-flight update task (output, completion, attention).
    Update(UpdateEvent),
}

/// Poll the in-flight update for events. Pends forever when no update is in
/// flight.
async fn poll_in_flight_update(in_flight_update: &mut Option<InFlightUpdate>) -> AgentEvent {
    let Some(update) = in_flight_update else {
        return std::future::pending().await;
    };

    #[cfg(feature = "interactive")]
    let mut attention_rx = update.attention_rx.take();
    let _update_history_id = update.update_history_id;

    #[cfg(feature = "interactive")]
    let event = {
        tokio::select! {
            biased;
            Some(output_msg) = update.output_rx.recv() => {
                AgentEvent::Update(UpdateEvent::Output(output_msg))
            }
            result = &mut update.handle => {
                AgentEvent::Update(UpdateEvent::Completed(result))
            }
            Some(()) = recv_attention_rx(&mut attention_rx) => {
                AgentEvent::Update(UpdateEvent::Attention(_update_history_id))
            }
        }
    };

    #[cfg(not(feature = "interactive"))]
    let event = {
        tokio::select! {
            biased;
            Some(output_msg) = update.output_rx.recv() => {
                AgentEvent::Update(UpdateEvent::Output(output_msg))
            }
            result = &mut update.handle => {
                AgentEvent::Update(UpdateEvent::Completed(result))
            }
        }
    };

    #[cfg(feature = "interactive")]
    {
        update.attention_rx = attention_rx;
    }

    event
}

/// Receive attention notification from the optional attention channel.
#[cfg(feature = "interactive")]
async fn recv_attention_rx(
    attention_rx: &mut Option<tokio::sync::mpsc::Receiver<()>>,
) -> Option<()> {
    if let Some(rx) = attention_rx {
        return rx.recv().await;
    }
    std::future::pending().await
}

/// Run the embedded agent event loop.
///
/// This is the main entry point called from `EmbeddedServiceHost::add()`. It
/// mirrors the standalone agent's event loop but uses `EmbeddedTransport`
/// instead of a WebSocket connection.
pub(crate) async fn run_embedded_agent(
    mut transport: EmbeddedTransport,
    cancel: CancellationToken,
    state_dir: PathBuf,
) {
    let executor = make_executor();
    let caps = agent_capabilities();

    // Collect host info for ReportHosts.
    let host_info = uptrakit_agent_core::host_info::collect_host_info();
    let machine_id = host_info.machine_id.clone();

    // Resolve freeze file path.
    let freeze_file_path = state_dir.join("embedded-agent").join("update-freeze");

    // Background task channel.
    let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel::<ServiceMessage>(32);

    // Send Register.
    if let Err(e) = transport
        .transport_send(ServiceMessage::Register(RegisterPayload::new(caps.clone())))
        .await
    {
        tracing::error!(error = %e, "embedded agent: failed to send Register");
        return;
    }

    // Send ReportHosts.
    let report = ReportHostsPayload {
        hosts: vec![host_info],
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: caps.clone(),
    };
    if let Err(e) = transport
        .transport_send_auto_paginate(ServiceMessage::ReportHosts(report))
        .await
    {
        tracing::error!(error = %e, "embedded agent: failed to send ReportHosts");
        return;
    }

    tracing::info!(
        %machine_id,
        "embedded agent started"
    );

    let mut in_flight_update: Option<InFlightUpdate> = None;
    let mut last_update_accepted: Option<std::time::Instant> = None;
    let ctx = ConnectionContext::default();

    loop {
        tokio::select! {
            biased;

            // Cancellation (shutdown).
            () = cancel.cancelled() => {
                tracing::info!("embedded agent: shutting down");
                uptrakit_agent_core::handle_graceful_shutdown(
                    &mut transport,
                    in_flight_update,
                    SHUTDOWN_TIMEOUT,
                    DisconnectReason::Shutdown,
                    uptrakit_agent_core::LoopOutcome::Shutdown,
                )
                .await;
                break;
            }

            // In-flight update events (highest priority after cancel).
            event = poll_in_flight_update(&mut in_flight_update) => {
                match event {
                    AgentEvent::Update(UpdateEvent::Output(output_msg)) => {
                        if let Some(ref update) = in_flight_update {
                            uptrakit_agent_core::send_update_output(
                                &mut transport,
                                update.update_history_id,
                                output_msg,
                            )
                            .await;
                        }
                    }
                    AgentEvent::Update(UpdateEvent::Completed(result)) => {
                        if let Some(update) = in_flight_update.take()
                            && let Err(e) = uptrakit_agent_core::send_update_result(
                                &mut transport,
                                update.update_history_id,
                                result,
                            )
                            .await
                        {
                            tracing::error!(error = %e, "embedded agent: failed to send UpdateResult");
                        }
                    }
                    AgentEvent::Update(UpdateEvent::Attention(update_history_id)) => {
                        #[cfg(feature = "interactive")]
                        {
                            transport
                                .transport_send_best_effort(ServiceMessage::StdinAttention(
                                    StdinAttentionPayload::new(update_history_id),
                                ))
                                .await;
                        }
                        #[cfg(not(feature = "interactive"))]
                        {
                            let _ = update_history_id;
                        }
                    }
                }
            }

            // Background task results.
            Some(msg) = bg_rx.recv() => {
                if let Err(e) = transport.transport_send_auto_paginate(msg).await {
                    tracing::error!(error = %e, "embedded agent: failed to send background result");
                }
            }

            // Controller messages.
            msg = transport.transport_recv() => {
                let Some(msg) = msg else {
                    tracing::info!("embedded agent: transport closed");
                    break;
                };

                // Skip processing when yielded to an external agent.
                if transport.is_yielded() {
                    tracing::debug!("embedded agent: yielded, ignoring controller message");
                    continue;
                }

                match msg {
                    ControllerMessage::CheckVersions(payload) => {
                        if payload.host_machine_id != machine_id {
                            tracing::warn!(
                                expected = %machine_id,
                                received = %payload.host_machine_id,
                                "security_audit: CheckVersions machine_id mismatch; ignoring"
                            );
                            continue;
                        }
                        uptrakit_agent_core::spawn_background(&bg_tx, {
                            let executor = Arc::clone(&executor);
                            let ctx = ctx.clone();
                            async move {
                                uptrakit_agent_core::run_check_versions(payload, executor, &ctx).await
                            }
                        });
                    }

                    ControllerMessage::ExecuteUpdate(payload) => {
                        if payload.host_machine_id != machine_id {
                            tracing::warn!(
                                expected = %machine_id,
                                received = %payload.host_machine_id,
                                "security_audit: ExecuteUpdate machine_id mismatch; ignoring"
                            );
                            continue;
                        }
                        if is_frozen(&freeze_file_path).await {
                            tracing::warn!(
                                update_id = %payload.update_history_id,
                                "security_audit: ExecuteUpdate rejected — updates are frozen"
                            );
                            continue;
                        }
                        if let Some(last) = last_update_accepted
                            && last.elapsed() < UPDATE_COOLDOWN
                        {
                            tracing::warn!(
                                update_id = %payload.update_history_id,
                                "security_audit: ExecuteUpdate rejected — rate limit"
                            );
                            continue;
                        }
                        last_update_accepted = Some(std::time::Instant::now());
                        uptrakit_agent_core::handle_execute_update(
                            *payload,
                            Arc::clone(&executor),
                            &mut in_flight_update,
                            &mut transport,
                            &ctx,
                        )
                        .await;
                    }

                    ControllerMessage::DiscoverSoftware(payload) => {
                        if payload.host_machine_id != machine_id {
                            tracing::warn!(
                                expected = %machine_id,
                                received = %payload.host_machine_id,
                                "security_audit: DiscoverSoftware machine_id mismatch; ignoring"
                            );
                            continue;
                        }
                        uptrakit_agent_core::spawn_background(&bg_tx, {
                            let executor = Arc::clone(&executor);
                            let ctx = ctx.clone();
                            async move {
                                uptrakit_agent_core::run_discover_software(payload, executor, &ctx).await
                            }
                        });
                    }

                    ControllerMessage::ExecuteBatchUpdate(payload) => {
                        if payload.host_machine_id != machine_id {
                            tracing::warn!(
                                expected = %machine_id,
                                received = %payload.host_machine_id,
                                "security_audit: ExecuteBatchUpdate machine_id mismatch; ignoring"
                            );
                            continue;
                        }
                        if is_frozen(&freeze_file_path).await {
                            tracing::warn!(
                                batch_id = %payload.batch_id,
                                "security_audit: ExecuteBatchUpdate rejected — updates are frozen"
                            );
                            continue;
                        }
                        if let Some(last) = last_update_accepted
                            && last.elapsed() < UPDATE_COOLDOWN
                        {
                            tracing::warn!(
                                batch_id = %payload.batch_id,
                                "security_audit: ExecuteBatchUpdate rejected — rate limit"
                            );
                            continue;
                        }
                        last_update_accepted = Some(std::time::Instant::now());
                        uptrakit_agent_core::spawn_background(&bg_tx, {
                            let executor = Arc::clone(&executor);
                            let ctx = ctx.clone();
                            async move {
                                uptrakit_agent_core::run_execute_batch_update(*payload, executor, &ctx).await
                            }
                        });
                    }

                    ControllerMessage::SetUpdateFreeze(payload) => {
                        handle_set_update_freeze(&freeze_file_path, payload).await;
                    }

                    #[cfg(feature = "interactive")]
                    ControllerMessage::UpdateStdinData(payload) => {
                        handle_update_stdin_data(payload, &in_flight_update);
                    }

                    // Ignore messages not relevant to the agent.
                    _ => {
                        tracing::trace!("embedded agent: ignoring unhandled controller message");
                    }
                }
            }
        }
    }

    // Drain any remaining background results (best-effort).
    while let Ok(msg) = bg_rx.try_recv() {
        transport.transport_send_best_effort(msg).await;
    }

    tracing::info!("embedded agent stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_capabilities_includes_expected_set() {
        let caps = agent_capabilities();
        assert!(caps.contains(&Capability::SoftwareDiscovery));
        assert!(caps.contains(&Capability::UpdateHooks));
        assert!(caps.contains(&Capability::GracefulShutdown));
    }

    #[cfg(feature = "interactive")]
    #[test]
    fn agent_capabilities_includes_interactive_when_feature_enabled() {
        let caps = agent_capabilities();
        assert!(caps.contains(&Capability::InteractiveUpdates));
    }

    #[tokio::test]
    async fn freeze_file_create_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let freeze_path = dir.path().join("embedded-agent").join("update-freeze");

        assert!(!is_frozen(&freeze_path).await);

        handle_set_update_freeze(
            &freeze_path,
            uptrakit_internal_wire::SetUpdateFreezePayload {
                enabled: true,
                reason: Some("test freeze".to_string()),
            },
        )
        .await;
        assert!(is_frozen(&freeze_path).await);

        handle_set_update_freeze(
            &freeze_path,
            uptrakit_internal_wire::SetUpdateFreezePayload {
                enabled: false,
                reason: None,
            },
        )
        .await;
        assert!(!is_frozen(&freeze_path).await);
    }
}
