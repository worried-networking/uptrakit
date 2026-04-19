mod cli;

use clap::Parser;
use std::collections::BTreeSet;
use std::path::PathBuf;

use uptrakit_agent_runtime::{
    AgentRuntime, AgentRuntimeConfig, AgentRuntimeEvent, agent_capabilities, make_local_executor,
};
use uptrakit_audit_log::RuntimeAuditEmitter;
use uptrakit_internal_wire::Capability;
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, default_resolve_shutdown,
};

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
        conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        self.runtime
            .on_connected(conn)
            .await
            .map_err(|error| rootcause::Report::new(LoopError::Other(error.to_string())))
    }

    async fn on_settings(
        &mut self,
        _settings: &uptrakit_internal_wire::ServiceSettingsPayload,
        conn: &mut ControllerConnection,
    ) {
        if let Err(error) = self.runtime.send_pending_initial_report(conn).await {
            tracing::warn!(error = %error, "failed to send initial ReportHosts message");
        }
    }

    async fn on_message(
        &mut self,
        msg: uptrakit_internal_wire::ControllerMessage,
        conn: &mut ControllerConnection,
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
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self.runtime.handle_event(event, conn).await)
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
        self.runtime
            .shutdown(conn, shutdown_timeout, disconnect_reason, outcome)
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

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    let freeze_file_path = args
        .common
        .resolve_dirs("agent")
        .map(|dirs| dirs.state_dir().join("update-freeze"))
        .unwrap_or_else(|_| PathBuf::from("update-freeze"));

    let mut handler = AgentHandler {
        runtime: AgentRuntime::new(AgentRuntimeConfig::with_audit_emitter(
            make_local_executor(),
            freeze_file_path,
            env!("CARGO_PKG_VERSION").to_string(),
            RuntimeAuditEmitter::new(),
        )),
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent",
        &args.common,
        &mut handler,
    )
    .await;
}
