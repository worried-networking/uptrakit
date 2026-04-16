mod cli;

use base64::Engine as _;
use clap::Parser;
use rootcause::prelude::*;

use uptrakit_internal_wire::{Capability, ControllerMessage};
use uptrakit_mqtt_runtime::{
    MQTT_DIR_NAME, MQTT_SERVICE_APP_NAME, MQTT_SERVICE_LABEL, MqttRuntime, MqttRuntimeIdentity,
    MqttRuntimeLoopOutcome, MqttRuntimeSettings, mqtt_capabilities,
};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, default_resolve_shutdown,
};

struct StandaloneMqttHandler {
    runtime: MqttRuntime,
}

#[async_trait::async_trait]
impl ServiceHandler for StandaloneMqttHandler {
    const DIR_NAME: &'static str = MQTT_DIR_NAME;
    const SERVICE_LABEL: &'static str = MQTT_SERVICE_LABEL;
    const SERVICE_APP_NAME: &'static str = MQTT_SERVICE_APP_NAME;

    type ServiceEvent = Option<uptrakit_mqtt_runtime::MqttServiceEvent>;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        self.runtime
            .on_connected(
                conn,
                MqttRuntimeIdentity {
                    service_id: identity.service_id(),
                    private_key_der: identity.private_key_pkcs8_der(),
                    encryption_public_key: identity
                        .public_key_raw()
                        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
                },
            )
            .await
            .map_err(|_| report!(LoopError::Other("failed to send MQTT register".to_string())))
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        self.runtime
            .handle_controller_message(msg, conn)
            .await
            .map(|outcome| outcome.map(map_runtime_outcome))
            .map_err(|_| {
                report!(LoopError::Other(
                    "failed to handle MQTT controller message".to_string()
                ))
            })
    }

    async fn on_settings(
        &mut self,
        settings: &uptrakit_internal_wire::ServiceSettingsPayload,
        conn: &mut ControllerConnection,
    ) {
        self.runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: conn
                        .agreed_capabilities()
                        .contains(&Capability::UiSurfaces),
                    tenant_id: settings.tenant_id,
                },
                conn,
            )
            .await;
    }

    fn capabilities(&self) -> std::collections::BTreeSet<Capability> {
        mqtt_capabilities()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.runtime.poll_event().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self
            .runtime
            .handle_event(event, conn)
            .await
            .map(map_runtime_outcome))
    }

    fn on_surface_action_response(
        &mut self,
        response: uptrakit_internal_wire::surfaces::SurfaceActionResponse,
    ) {
        self.runtime.on_surface_action_response(response);
    }

    fn on_service_config_ack(
        &self,
        ack: uptrakit_internal_wire::payloads::ServiceConfigAckPayload,
    ) {
        self.runtime.on_service_config_ack(ack);
    }

    async fn on_surface_action_request(
        &mut self,
        request: uptrakit_internal_wire::surfaces::SurfaceActionRequest,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        self.runtime
            .handle_controller_message(ControllerMessage::SurfaceActionRequest(request), conn)
            .await
            .map(|_| ())
            .map_err(|_| {
                report!(LoopError::Other(
                    "failed to handle MQTT surface action request".to_string()
                ))
            })
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        _shutdown_timeout: std::time::Duration,
    ) -> LoopOutcome {
        let (reason, outcome) = default_resolve_shutdown(cause);
        self.runtime.shutdown(conn, reason).await;
        outcome
    }
}

fn map_runtime_outcome(outcome: MqttRuntimeLoopOutcome) -> LoopOutcome {
    match outcome {
        MqttRuntimeLoopOutcome::Disconnected => LoopOutcome::Disconnected,
    }
}

#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    let _ = args.max_tenants;

    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            MQTT_SERVICE_APP_NAME,
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    tracing::info!("starting uptrakit-mqtt service");

    let mut handler = StandaloneMqttHandler {
        runtime: MqttRuntime::new(),
    };

    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        MQTT_SERVICE_APP_NAME,
        &args.common,
        &mut handler,
    )
    .await;
}
