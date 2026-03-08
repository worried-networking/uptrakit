use serde::{Deserialize, Serialize};

use super::messages::{ControllerMessage, ServiceMessage};
use super::trace_context::TraceContext;

/// The current wire protocol version stamped on every envelope.
///
/// Increment this constant whenever a breaking change is introduced to the
/// wire protocol (e.g. a required field is added, a variant renamed, or
/// capability-negotiation semantics change). Peers that receive a
/// `protocol_version` value they do not recognise must close the connection
/// with [`CloseReason::ProtocolError`](super::CloseReason).
pub const CURRENT_PROTOCOL_VERSION: u32 = 1;

/// Envelope wrapping a [`ServiceMessage`] with a monotonically increasing
/// sequence number for replay protection and the current protocol version.
///
/// JSON on the wire includes an optional `trace_context` object for distributed
/// tracing correlation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEnvelope {
    pub protocol_version: u32,
    pub seq: u64,
    /// Distributed tracing context for correlating this message across services.
    /// Always populated when sending; tolerates absence when receiving from older peers.
    #[serde(default)]
    pub trace_context: TraceContext,
    #[serde(flatten)]
    pub message: ServiceMessage,
}

/// Envelope wrapping a [`ControllerMessage`] with a monotonically increasing
/// sequence number for replay protection and the current protocol version.
///
/// JSON on the wire includes an optional `trace_context` object for distributed
/// tracing correlation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerEnvelope {
    pub protocol_version: u32,
    pub seq: u64,
    /// Distributed tracing context for correlating this message across services.
    /// Always populated when sending; tolerates absence when receiving from older peers.
    #[serde(default)]
    pub trace_context: TraceContext,
    #[serde(flatten)]
    pub message: ControllerMessage,
}

/// Tracks outgoing sequence numbers for a single direction of a WebSocket
/// connection. Assigns monotonically increasing numbers starting at 1.
#[derive(Debug)]
pub struct OutgoingSeq {
    next: u64,
}

impl OutgoingSeq {
    /// Create a new outgoing sequence counter (first message gets seq 1).
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// Wrap a [`ServiceMessage`] in a [`ServiceEnvelope`], assigning the next
    /// sequence number, stamping [`CURRENT_PROTOCOL_VERSION`], and attaching
    /// the given [`TraceContext`] for distributed tracing.
    pub fn wrap_service(
        &mut self,
        message: ServiceMessage,
        trace_context: TraceContext,
    ) -> ServiceEnvelope {
        let seq = self.next;
        self.next += 1;
        ServiceEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq,
            trace_context,
            message,
        }
    }

    /// Wrap a [`ControllerMessage`] in a [`ControllerEnvelope`], assigning the
    /// next sequence number, stamping [`CURRENT_PROTOCOL_VERSION`], and attaching
    /// the given [`TraceContext`] for distributed tracing.
    pub fn wrap_controller(
        &mut self,
        message: ControllerMessage,
        trace_context: TraceContext,
    ) -> ControllerEnvelope {
        let seq = self.next;
        self.next += 1;
        ControllerEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq,
            trace_context,
            message,
        }
    }
}

impl Default for OutgoingSeq {
    fn default() -> Self {
        Self::new()
    }
}

/// Validates incoming sequence numbers for a single direction of a WebSocket
/// connection. Expects messages to arrive as 1, 2, 3, ...
#[derive(Debug)]
pub struct IncomingSeq {
    expected: u64,
}

impl IncomingSeq {
    /// Create a new incoming sequence validator (first expected seq is 1).
    pub fn new() -> Self {
        Self { expected: 1 }
    }

    /// Validate that the received sequence number matches the expected value.
    ///
    /// On success, advances the expected counter. On failure, returns a
    /// [`SeqError`] describing the mismatch.
    pub fn validate(&mut self, received: u64) -> Result<(), SeqError> {
        if received != self.expected {
            return Err(SeqError {
                expected: self.expected,
                received,
            });
        }
        self.expected += 1;
        Ok(())
    }
}

impl Default for IncomingSeq {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when a received sequence number does not match the expected
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("sequence error: expected {expected}, received {received}")]
pub struct SeqError {
    pub expected: u64,
    pub received: u64,
}
