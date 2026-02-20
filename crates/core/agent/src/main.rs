mod cli;
mod client;
mod error;
mod host_info;
mod update;
mod version_check;

use clap::Parser;
use uptrakit_internal_wire::{
    ControllerMessage, DisconnectReason, ReportHostsPayload, ServiceMessage, ServiceType,
};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, ServiceHandler, ServiceIdentityState, Signal,
};

use cli::Args;

struct AgentHandler {
    in_flight_update: Option<client::InFlightUpdate>,
}

#[async_trait::async_trait]
impl ServiceHandler for AgentHandler {
    const DIR_NAME: &'static str = "agent";
    const SERVICE_LABEL: &'static str = "uptrakit-agent service";
    const SERVICE_TYPE: ServiceType = ServiceType::Agent;

    type ServiceEvent = client::UpdateEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> std::result::Result<(), LoopError> {
        let host_info = crate::host_info::collect_host_info();
        conn.send(ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts: vec![host_info],
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: uptrakit_internal_wire::PROTOCOL_VERSION,
        }))
        .await
        .map_err(LoopError::from)?;
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
    ) -> std::result::Result<Option<LoopOutcome>, LoopError> {
        match msg {
            ControllerMessage::CheckVersions(payload) => {
                Ok(client::handle_check_versions(payload, conn).await)
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                client::handle_execute_update(*payload, &mut self.in_flight_update, conn).await;
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
    ) -> std::result::Result<Option<LoopOutcome>, LoopError> {
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

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        signal: Signal,
        shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = match signal {
            Signal::Hangup => (DisconnectReason::Restart, LoopOutcome::Restart),
            _ => (DisconnectReason::Shutdown, LoopOutcome::Shutdown),
        };
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

    uptrakit_service_sdk::init_tracing("uptrakit_agent=info");
    uptrakit_service_sdk::init_crypto();

    let mut handler = AgentHandler {
        in_flight_update: None,
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent",
        &args.common,
        &mut handler,
    )
    .await;
}
