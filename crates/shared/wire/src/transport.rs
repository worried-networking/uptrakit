//! Transport-agnostic message sending interface.
//!
//! [`ServiceTransport`] abstracts the send/receive side of a service's
//! connection to the controller. Concrete implementations exist for:
//!
//! - **`ControllerConnection`** (WebSocket) in `service-sdk`
//! - **`EmbeddedTransport`** (mpsc channels) in the controller crate
//!
//! `agent-core` functions accept `&mut dyn ServiceTransport` so the same
//! update/version-check logic works for both standalone and embedded agents.

use crate::messages::{ControllerMessage, ServiceMessage};

/// Error returned when a transport send operation fails.
///
/// Deliberately opaque — callers should treat any send failure as a reason
/// to disconnect/reconnect rather than inspecting the cause.
#[derive(Debug, thiserror::Error)]
#[error("transport send failed")]
pub struct TransportError;

/// Transport-agnostic interface for sending/receiving wire messages.
///
/// Method names are prefixed with `transport_` to avoid collision with
/// existing `send()`/`recv()` methods on concrete types.
#[async_trait::async_trait]
pub trait ServiceTransport: Send {
    /// Send a message reliably. Returns error on transport failure.
    async fn transport_send(&mut self, msg: ServiceMessage) -> Result<(), TransportError>;

    /// Send a message on a best-effort basis. Drops silently on failure.
    async fn transport_send_best_effort(&mut self, msg: ServiceMessage);

    /// Send a message, auto-paginating large report payloads.
    ///
    /// For WebSocket transports this splits large payloads into pages.
    /// For in-process transports this delegates to [`transport_send`](Self::transport_send)
    /// since channel-based transports have no frame size limits.
    async fn transport_send_auto_paginate(
        &mut self,
        msg: ServiceMessage,
    ) -> Result<(), TransportError>;

    /// Receive the next controller message.
    ///
    /// Returns `None` on clean close or when the transport is shut down.
    async fn transport_recv(&mut self) -> Option<ControllerMessage>;
}
