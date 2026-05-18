//! Entry point for running a [`ServiceHandler`] in embedded mode.
//!
//! Embedded services receive messages over an in-process channel
//! (`EmbeddedTransport`) instead of a WebSocket. They skip enrollment,
//! certificate management, and OS signal handling. Shutdown is driven by
//! two `CancellationToken`s: `drain` (graceful) and `abort` (immediate).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::shared_types::{LoopOutcome, ServiceHandler, ShutdownCause};
use crate::wire_api::{Capability, ControllerMessage, ServiceSettingsPayload, ServiceTransport};

/// Startup timeout for the initial `ServiceSettings` message.
const EMBEDDED_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Default shutdown timeout, used until `ServiceSettings` provides one.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);

/// Run a [`ServiceHandler`] in embedded mode.
///
/// Startup sequence:
/// 1. Wait up to 10 s for the controller to send `ServiceSettings` (first message).
/// 2. Generate an ephemeral P-256 keypair and build a [`ServiceIdentityState`].
/// 3. Call `on_connected` with the identity — mirrors the standalone lifecycle.
/// 4. Compute agreed capabilities (intersection of handler's and controller's).
/// 5. Call `on_settings`.
/// 6. Call `on_yield_change` with the transport's current yield state.
/// 7. Enter the two-phase event loop.
///
/// Exits when: drain fires (graceful via `on_shutdown`), abort fires
/// (immediate), transport closes, or a handler callback requests exit.
///
/// [`ServiceIdentityState`]: crate::identity::ServiceIdentityState
pub async fn run_embedded_service<H: ServiceHandler>(
    service_id: Uuid,
    mut handler: H,
    mut transport: impl ServiceTransport,
    drain: CancellationToken,
    abort: CancellationToken,
) {
    // ── Startup: wait for ServiceSettings ──────────────────────────────────
    let first_msg = tokio::select! {
        biased;
        () = abort.cancelled() => return,
        result = tokio::time::timeout(
            EMBEDDED_STARTUP_TIMEOUT,
            transport.transport_recv(),
        ) => {
            match result {
                Err(_elapsed) => {
                    tracing::error!(
                        service = H::SERVICE_LABEL,
                        "embedded service did not receive ServiceSettings within 10s; aborting"
                    );
                    return;
                }
                Ok(None) => {
                    tracing::error!(
                        service = H::SERVICE_LABEL,
                        "embedded transport closed before ServiceSettings arrived"
                    );
                    return;
                }
                Ok(Some(msg)) => msg,
            }
        }
    };

    let settings = match first_msg {
        ControllerMessage::ServiceSettings(s) => s,
        other => {
            tracing::warn!(
                service = H::SERVICE_LABEL,
                ?other,
                "expected ServiceSettings as first embedded message; aborting"
            );
            return;
        }
    };

    let mut shutdown_timeout = settings
        .shutdown_timeout
        .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);

    // ── Identity + on_connected ─────────────────────────────────────────────
    {
        // P-256 is intentional: sealed_box_decrypt in sensitive_params.rs is
        // hardcoded to ECDH_P256.
        let keypair = match rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256) {
            Ok(kp) => kp,
            Err(e) => {
                tracing::error!(
                    service = H::SERVICE_LABEL,
                    error = %e,
                    "failed to generate embedded service keypair; aborting"
                );
                return;
            }
        };
        let identity = crate::identity::ServiceIdentityState::for_embedded(service_id, keypair);
        if let Err(e) = handler.on_connected(&mut transport, &identity).await {
            tracing::error!(
                service = H::SERVICE_LABEL,
                error = %e,
                "embedded on_connected failed; aborting"
            );
            return;
        }
    } // identity and keypair dropped here

    let agreed = compute_agreed_capabilities(&handler, &settings);

    handler
        .on_settings(&settings, &mut transport, &agreed)
        .await;

    // Initial yield notification.
    {
        let is_yielded = transport.is_yielded();
        handler.on_yield_change(is_yielded, &mut transport).await;
    }

    // ── Obtain yield notifier handle ───────────────────────────────────────
    let yield_notifier: Option<Arc<tokio::sync::Notify>> = transport.yield_change_notifier();

    // ── Event loop ─────────────────────────────────────────────────────────
    loop {
        // Phase 1: resolve the next event.
        let maybe_event = tokio::select! {
            biased;
            () = abort.cancelled() => break,
            () = drain.cancelled() => {
                handler
                    .on_shutdown(&mut transport, ShutdownCause::EmbeddedDrain, shutdown_timeout)
                    .await;
                break;
            }
            () = async {
                if let Some(n) = &yield_notifier { n.notified().await }
                // `pending()` makes this arm never-ready when yield signalling is
                // unsupported. Do NOT replace with `unreachable!()` or `unwrap()`.
                else { std::future::pending().await }
            } => {
                let is_yielded = transport.is_yielded();
                handler.on_yield_change(is_yielded, &mut transport).await;
                None
            }
            event = handler.poll_service_event() => Some(event),
            msg = transport.transport_recv() => {
                match msg {
                    None => break,
                    Some(msg) => {
                        if !transport.is_yielded()
                            && let Some(outcome) =
                                dispatch_message(msg, &mut handler, &mut transport, &mut shutdown_timeout).await
                        {
                            let _ = outcome;
                            break;
                        }
                        None
                    }
                }
            }
        };

        // Phase 2: run on_service_event with drain/abort guards.
        if let Some(event) = maybe_event {
            let should_break = tokio::select! {
                biased;
                () = abort.cancelled() => true,
                () = drain.cancelled() => {
                    handler
                        .on_shutdown(&mut transport, ShutdownCause::EmbeddedDrain, shutdown_timeout)
                        .await;
                    true
                }
                outcome = handler.on_service_event(event, &mut transport) => {
                    match outcome {
                        Ok(Some(_)) => true,
                        Ok(None) => false,
                        Err(e) => {
                            tracing::error!(
                                service = H::SERVICE_LABEL,
                                error = %e,
                                "embedded service event handler error; exiting"
                            );
                            true
                        }
                    }
                }
            };
            if should_break {
                break;
            }
        }
    }
}

fn compute_agreed_capabilities<H: ServiceHandler>(
    handler: &H,
    settings: &ServiceSettingsPayload,
) -> BTreeSet<Capability> {
    settings
        .capabilities
        .intersection(&handler.capabilities())
        .filter(|c| c.is_known())
        .cloned()
        .collect()
}

async fn dispatch_message<H: ServiceHandler>(
    msg: ControllerMessage,
    handler: &mut H,
    transport: &mut dyn ServiceTransport,
    shutdown_timeout: &mut Duration,
) -> Option<LoopOutcome> {
    match msg {
        ControllerMessage::ServiceSettings(settings) => {
            *shutdown_timeout = settings
                .shutdown_timeout
                .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);
            let agreed = compute_agreed_capabilities(handler, &settings);
            handler.on_settings(&settings, transport, &agreed).await;
            let is_yielded = transport.is_yielded();
            handler.on_yield_change(is_yielded, transport).await;
            None
        }
        ControllerMessage::SurfaceActionRequest(payload) => {
            if let Err(e) = handler.on_surface_action_request(payload, transport).await {
                tracing::error!(error = %e, "surface action request handler error");
            }
            None
        }
        ControllerMessage::SurfaceActionResponse(payload) => {
            handler.on_surface_action_response(payload);
            None
        }
        ControllerMessage::ServiceConfigAck(ack) => {
            handler.on_service_config_ack(ack);
            None
        }
        ControllerMessage::ServerRestarting(_payload) => {
            tracing::info!(
                service = H::SERVICE_LABEL,
                "controller restarting; embedded service shutting down"
            );
            let outcome = handler
                .on_shutdown(
                    transport,
                    ShutdownCause::ServerRestarting,
                    *shutdown_timeout,
                )
                .await;
            Some(outcome)
        }
        ControllerMessage::Unknown => {
            tracing::warn!(
                service = H::SERVICE_LABEL,
                "received unknown controller message type in embedded mode; ignoring"
            );
            None
        }
        ControllerMessage::Certificate(_)
        | ControllerMessage::CaBundleUpdated(_)
        | ControllerMessage::Pong(_)
        | ControllerMessage::RequestCertRenewal(_) => {
            tracing::debug!(
                service = H::SERVICE_LABEL,
                "ignoring cert/CA/pong message in embedded mode"
            );
            None
        }
        msg => match handler.on_message(msg, transport).await {
            Ok(Some(outcome)) => Some(outcome),
            Ok(None) => None,
            Err(e) => {
                tracing::error!(service = H::SERVICE_LABEL, error = %e, "on_message error");
                None
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::shared_types::{LoopOutcome, LoopResult, ServiceHandler, ShutdownCause};
    use crate::wire_api::{
        Capability, ControllerMessage, ServiceMessage, ServiceSettingsPayload, ServiceTransport,
        TransportError,
    };

    use super::run_embedded_service;

    struct MockTransport {
        svc_rx: mpsc::Receiver<ControllerMessage>,
        yielded: bool,
    }

    fn make_transport(ctrl_in: mpsc::Receiver<ControllerMessage>) -> MockTransport {
        MockTransport {
            svc_rx: ctrl_in,
            yielded: false,
        }
    }

    fn make_yielded_transport(ctrl_in: mpsc::Receiver<ControllerMessage>) -> MockTransport {
        MockTransport {
            svc_rx: ctrl_in,
            yielded: true,
        }
    }

    #[async_trait]
    impl ServiceTransport for MockTransport {
        async fn transport_send(&mut self, _msg: ServiceMessage) -> Result<(), TransportError> {
            Ok(())
        }
        async fn transport_send_best_effort(&mut self, _msg: ServiceMessage) {}
        async fn transport_send_auto_paginate(
            &mut self,
            msg: ServiceMessage,
        ) -> Result<(), TransportError> {
            self.transport_send(msg).await
        }
        async fn transport_recv(&mut self) -> Option<ControllerMessage> {
            self.svc_rx.recv().await
        }
        fn close_policy(&self) -> crate::wire_api::TransportClosePolicy {
            crate::wire_api::TransportClosePolicy::Shutdown
        }
        fn is_yielded(&self) -> bool {
            self.yielded
        }
    }

    #[derive(Debug, Default)]
    struct CallLog {
        call_order: Vec<&'static str>,
        on_settings_called: bool,
        on_shutdown_called: bool,
        on_yield_change_called: bool,
        on_message_called: bool,
    }

    struct MockHandler {
        log: std::sync::Arc<parking_lot::Mutex<CallLog>>,
        on_connected_result: LoopResult<()>,
    }

    impl MockHandler {
        fn new() -> (Self, std::sync::Arc<parking_lot::Mutex<CallLog>>) {
            let log = std::sync::Arc::new(parking_lot::Mutex::new(CallLog::default()));
            (
                Self {
                    log: log.clone(),
                    on_connected_result: Ok(()),
                },
                log,
            )
        }

        fn new_failing_connected() -> (Self, std::sync::Arc<parking_lot::Mutex<CallLog>>) {
            let log = std::sync::Arc::new(parking_lot::Mutex::new(CallLog::default()));
            (
                Self {
                    log: log.clone(),
                    on_connected_result: Err(rootcause::report!(
                        crate::shared_types::LoopError::Other("on_connected failed".to_string())
                    )),
                },
                log,
            )
        }
    }

    #[async_trait]
    impl ServiceHandler for MockHandler {
        const DIR_NAME: &'static str = "mock";
        const SERVICE_LABEL: &'static str = "mock service";
        const SERVICE_APP_NAME: &'static str = "mock";

        type ServiceEvent = std::convert::Infallible;

        async fn on_connected(
            &mut self,
            _conn: &mut dyn ServiceTransport,
            _identity: &crate::identity::ServiceIdentityState,
        ) -> LoopResult<()> {
            self.log.lock().call_order.push("on_connected");
            match &self.on_connected_result {
                Ok(()) => Ok(()),
                Err(e) => Err(rootcause::report!(crate::shared_types::LoopError::Other(
                    e.to_string()
                ))),
            }
        }

        async fn on_message(
            &mut self,
            _msg: ControllerMessage,
            _conn: &mut dyn ServiceTransport,
        ) -> LoopResult<Option<LoopOutcome>> {
            self.log.lock().on_message_called = true;
            Ok(None)
        }

        async fn on_settings(
            &mut self,
            _settings: &ServiceSettingsPayload,
            _conn: &mut dyn ServiceTransport,
            _agreed: &BTreeSet<Capability>,
        ) {
            let mut log = self.log.lock();
            log.on_settings_called = true;
            log.call_order.push("on_settings");
        }

        async fn poll_service_event(&mut self) -> Self::ServiceEvent {
            std::future::pending().await
        }

        async fn on_service_event(
            &mut self,
            event: Self::ServiceEvent,
            _conn: &mut dyn ServiceTransport,
        ) -> LoopResult<Option<LoopOutcome>> {
            match event {}
        }

        async fn on_shutdown(
            &mut self,
            _conn: &mut dyn ServiceTransport,
            _cause: ShutdownCause,
            _timeout: Duration,
        ) -> LoopOutcome {
            self.log.lock().on_shutdown_called = true;
            LoopOutcome::Shutdown
        }

        async fn on_yield_change(&mut self, _is_yielded: bool, _conn: &mut dyn ServiceTransport) {
            self.log.lock().on_yield_change_called = true;
        }
    }

    fn make_settings() -> ServiceSettingsPayload {
        ServiceSettingsPayload::new(0, Duration::from_secs(60))
    }

    #[tokio::test]
    async fn exits_when_transport_closed_before_settings() {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(1);
        drop(ctrl_tx);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;

        let log = log.lock();
        assert!(!log.on_settings_called, "on_settings must not be called");
        assert!(!log.on_shutdown_called, "on_shutdown must not be called");
    }

    #[tokio::test]
    async fn abort_before_settings_exits_immediately() {
        let (_ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(1);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();
        abort.cancel();

        run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;

        assert!(!log.lock().on_settings_called);
    }

    #[tokio::test]
    async fn normal_startup_then_drain_calls_on_shutdown() {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings()))
            .await
            .expect("send settings");
        drain.cancel();

        run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;

        let log = log.lock();
        assert!(log.on_settings_called, "on_settings must be called");
        assert!(
            log.on_shutdown_called,
            "on_shutdown must be called on drain"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn startup_timeout_exits_without_callback() {
        let (_ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(1);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        let task = tokio::spawn(run_embedded_service(
            uuid::Uuid::nil(),
            handler,
            transport,
            drain,
            abort,
        ));

        tokio::time::advance(Duration::from_secs(11)).await;

        task.await.expect("task panicked");

        assert!(!log.lock().on_settings_called);
        assert!(!log.lock().on_shutdown_called);
    }

    #[tokio::test]
    async fn yielded_transport_drops_messages_silently() {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
        let transport = make_yielded_transport(ctrl_rx);
        let (handler, log) = MockHandler::new();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings()))
            .await
            .expect("send settings");
        ctrl_tx
            .send(ControllerMessage::Unknown)
            .await
            .expect("send unknown");
        drop(ctrl_tx);

        run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;

        let log = log.lock();
        assert!(log.on_settings_called, "on_settings must be called");
        assert!(
            !log.on_message_called,
            "on_message must NOT be called when yielded"
        );
        assert!(
            !log.on_shutdown_called,
            "on_shutdown not called on transport close"
        );
    }

    #[tokio::test]
    async fn on_connected_called_before_on_settings() {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings()))
            .await
            .expect("send settings");
        drain.cancel();

        run_embedded_service(uuid::Uuid::new_v4(), handler, transport, drain, abort).await;

        let log = log.lock();
        let connected_pos = log.call_order.iter().position(|&s| s == "on_connected");
        let settings_pos = log.call_order.iter().position(|&s| s == "on_settings");
        let ci = connected_pos.expect("on_connected must be in call_order");
        let si = settings_pos.expect("on_settings must be in call_order");
        assert!(ci < si, "on_connected must be called before on_settings");
    }

    #[tokio::test]
    async fn abort_when_on_connected_returns_err() {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new_failing_connected();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings()))
            .await
            .expect("send settings");

        run_embedded_service(uuid::Uuid::new_v4(), handler, transport, drain, abort).await;

        let log = log.lock();
        assert!(
            log.call_order.contains(&"on_connected"),
            "on_connected must be called"
        );
        assert!(
            !log.on_settings_called,
            "on_settings must NOT be called when on_connected fails"
        );
        assert!(
            !log.on_shutdown_called,
            "on_shutdown must NOT be called when on_connected fails"
        );
    }
}
