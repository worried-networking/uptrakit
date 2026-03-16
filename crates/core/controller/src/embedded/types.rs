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

/// Info about an external service, used for yield decisions.
pub(crate) struct ExternalServiceInfo {
    pub service_id: Uuid,
    pub capabilities: BTreeSet<Capability>,
    pub hostname: Option<String>,
    pub machine_id: Option<String>,
    pub is_system: bool,
}
