//! Embedded agent service for single-tenant controller deployments.

use std::collections::BTreeSet;
use std::path::PathBuf;

use tokio_util::sync::CancellationToken;
use uptrakit_agent_runtime::{
    AgentRuntime, AgentRuntimeConfig, agent_capabilities as runtime_capabilities,
    make_local_executor,
};
use uptrakit_internal_wire::{Capability, DisconnectReason, ServiceTransport};

use crate::embedded::types::EmbeddedTransport;

/// Timeout for graceful shutdown: how long to wait for an in-flight update.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Build the embedded agent capability set.
pub(crate) fn agent_capabilities() -> BTreeSet<Capability> {
    runtime_capabilities()
}

/// Run the embedded agent event loop.
pub(crate) async fn run_embedded_agent(
    mut transport: EmbeddedTransport,
    cancel: CancellationToken,
    state_dir: PathBuf,
) {
    let mut runtime = AgentRuntime::new(AgentRuntimeConfig::new(
        make_local_executor(),
        state_dir.join("embedded-agent").join("update-freeze"),
        env!("CARGO_PKG_VERSION").to_string(),
    ));

    if let Err(error) = runtime.on_connected(&mut transport).await {
        tracing::error!(error = %error, "embedded agent: failed to initialize runtime");
        return;
    }
    if let Err(error) = runtime.send_pending_initial_report(&mut transport).await {
        tracing::error!(error = %error, "embedded agent: failed to send initial ReportHosts");
        return;
    }

    tracing::info!(
        machine_id = %runtime.machine_id().unwrap_or(""),
        "embedded agent started"
    );

    loop {
        tokio::select! {
            biased;

            () = cancel.cancelled() => {
                tracing::info!("embedded agent: shutting down");
                runtime
                    .shutdown(
                        &mut transport,
                        SHUTDOWN_TIMEOUT,
                        DisconnectReason::Shutdown,
                        uptrakit_agent_core::LoopOutcome::Shutdown,
                    )
                    .await;
                break;
            }

            event = runtime.poll_event() => {
                if let Some(outcome) = runtime.handle_event(event, &mut transport).await {
                    tracing::warn!(?outcome, "embedded agent: runtime requested loop exit");
                    break;
                }
            }

            msg = transport.transport_recv() => {
                let Some(msg) = msg else {
                    tracing::info!("embedded agent: transport closed");
                    break;
                };

                if transport.is_yielded() {
                    tracing::debug!("embedded agent: yielded, ignoring controller message");
                    continue;
                }

                runtime.handle_controller_message(msg, &mut transport).await;
            }
        }
    }

    runtime.drain_background_results(&mut transport).await;
    tracing::info!("embedded agent stopped");
}
