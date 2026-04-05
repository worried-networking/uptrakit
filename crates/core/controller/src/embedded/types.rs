//! Local types for the embedded service infrastructure.
//!
//! These types live entirely within the controller crate — no new shared
//! dependencies are introduced.
//!
//! Some types and methods are infrastructure-only and will be exercised by
//! follow-up service embeddings (agent, mqtt, agent-ssh).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc;
use uptrakit_internal_wire::{Capability, ControllerMessage, ServiceMessage};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// EmbeddedTransport
// ---------------------------------------------------------------------------

/// In-process transport handle for embedded services.
///
/// Provides the same send/recv interface as a WebSocket connection but backed
/// by mpsc channels. The `yielded` flag is set by the `EmbeddedServiceHost`
/// when the coexistence policy dictates that this embedded service should
/// defer to an external counterpart.
///
/// `transport_recv()` follows the Layer-1 `Option` contract:
/// `None` means the inbound channel sender was dropped. In normal shutdown
/// flow, the shutdown signal resolves before channel closure. The host
/// cancels and joins tasks before releasing senders. If `None` is reached,
/// the embedded service loop exits without reconnection.
#[allow(dead_code)] // Transport methods are used by service closures, not directly by the host.
pub(crate) struct EmbeddedTransport {
    tx: mpsc::Sender<ServiceMessage>,
    rx: mpsc::Receiver<ControllerMessage>,
    yielded: Arc<AtomicBool>,
}

impl EmbeddedTransport {
    pub(crate) fn new(
        tx: mpsc::Sender<ServiceMessage>,
        rx: mpsc::Receiver<ControllerMessage>,
        yielded: Arc<AtomicBool>,
    ) -> Self {
        Self { tx, rx, yielded }
    }

    /// Send a service message to the controller-side processor.
    pub(crate) async fn send(
        &self,
        msg: ServiceMessage,
    ) -> Result<(), mpsc::error::SendError<ServiceMessage>> {
        self.tx.send(msg).await
    }

    /// Receive the next controller message from the processor.
    pub(crate) async fn recv(&mut self) -> Option<ControllerMessage> {
        self.rx.recv().await
    }
}

// ---------------------------------------------------------------------------
// ExternalServiceInfo
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl uptrakit_internal_wire::ServiceTransport for EmbeddedTransport {
    async fn transport_send(
        &mut self,
        msg: ServiceMessage,
    ) -> Result<(), uptrakit_internal_wire::TransportError> {
        self.tx
            .send(msg)
            .await
            .map_err(|_| uptrakit_internal_wire::TransportError)
    }

    async fn transport_send_best_effort(&mut self, msg: ServiceMessage) {
        let _ = self.tx.try_send(msg);
    }

    async fn transport_send_auto_paginate(
        &mut self,
        msg: ServiceMessage,
    ) -> Result<(), uptrakit_internal_wire::TransportError> {
        // In-process channels have no frame size limits — delegate to send.
        self.transport_send(msg).await
    }

    async fn transport_recv(&mut self) -> Option<ControllerMessage> {
        // Channel transports cannot surface typed transport errors here;
        // `None` is always the terminal receive condition.
        self.rx.recv().await
    }

    fn close_policy(&self) -> uptrakit_internal_wire::TransportClosePolicy {
        uptrakit_internal_wire::TransportClosePolicy::Shutdown
    }

    fn is_yielded(&self) -> bool {
        self.yielded.load(Ordering::Relaxed)
    }
}

/// Info about an external service, used for yield decisions.
pub(crate) struct ExternalServiceInfo {
    pub service_id: Uuid,
    pub capabilities: BTreeSet<Capability>,
    pub hostname: Option<String>,
    pub machine_id: Option<String>,
    /// `service_app_name` from the service record, read from
    /// `ServiceConnectionRegistry` when the external service connects.
    pub service_app_name: Option<String>,
    pub is_system: bool,
}

// ---------------------------------------------------------------------------
// CoexistencePolicy
// ---------------------------------------------------------------------------

/// Custom yield predicate for [`CoexistencePolicy::Custom`].
pub(crate) type YieldCheckFn = Box<dyn Fn(&ExternalServiceInfo) -> bool + Send + Sync>;

/// Controls whether an embedded service yields to a connecting external service.
#[derive(Default)]
pub(crate) enum CoexistencePolicy {
    /// Yield when an external service with the same `service_app_name` connects.
    ///
    /// This is the default — it matches by binary identity, not by capability
    /// set, so shared capabilities like `GracefulShutdown` never cause false
    /// yields.
    #[default]
    YieldOnSameAppName,
    /// Custom yield predicate — use when additional context (e.g. `machine_id`)
    /// is needed beyond `service_app_name`.
    Custom(YieldCheckFn),
    /// Never yield — always coexist with external services.
    NeverYield,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc;
    use uptrakit_internal_wire::ServiceTransport;

    fn make_transport(
        yielded: bool,
    ) -> (
        EmbeddedTransport,
        Arc<AtomicBool>,
        mpsc::Receiver<ServiceMessage>,
        mpsc::Sender<ControllerMessage>,
    ) {
        let (svc_tx, svc_rx) = mpsc::channel::<ServiceMessage>(1);
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(1);
        let flag = Arc::new(AtomicBool::new(yielded));
        let transport = EmbeddedTransport::new(svc_tx, ctrl_rx, Arc::clone(&flag));
        (transport, flag, svc_rx, ctrl_tx)
    }

    #[test]
    fn close_policy_returns_shutdown() {
        let (transport, _flag, _rx, _tx) = make_transport(false);
        assert_eq!(
            transport.close_policy(),
            uptrakit_internal_wire::TransportClosePolicy::Shutdown,
        );
    }

    #[test]
    fn is_yielded_returns_false_when_flag_is_false() {
        let (transport, _flag, _rx, _tx) = make_transport(false);
        assert!(!transport.is_yielded());
    }

    #[test]
    fn is_yielded_returns_true_when_flag_is_true() {
        let (transport, _flag, _rx, _tx) = make_transport(true);
        assert!(transport.is_yielded());
    }

    #[test]
    fn is_yielded_reflects_runtime_flag_change() {
        let (transport, flag, _rx, _tx) = make_transport(false);
        assert!(!transport.is_yielded());
        flag.store(true, Ordering::Release);
        assert!(transport.is_yielded());
        flag.store(false, Ordering::Release);
        assert!(!transport.is_yielded());
    }
}
