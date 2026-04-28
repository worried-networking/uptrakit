// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Transport abstraction shared by service business logic.
//!
//! # Layered Error Contract
//!
//! The service runtime uses three error layers:
//!
//! - **Layer 1 (this module)**: [`ServiceTransport`] + [`TransportError`].
//!   This layer is intentionally lossy: send failures collapse to an opaque
//!   `TransportError`, and receive failures collapse to `None`.
//! - **Layer 2 (`service-sdk`)**: `ControllerConnection::recv()`.
//!   This layer keeps rich receive fidelity via
//!   `Result<Option<ControllerMessage>, Report<EnrollmentError>>`.
//! - **Layer 3 (`service-sdk`)**: event-loop/lifecycle classification.
//!   This layer maps `EnrollmentError` to `LoopError` and decides reconnect
//!   semantics (`Disconnected`, backoff, or fatal propagation).
//!
//! `agent-core` functions accept `&mut dyn ServiceTransport` so the same
//! logic can run with either WebSocket or in-process channel transports.
//!
//! # `transport_recv()` Terminal Contract
//!
//! `transport_recv()` returns `Option<ControllerMessage>`:
//!
//! - `Some(message)` => keep processing.
//! - `None` => transport is closed or broken; stop reading.
//!
//! Layer 1 callers must not distinguish clean close from transport error.
//!
//! # Unresolved Gaps
//!
//! `ServiceTransport` intentionally has no `transport_close()` method.
//! Close semantics are transport-specific and managed by lifecycle owners,
//! not by shared business logic.
use crate::generated::wire::CloseReason;
use crate::generated::wire::messages::{ControllerMessage, ServiceMessage};
/// Opaque send-failure marker for [`ServiceTransport`].
///
/// Layer 1 deliberately erases transport-specific failure details. Callers
/// should treat any `TransportError` as a disconnect/reconnect signal.
#[derive(Debug, thiserror::Error)]
#[error("transport send failed")]
pub struct TransportError;
/// Policy the event loop should follow when the transport receive stream ends.
///
/// Each transport implementation returns the appropriate policy via
/// [`ServiceTransport::close_policy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportClosePolicy {
    /// Reconnect to the controller, optionally including the cached close reason
    /// from the last close frame.
    Reconnect { reason: Option<CloseReason> },
    /// Shut down the event loop entirely.
    Shutdown,
}
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
    /// # Contract
    ///
    /// `None` is the terminal condition for this abstraction:
    ///
    /// - clean close
    /// - peer-closed stream
    /// - receive-side transport failure
    ///
    /// Layer 1 callers must treat all three the same and stop reading.
    ///
    /// In production, standalone services bypass this lossy method and call
    /// `ControllerConnection::recv()` directly for richer error handling.
    /// The `None` contract here is exercised by embedded channel transports.
    async fn transport_recv(&mut self) -> Option<ControllerMessage>;
    /// Policy the event loop should follow when `transport_recv` returns `None`.
    fn close_policy(&self) -> TransportClosePolicy {
        TransportClosePolicy::Reconnect { reason: None }
    }
    /// Whether this transport is currently yielded to an external counterpart.
    fn is_yielded(&self) -> bool {
        false
    }
}
