//! Local types for the embedded service infrastructure.
//!
//! These types live entirely within the controller crate — no new shared
//! dependencies are introduced.
//!
//! Some types and methods are infrastructure-only and will be exercised by
//! follow-up service embeddings (agent, mqtt, agent-ssh).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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

    /// Check whether this embedded service is currently yielded.
    pub(crate) fn is_yielded(&self) -> bool {
        self.yielded.load(std::sync::atomic::Ordering::Relaxed)
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
        self.rx.recv().await
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
