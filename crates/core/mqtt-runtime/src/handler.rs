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
}

impl MqttHandler {
    pub fn new() -> Self {
        Self {
            runtime: MqttRuntime::new(),
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uptrakit_wire::{
        Capability, ControllerMessage, ServiceMessage, ServiceTransport, TransportError,
        payloads::ServiceSettingsPayload,
    };
    use uuid::Uuid;

    use super::MqttHandler;

    // ---------------------------------------------------------------------------
    // Test-local transport
    // ---------------------------------------------------------------------------

    /// Minimal in-process transport for handler integration tests.
    ///
    /// `ctrl_rx` carries messages from the test into the service (inbound).
    /// `svc_tx` forwards messages from the service back to the test (outbound).
    struct TestTransport {
        ctrl_rx: mpsc::Receiver<ControllerMessage>,
        svc_tx: mpsc::Sender<ServiceMessage>,
    }

    #[async_trait]
    impl ServiceTransport for TestTransport {
        async fn transport_send(&mut self, msg: ServiceMessage) -> Result<(), TransportError> {
            // Swallow send errors — the test receiver may have been dropped by the
            // time the service sends its last message (e.g. Disconnecting on shutdown).
            drop(self.svc_tx.send(msg).await);
            Ok(())
        }

        async fn transport_send_best_effort(&mut self, msg: ServiceMessage) {
            drop(self.svc_tx.try_send(msg));
        }

        async fn transport_send_auto_paginate(
            &mut self,
            msg: ServiceMessage,
        ) -> Result<(), TransportError> {
            self.transport_send(msg).await
        }

        async fn transport_recv(&mut self) -> Option<ControllerMessage> {
            self.ctrl_rx.recv().await
        }

        fn close_policy(&self) -> uptrakit_wire::TransportClosePolicy {
            uptrakit_wire::TransportClosePolicy::Shutdown
        }

        fn is_yielded(&self) -> bool {
            false
        }
    }

    /// Construct a linked `(TestTransport, ctrl_tx, svc_rx)` triple.
    ///
    /// * `ctrl_tx` — test side: enqueue `ControllerMessage`s into the service.
    /// * `svc_rx`  — test side: drain `ServiceMessage`s sent by the service.
    fn make_transport() -> (
        TestTransport,
        mpsc::Sender<ControllerMessage>,
        mpsc::Receiver<ServiceMessage>,
    ) {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(16);
        let (svc_tx, svc_rx) = mpsc::channel::<ServiceMessage>(64);
        (TestTransport { ctrl_rx, svc_tx }, ctrl_tx, svc_rx)
    }

    /// Build a minimal `ServiceSettingsPayload` that satisfies the embedded startup
    /// sequence. `extra_caps` are merged into the capability set so tests can
    /// control the agreed-capability negotiation outcome.
    fn make_settings(
        tenant_id: Option<Uuid>,
        extra_caps: impl IntoIterator<Item = Capability>,
    ) -> ServiceSettingsPayload {
        let mut settings =
            ServiceSettingsPayload::new(0, Duration::from_secs(60)).with_capabilities(extra_caps);
        if let Some(tid) = tenant_id {
            settings = settings.with_tenant_id(tid);
        }
        settings
    }

    // ---------------------------------------------------------------------------
    // Test 1 – drain-triggered shutdown sends Disconnecting
    // ---------------------------------------------------------------------------

    /// Verify that a graceful drain causes the embedded MQTT handler to emit
    /// `ServiceMessage::Disconnecting` with reason `Shutdown`.
    ///
    /// This mirrors the intent of the original `transport_close_still_runs_runtime_shutdown`
    /// test from `controller-runtime`. In the new `run_embedded_service` harness, `on_shutdown`
    /// is invoked on drain (not on transport close), so the trigger is the drain token.
    #[tokio::test]
    async fn drain_shutdown_sends_disconnecting() {
        let handler = MqttHandler::new();
        let (transport, ctrl_tx, mut svc_rx) = make_transport();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        // Send ServiceSettings first (required by the embedded startup sequence),
        // then immediately cancel the drain token.
        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings(
                None,
                std::iter::empty(),
            )))
            .await
            .expect("settings send must succeed");
        drain.cancel();

        uptrakit_service_sdk::run_embedded_service(
            uuid::Uuid::nil(),
            handler,
            transport,
            drain,
            abort,
        )
        .await;

        // Collect all outbound messages.
        let mut messages = Vec::new();
        svc_rx.close();
        while let Some(msg) = svc_rx.recv().await {
            messages.push(msg);
        }

        assert!(
            messages.iter().any(|msg| matches!(
                msg,
                ServiceMessage::Disconnecting(payload)
                    if payload.reason == uptrakit_wire::DisconnectReason::Shutdown
            )),
            "expected at least one Disconnecting(Shutdown) message; got: {messages:?}"
        );
    }

    // ---------------------------------------------------------------------------
    // Test 2 – ServiceSettings with UiSurfaces triggers SurfaceRegistration
    // ---------------------------------------------------------------------------

    /// Verify that when the embedded MQTT handler receives `ServiceSettings` with
    /// `UiSurfaces` capability and a tenant ID, it emits a `SurfaceRegistration`
    /// message whose `effective_tenant_binding.tenant_id` matches the tenant.
    #[tokio::test]
    async fn embedded_mqtt_registers_surface_with_default_tenant_binding() {
        let handler = MqttHandler::new();
        let (transport, ctrl_tx, mut svc_rx) = make_transport();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        let default_tenant_id = Uuid::now_v7();

        // Send settings that include UiSurfaces so the agreed set enables surface
        // registration, and supply the tenant ID that must appear in the binding.
        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings(
                Some(default_tenant_id),
                [Capability::UiSurfaces],
            )))
            .await
            .expect("settings send must succeed");

        // Drop the controller sender so the transport closes after settings are
        // processed, causing the event loop to exit naturally.
        drop(ctrl_tx);

        uptrakit_service_sdk::run_embedded_service(
            uuid::Uuid::nil(),
            handler,
            transport,
            drain,
            abort,
        )
        .await;

        let mut messages = Vec::new();
        svc_rx.close();
        while let Some(msg) = svc_rx.recv().await {
            messages.push(msg);
        }

        let register_pos = messages
            .iter()
            .position(|m| matches!(m, ServiceMessage::Register(_)));
        let surface_pos = messages
            .iter()
            .position(|m| matches!(m, ServiceMessage::SurfaceRegistration(_)));

        assert!(
            register_pos.is_some(),
            "expected ServiceMessage::Register from on_connected; got: {messages:?}"
        );
        assert!(
            surface_pos.is_some(),
            "expected ServiceMessage::SurfaceRegistration from on_settings; got: {messages:?}"
        );
        assert!(
            register_pos < surface_pos,
            "Register must be sent before SurfaceRegistration (on_connected before on_settings)"
        );

        let registrations: Vec<_> = messages
            .iter()
            .filter_map(|m| {
                if let ServiceMessage::SurfaceRegistration(reg) = m {
                    Some(reg)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            registrations.len(),
            1,
            "expected exactly one SurfaceRegistration; got {messages:?}"
        );
        let registration = &registrations[0];
        let expected_tenant = default_tenant_id.to_string();
        assert_eq!(
            registration.effective_tenant_binding.tenant_id.as_deref(),
            Some(expected_tenant.as_str()),
            "SurfaceRegistration tenant binding must match the configured default tenant"
        );
    }
}
