//! Shared WebSocket protocol primitives.
//!
//! Types, serialization helpers, rate limiting, and error types consumed by
//! `connection`, `handler`, and external modules.

use std::collections::BTreeSet;
use std::net::IpAddr;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::SinkExt;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_internal_wire::{
    Capability, CloseReason, ControllerMessage, CURRENT_PROTOCOL_VERSION, IncomingSeq, OutgoingSeq,
    PongPayload, ServiceEnvelope, ServiceMessage, now_millis,
};
use uptrakit_shared_db::entity::service as service_entity;
use uptrakit_shared_macros::impl_report_conversion;

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

/// Maximum number of incoming WebSocket messages per connection per second.
pub(crate) const WS_MESSAGE_RATE_LIMIT: u32 = 50;
/// Window for WebSocket message rate limiting.
pub(crate) const WS_MESSAGE_RATE_WINDOW: std::time::Duration = std::time::Duration::from_secs(1);

/// Fixed-window rate limiter for WebSocket message processing.
pub(crate) struct MessageRateLimiter {
    window_start: std::time::Instant,
    window: std::time::Duration,
    max_per_window: u32,
    count: u32,
}

impl MessageRateLimiter {
    pub(crate) fn new(window: std::time::Duration, max_per_window: u32) -> Self {
        Self {
            window_start: std::time::Instant::now(),
            window,
            max_per_window,
            count: 0,
        }
    }

    pub(crate) fn allow(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.window_start) >= self.window {
            self.window_start = now;
            self.count = 0;
        }

        if self.count < self.max_per_window {
            self.count = self.count.saturating_add(1);
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    pub(crate) fn set_window_start(&mut self, start: std::time::Instant) {
        self.window_start = start;
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub(crate) enum ServiceWsError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("invalid enrollment secret")]
    InvalidSecret,
    #[error("message deserialization failed: {0}")]
    Deserialize(String),
    #[error("sequence validation failed: {0}")]
    SequenceValidation(String),
    #[error("protocol version mismatch: expected {expected}, received {received}")]
    ProtocolVersionMismatch { expected: u32, received: u32 },
}

pub(crate) type ServiceWsResult<T> = std::result::Result<T, Report<ServiceWsError>>;

impl_report_conversion!(sea_orm::DbErr => ServiceWsError::Database);

#[derive(Debug, Error)]
pub(crate) enum ServiceActivityError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("service not found: {0}")]
    ServiceNotFound(uuid::Uuid),
}

pub(crate) type ServiceActivityResult<T> = std::result::Result<T, Report<ServiceActivityError>>;

impl_report_conversion!(sea_orm::DbErr => ServiceActivityError::Database);

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

/// Serialize a [`ControllerMessage`] into a sequenced [`ControllerEnvelope`]
/// JSON string, logging on failure.
pub(crate) fn serialize_controller_msg(
    out_seq: &mut OutgoingSeq,
    msg: ControllerMessage,
) -> Option<String> {
    let envelope = out_seq.wrap_controller(msg);
    match serde_json::to_string(&envelope) {
        Ok(json) => Some(json),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize controller message");
            None
        }
    }
}

/// Minimal envelope used to extract envelope fields before full deserialization.
///
/// Advancing the sequence counter even when the full payload cannot be parsed
/// (e.g. an unknown message type from a future service) is required for replay
/// protection to remain accurate. Extracting `protocol_version` here lets us
/// reject version-mismatched connections before attempting the full parse.
#[derive(serde::Deserialize)]
struct EnvelopeHeader {
    protocol_version: u32,
    seq: u64,
}

/// Deserialize a [`ServiceMessage`] from a sequenced [`ServiceEnvelope`]
/// JSON string, validating the protocol version and sequence number.
///
/// Returns:
/// - `Err(_)` on malformed JSON, protocol version mismatch, or sequence mismatch
///   (hard errors — connection should be closed).
/// - `Ok(Some(ServiceMessage::Unknown))` when the `type` tag is not recognised
///   (unknown message type from a newer service build — sequence was already
///   advanced; the caller should log a warning and continue).
/// - `Ok(None)` when the full envelope parse fails for other reasons (soft
///   failure — sequence was already advanced).
/// - `Ok(Some(msg))` on successful parse.
pub(crate) fn deserialize_service_msg(
    in_seq: &mut IncomingSeq,
    text: &str,
) -> ServiceWsResult<Option<ServiceMessage>> {
    // Step 1: Extract protocol version and sequence number (hard fail on malformed JSON).
    let header: EnvelopeHeader = serde_json::from_str(text)
        .context_transform(|e| ServiceWsError::Deserialize(format!("invalid message: {e}")))?;

    // Step 2: Validate protocol version (hard fail on mismatch).
    if header.protocol_version != CURRENT_PROTOCOL_VERSION {
        return Err(report!(ServiceWsError::ProtocolVersionMismatch {
            expected: CURRENT_PROTOCOL_VERSION,
            received: header.protocol_version,
        }));
    }

    // Step 3: Validate sequence (hard fail on mismatch).
    in_seq
        .validate(header.seq)
        .map_err(|e| report!(ServiceWsError::SequenceValidation(e.to_string())))?;

    // Step 4: Full parse — soft fail for unknown types from newer service builds.
    match serde_json::from_str::<ServiceEnvelope>(text) {
        Ok(envelope) => Ok(Some(envelope.message)),
        Err(e) => {
            tracing::debug!("ignoring unrecognized service message: {e}");
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Protocol helpers
// ---------------------------------------------------------------------------

/// Returns the complete set of capabilities advertised by the controller.
///
/// The controller advertises all known capabilities so every service type can
/// compute its agreed set regardless of which service type is connecting.
pub(crate) fn controller_capabilities() -> BTreeSet<Capability> {
    [
        Capability::SoftwareDiscovery,
        Capability::UpdateHooks,
        Capability::GracefulShutdown,
        Capability::MqttBridge,
        Capability::SshRemote,
        Capability::Scheduler,
        Capability::DatabaseAccess,
        Capability::NatsAccess,
        Capability::MasterKeyAccess,
        Capability::CaManagement,
    ]
    .into_iter()
    .collect()
}

pub(crate) async fn close_with_reason(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    reason: CloseReason,
) -> Result<(), axum::Error> {
    sink.send(Message::Close(Some(CloseFrame {
        code: axum::extract::ws::close_code::POLICY,
        reason: reason.as_str().into(),
    })))
    .await
}

/// Send a Pong response for a received Ping.
///
/// Returns the controller timestamp on success so callers can use it
/// for trace logging.
pub(crate) async fn send_pong(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    service_ts: i64,
) -> Result<i64, ()> {
    let controller_ts = now_millis();
    let response = ControllerMessage::Pong(PongPayload::new(service_ts, controller_ts));
    let Some(json) = serialize_controller_msg(out_seq, response) else {
        return Err(());
    };
    sink.send(Message::Text(json.into()))
        .await
        .map(|()| controller_ts)
        .map_err(|_| ())
}

pub(crate) async fn record_service_activity(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    ip_address: Option<IpAddr>,
) -> ServiceActivityResult<()> {
    let service = service_entity::Entity::find_by_id(service_id)
        .one(db)
        .await
        .context_to::<ServiceActivityError>()?
        .ok_or_else(|| report!(ServiceActivityError::ServiceNotFound(service_id)))?;

    let now = time::OffsetDateTime::now_utc();
    let mut active: service_entity::ActiveModel = service.into();
    active.last_seen_at = Set(Some(now));
    active.updated_at = Set(now);
    if let Some(ip) = ip_address {
        active.ip_address = Set(Some(ip.to_string()));
    }
    active
        .update(db)
        .await
        .context_to::<ServiceActivityError>()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Connection types
// ---------------------------------------------------------------------------

/// Certificate identity information extracted from the mTLS handshake.
///
/// Bundled into a struct to keep function signatures under the argument limit.
pub(crate) struct CertIdentity {
    pub serial: String,
    pub ca_fingerprint: String,
}

/// Shared context for authenticated service handlers.
pub(crate) struct AuthenticatedContext<'a> {
    pub service_id: uuid::Uuid,
    pub cert: CertIdentity,
    pub out_seq: &'a mut OutgoingSeq,
    pub in_seq: &'a mut IncomingSeq,
}

/// Connection type determined at WebSocket upgrade time.
pub(super) enum ConnectionType {
    /// mTLS client cert present -- authenticated service.
    Authenticated {
        service_id: uuid::Uuid,
        cert_serial: String,
    },
    /// Authorization: Bearer <secret> -- reconnecting enrolled service.
    Enrolled(uuid::Uuid),
    /// No auth -- expects Enroll message.
    Anonymous,
}
