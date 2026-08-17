mod cli;

use clap::Parser;
use std::collections::BTreeSet;
use std::path::PathBuf;

use uptrakit_agent_runtime::{
    AgentRuntime, AgentRuntimeConfig, AgentRuntimeEvent, agent_capabilities, make_local_executor,
};
use uptrakit_audit_log::RuntimeAuditEmitter;
use uptrakit_service_sdk::{
    LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState, ShutdownCause,
    default_resolve_shutdown,
};
use uptrakit_wire::{Capability, ServiceTransport};

use cli::Args;

struct AgentHandler {
    runtime: AgentRuntime,
}

#[async_trait::async_trait]
impl ServiceHandler for AgentHandler {
    const DIR_NAME: &'static str = "agent";
    const SERVICE_LABEL: &'static str = "uptrakit-agent service";
    const SERVICE_APP_NAME: &'static str = env!("CARGO_PKG_NAME");

    type ServiceEvent = AgentRuntimeEvent;

    async fn on_connected(
        &mut self,
        conn: &mut dyn ServiceTransport,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        self.runtime
            .on_connected(conn)
            .await
            .map_err(|error| rootcause::Report::new(LoopError::Other(error.to_string())))
    }

    async fn on_settings(
        &mut self,
        _settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        _agreed_capabilities: &std::collections::BTreeSet<Capability>,
    ) {
        if let Err(error) = self.runtime.send_pending_initial_report(conn).await {
            tracing::warn!(error = %error, "failed to send initial ReportHosts message");
        }
    }

    async fn on_message(
        &mut self,
        msg: uptrakit_wire::ControllerMessage,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        self.runtime.handle_controller_message(msg, conn).await;
        Ok(None)
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.runtime.poll_event().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self.runtime.handle_event(event, conn).await)
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        agent_capabilities()
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut dyn ServiceTransport,
        cause: ShutdownCause,
        shutdown_timeout: std::time::Duration,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
        self.runtime
            .shutdown(conn, shutdown_timeout, disconnect_reason, outcome)
            .await
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let info = uptrakit_build_info::build_info!();
    if args.common.version {
        print!("{}", info.render_human());
        return std::process::ExitCode::SUCCESS;
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    if let Some(ref command) = args.command {
        return match uptrakit_agent_runtime::cli::run_command(command).await {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(report) => {
                eprintln!("agent command error: {report:?}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    let freeze_file_path = args
        .common
        .resolve_dirs("agent")
        .map(|dirs| dirs.state_dir().join("update-freeze"))
        .unwrap_or_else(|_| PathBuf::from("update-freeze"));

    let mut handler = AgentHandler {
        runtime: AgentRuntime::new(AgentRuntimeConfig::with_audit_emitter(
            make_local_executor(),
            freeze_file_path,
            info.version.clone(),
            RuntimeAuditEmitter::new(),
        )),
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent",
        &args.common,
        &mut handler,
    )
    .await;
    std::process::ExitCode::SUCCESS
}
