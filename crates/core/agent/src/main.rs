mod cli;
mod client;
mod host_info;

use clap::Parser;
use rootcause::prelude::*;
use std::collections::BTreeSet;

use uptrakit_internal_wire::{
    Capability, ControllerMessage, DisconnectReason, ReportHostsPayload, ServiceMessage,
};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, Signal,
};

use cli::Args;

/// Initialize `tracing_subscriber` with a verbosity-aware filter.
///
/// Verbosity levels expand scope progressively, keeping third-party crates
/// silent unless `RUST_LOG` explicitly enables them:
///
/// - `verbosity == 0`: `{own_module}=info`
/// - `verbosity == 1`: `{own_module}=debug`
/// - `verbosity == 2`: `uptrakit=debug`
/// - `verbosity >= 3`: `uptrakit=trace`
fn init_tracing(own_module: &str, verbosity: u8) {
    use tracing_subscriber::EnvFilter;

    if verbosity > 3 {
        eprintln!(
            "warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)"
        );
    }

    let directive = match verbosity {
        0 => format!("{own_module}=info"),
        1 => format!("{own_module}=debug"),
        2 => "uptrakit=debug".to_string(),
        _ => "uptrakit=trace".to_string(),
    };
    let mut filter = EnvFilter::from_default_env();
    if let Ok(d) = directive.parse() {
        filter = filter.add_directive(d);
    }
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

struct AgentHandler {
    /// Local machine ID, collected once on connect and used to validate
    /// incoming `host_machine_id` fields as a defensive sanity check.
    machine_id: String,
    in_flight_update: Option<client::InFlightUpdate>,
}

#[async_trait::async_trait]
impl ServiceHandler for AgentHandler {
    const DIR_NAME: &'static str = "agent";
    const SERVICE_LABEL: &'static str = "uptrakit-agent service";

    type ServiceEvent = client::UpdateEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let host_info = crate::host_info::collect_host_info();
        // Capture and store the machine_id for use in on_message() validation.
        self.machine_id = host_info.machine_id.clone();
        conn.send(ServiceMessage::ReportHosts(ReportHostsPayload {
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
                Ok(client::handle_check_versions(payload, conn).await)
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
                client::handle_execute_update(*payload, &mut self.in_flight_update, conn).await;
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
                Ok(client::handle_discover_software(payload, conn).await)
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
                client::send_update_result(conn, update_history_id, result).await;
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
        shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = resolve_shutdown(cause);
        client::handle_graceful_shutdown(
            conn,
            self.in_flight_update.take(),
            shutdown_timeout_seconds,
            disconnect_reason,
            outcome,
        )
        .await
    }
}

/// Map a [`ShutdownCause`] to the appropriate [`DisconnectReason`] and
/// [`LoopOutcome`] for this service.
///
/// | Cause | `DisconnectReason` | `LoopOutcome` |
/// | --- | --- | --- |
/// | `Signal(Hangup)` | `Restart` | `Restart` |
/// | `Signal(_)` | `Shutdown` | `Shutdown` |
/// | `ServerRestarting` | `Restart` | `Disconnected` |
fn resolve_shutdown(cause: ShutdownCause) -> (DisconnectReason, LoopOutcome) {
    match cause {
        ShutdownCause::Signal(Signal::Hangup) => (DisconnectReason::Restart, LoopOutcome::Restart),
        ShutdownCause::Signal(_) => (DisconnectReason::Shutdown, LoopOutcome::Shutdown),
        ShutdownCause::ServerRestarting => (DisconnectReason::Restart, LoopOutcome::Disconnected),
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

    init_tracing("uptrakit_agent", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    let mut handler = AgentHandler {
        machine_id: String::new(),
        in_flight_update: None,
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent",
        &args.common,
        &mut handler,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_shutdown_hangup() {
        let (reason, outcome) = resolve_shutdown(ShutdownCause::Signal(Signal::Hangup));
        assert_eq!(reason, DisconnectReason::Restart);
        assert_eq!(outcome, LoopOutcome::Restart);
    }

    #[test]
    fn resolve_shutdown_terminate() {
        let (reason, outcome) = resolve_shutdown(ShutdownCause::Signal(Signal::Terminate));
        assert_eq!(reason, DisconnectReason::Shutdown);
        assert_eq!(outcome, LoopOutcome::Shutdown);
    }

    #[test]
    fn resolve_shutdown_server_restarting() {
        let (reason, outcome) = resolve_shutdown(ShutdownCause::ServerRestarting);
        assert_eq!(reason, DisconnectReason::Restart);
        assert_eq!(outcome, LoopOutcome::Disconnected);
    }
}
