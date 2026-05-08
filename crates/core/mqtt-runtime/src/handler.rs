use std::collections::BTreeSet;
use std::time::Duration;

use base64::Engine as _;
use rootcause::prelude::*;

use crate::{
    MqttRuntime, MqttRuntimeIdentity, MqttRuntimeLoopOutcome, MqttRuntimeSettings,
    mqtt_capabilities,
};
use uptrakit_service_sdk::{
    LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState, ShutdownCause,
    default_resolve_shutdown,
};
use uptrakit_wire::{
    Capability, ControllerMessage, ServiceTransport, payloads::ServiceConfigAckPayload,
};

pub struct MqttHandler {
    runtime: MqttRuntime,
    embedded_identity: Option<MqttRuntimeIdentity>,
}

impl MqttHandler {
    pub fn new() -> Self {
        Self {
            runtime: MqttRuntime::new(),
            embedded_identity: None,
        }
    }

    pub fn new_embedded(identity: MqttRuntimeIdentity) -> Self {
        Self {
            runtime: MqttRuntime::new(),
            embedded_identity: Some(identity),
        }
    }
}

impl Default for MqttHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ServiceHandler for MqttHandler {
    const DIR_NAME: &'static str = crate::MQTT_DIR_NAME;
    const SERVICE_LABEL: &'static str = crate::MQTT_SERVICE_LABEL;
    const SERVICE_APP_NAME: &'static str = crate::MQTT_SERVICE_APP_NAME;

    type ServiceEvent = Option<crate::MqttServiceEvent>;

    #[expect(
        clippy::map_err_ignore,
        reason = "internal runtime errors are mapped to LoopError::Other(String) with a descriptive message; the original error type is not part of the public interface"
    )]
    async fn on_connected(
        &mut self,
        conn: &mut dyn ServiceTransport,
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

    #[expect(
        clippy::map_err_ignore,
        reason = "internal runtime errors are mapped to LoopError::Other(String) with a descriptive message; the original error type is not part of the public interface"
    )]
    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut dyn ServiceTransport,
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
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        agreed_capabilities: &BTreeSet<Capability>,
    ) {
        if let Some(identity) = self.embedded_identity.take()
            && let Err(e) = self.runtime.on_connected(conn, identity).await
        {
            tracing::error!(error = %e, "embedded MQTT: failed to initialize runtime");
            return;
        }
        self.runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: agreed_capabilities.contains(&Capability::UiSurfaces),
                    tenant_id: settings.tenant_id,
                },
                conn,
            )
            .await;
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        mqtt_capabilities()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.runtime.poll_event().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self
            .runtime
            .handle_event(event, conn)
            .await
            .map(map_runtime_outcome))
    }

    fn on_surface_action_response(
        &mut self,
        response: uptrakit_wire::surfaces::SurfaceActionResponse,
    ) {
        self.runtime.on_surface_action_response(response);
    }

    fn on_service_config_ack(&self, ack: ServiceConfigAckPayload) {
        self.runtime.on_service_config_ack(ack);
    }

    async fn on_yield_change(&mut self, is_yielded: bool, conn: &mut dyn ServiceTransport) {
        self.runtime.handle_yield_change(is_yielded, conn).await;
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "internal runtime errors are mapped to LoopError::Other(String) with a descriptive message; the original error type is not part of the public interface"
    )]
    async fn on_surface_action_request(
        &mut self,
        request: uptrakit_wire::surfaces::SurfaceActionRequest,
        conn: &mut dyn ServiceTransport,
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
        conn: &mut dyn ServiceTransport,
        cause: ShutdownCause,
        _shutdown_timeout: Duration,
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
