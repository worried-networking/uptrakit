//! Shared authenticated WebSocket connection to the controller.
//!
//! [`ControllerConnection`] wraps a TLS WebSocket stream with automatic
//! envelope serialization, sequence validation, and WebSocket frame handling.
//! Both the agent and the MQTT service use this type for their authenticated
//! event loops.
//!
//! ## Non-blocking write path
//!
//! The WebSocket stream is split into read and write halves at construction.
//! A dedicated writer task drains a bounded channel (`WRITE_CHANNEL_CAPACITY`),
//! performing the actual I/O with a per-write timeout (`SEND_TIMEOUT`).
//!
//! `send()` and `send_paginated()` serialize the message and push the
//! resulting text frame into the channel — no I/O block on the caller side.
//! `send_best_effort()` uses `try_send()` so it never blocks even if the
//! channel is full.
//!
//! If the writer encounters an I/O error, it sets an `Arc<AtomicBool>` flag.
//! Subsequent `send()` and `recv()` calls check this flag and return an error
//! immediately rather than queuing into a dead channel.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use rootcause::prelude::*;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use uptrakit_internal_wire::{
    CURRENT_PROTOCOL_VERSION, Capability, CloseReason, ControllerEnvelope, ControllerMessage,
    IncomingSeq, OutgoingSeq, Paginatable, ReportPageLimits, ServiceMessage,
    paginate::paginate_payload,
};

use crate::error::{EnrollmentError, ProtocolError, Result};
use crate::ws::{WsSink, connect_ws, is_peer_closed, log_close_frame, split_ws_stream};

/// Maximum time to wait for a single WebSocket write to complete.
///
/// If the controller stops consuming data (e.g. after a restart), the TCP
/// send buffer fills and writes block indefinitely. This timeout bounds
/// the worst-case hang to 30 seconds, after which the writer task sets the
/// error flag and exits.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Bounded channel capacity for outbound frames.
///
/// 128 slots allow paginated reports (typically ≤20 pages) and bursty
/// status messages to queue without blocking the caller, while still
/// applying back-pressure if the network cannot keep up.
const WRITE_CHANNEL_CAPACITY: usize = 128;

/// An outbound frame queued for the writer task.
enum OutboundFrame {
    /// A WebSocket text frame (serialized JSON envelope).
    Text(String),
    /// A WebSocket pong response to a controller ping.
    Pong(Vec<u8>),
    /// Graceful WebSocket close.
    Close,
}

/// Minimal envelope used to extract protocol version and sequence number before
/// full deserialization. This lets us validate both fields and advance the
/// incoming sequence counter even when the message payload cannot be parsed
/// (e.g. a new variant the service doesn't know about yet).
#[derive(Deserialize)]
struct EnvelopeHeader {
    protocol_version: u32,
    seq: u64,
}

/// Authenticated WebSocket connection to the controller.
///
/// Handles envelope wrapping, sequence validation, and WebSocket frame
/// processing (Ping/Pong, Close frames, peer-closed detection).
///
/// The write path is fully non-blocking: serialized frames are pushed into
/// a bounded channel and drained by a dedicated writer task.
pub struct ControllerConnection {
    /// Read half of the split WebSocket stream.
    read: Pin<
        Box<
            dyn Stream<Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>>
                + Send,
        >,
    >,
    /// Channel sender for outbound frames (drained by the writer task).
    write_tx: mpsc::Sender<OutboundFrame>,
    /// Signals that the writer task encountered an I/O error.
    write_error: Arc<AtomicBool>,
    /// Handle to the writer task (joined on close).
    writer_handle: Option<JoinHandle<()>>,
    out_seq: OutgoingSeq,
    in_seq: IncomingSeq,
    close_reason: Option<CloseReason>,
    agreed_capabilities: BTreeSet<Capability>,
    report_page_limits: ReportPageLimits,
}

/// Classification of a raw WebSocket frame for the `recv()` loop.
///
/// `recv()` delegates to [`classify_ws_frame()`] to determine the action
/// for each raw frame. The close/EOF contract: [`CloseFrame`],
/// [`RecvAction::PeerClosed`], and [`RecvAction::StreamEnd`] all cause `recv()`
/// to return `Ok(None)`. No path through this classification can produce
/// `ProtocolError::ReceiveClosed`.
enum RecvAction {
    /// Text or binary message payload — decode, validate, and return.
    Message(tokio_tungstenite::tungstenite::Message),
    /// WebSocket close frame received — store close reason, return `Ok(None)`.
    CloseFrame(Option<CloseFrame>),
    /// Peer closed the connection (transport-level) — return `Ok(None)`.
    PeerClosed,
    /// WebSocket stream exhausted — return `Ok(None)`.
    StreamEnd,
    /// Control frame (ping, pong, raw frame) — continue the loop.
    Control(tokio_tungstenite::tungstenite::Message),
    /// Transport error — propagate as `Err`.
    TransportError(tokio_tungstenite::tungstenite::Error),
}

/// Classify a raw WebSocket frame result into a [`RecvAction`].
///
/// This is the production classification logic used by
/// [`ControllerConnection::recv()`]. The close/EOF contract:
/// `Close`, peer-closed errors (via [`is_peer_closed()`]), and stream
/// exhaustion (`None`) all produce variants that map to `Ok(None)` in
/// `recv()`. Transport errors produce [`RecvAction::TransportError`]
/// which `recv()` propagates as `Err`.
///
/// The `is_peer_closed()` function matches:
/// - `WsErr::Io(io)` where `io.kind()` is `UnexpectedEof | BrokenPipe |
///   ConnectionReset | ConnectionAborted | NotConnected`
/// - `WsErr::Protocol(ResetWithoutClosingHandshake | SendAfterClosing)`
fn classify_ws_frame(
    frame: Option<
        std::result::Result<
            tokio_tungstenite::tungstenite::Message,
            tokio_tungstenite::tungstenite::Error,
        >,
    >,
) -> RecvAction {
    use tokio_tungstenite::tungstenite::Message;

    match frame {
        Some(Ok(msg @ (Message::Text(_) | Message::Binary(_)))) => RecvAction::Message(msg),
        Some(Ok(Message::Close(frame))) => RecvAction::CloseFrame(frame),
        Some(Ok(msg)) => RecvAction::Control(msg),
        Some(Err(e)) if is_peer_closed(&e) => RecvAction::PeerClosed,
        Some(Err(e)) => RecvAction::TransportError(e),
        None => RecvAction::StreamEnd,
    }
}

impl ControllerConnection {
    /// Connect TCP -> TLS -> WebSocket to the controller.
    ///
    /// Splits the stream into read and write halves and spawns the writer task.
    pub async fn connect(
        host: &str,
        port: u16,
        tls_connector: &TlsConnector,
        auth_header: Option<&str>,
    ) -> Result<Self> {
        tracing::debug!(host, port, "connecting to controller");
        let ws = connect_ws(host, port, tls_connector, auth_header, None).await?;
        tracing::debug!("WebSocket connection established");

        let (sink, read) = split_ws_stream(ws);
        let (write_tx, write_rx) = mpsc::channel(WRITE_CHANNEL_CAPACITY);
        let write_error = Arc::new(AtomicBool::new(false));
        let writer_handle = tokio::spawn(writer_task(sink, write_rx, Arc::clone(&write_error)));

        Ok(Self {
            read: Box::pin(read),
            write_tx,
            write_error,
            writer_handle: Some(writer_handle),
            out_seq: OutgoingSeq::new(),
            in_seq: IncomingSeq::new(),
            close_reason: None,
            agreed_capabilities: BTreeSet::new(),
            report_page_limits: ReportPageLimits::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn new_test(
        read: Pin<
            Box<
                dyn Stream<
                        Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
                    > + Send,
            >,
        >,
    ) -> Self {
        let (write_tx, mut write_rx) = mpsc::channel::<OutboundFrame>(WRITE_CHANNEL_CAPACITY);
        let write_error = Arc::new(AtomicBool::new(false));
        let writer_handle = tokio::spawn(async move {
            while let Some(frame) = write_rx.recv().await {
                if matches!(frame, OutboundFrame::Close) {
                    return;
                }
            }
        });

        Self {
            read,
            write_tx,
            write_error,
            writer_handle: Some(writer_handle),
            out_seq: OutgoingSeq::new(),
            in_seq: IncomingSeq::new(),
            close_reason: None,
            agreed_capabilities: BTreeSet::new(),
            report_page_limits: ReportPageLimits::default(),
        }
    }

    /// Send a [`ServiceMessage`] to the controller.
    ///
    /// Wraps the message in a [`ServiceEnvelope`](uptrakit_internal_wire::ServiceEnvelope)
    /// with the next sequence number, serializes to JSON, and pushes the text
    /// frame into the write channel. The actual WebSocket write happens
    /// asynchronously in the writer task.
    #[tracing::instrument(skip_all)]
    pub async fn send(&mut self, msg: ServiceMessage) -> Result<()> {
        self.check_write_error()?;
        let envelope = self
            .out_seq
            .wrap_service(msg, uptrakit_internal_wire::current_trace_context());
        let json = serde_json::to_string(&envelope).context_to::<EnrollmentError>()?;
        self.write_tx
            .send(OutboundFrame::Text(json))
            .await
            .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::SendTimeout)))?;
        Ok(())
    }

    /// Receive the next [`ControllerMessage`] from the controller.
    ///
    /// # Return values
    ///
    /// - `Ok(Some(msg))`: a decoded controller message with validated protocol
    ///   version and sequence.
    /// - `Ok(None)`: close/EOF terminal condition (close frame, peer-closed,
    ///   or stream end).
    /// - `Err(Report<EnrollmentError>)`: transport/protocol failure.
    ///
    /// `ProtocolError::ReceiveClosed` is never produced by this method.
    /// Close/EOF outcomes are classified by `classify_ws_frame()` and mapped
    /// to `Ok(None)`.
    ///
    /// # Writer-health check
    ///
    /// Every loop iteration calls `check_write_error()`
    /// before reading. If the background writer task failed, this method
    /// returns `Err(EnrollmentError::Protocol(ProtocolError::SendTimeout))`.
    ///
    /// # Sequence and Version Validation
    ///
    /// `validate_header()` enforces:
    ///
    /// - protocol version equality (`ProtocolError::VersionMismatch`)
    /// - monotonic sequence validation (`ProtocolError::Enrollment("sequence validation failed: ...")`)
    ///
    /// Automatically:
    /// - Responds to WebSocket `Ping` with `Pong` (via the write channel)
    /// - Skips `Pong` and `Frame` messages
    /// - Stores close frame reason (accessible via [`close_reason`](Self::close_reason))
    /// - Validates incoming sequence numbers
    /// - Checks the writer error flag on each iteration
    #[tracing::instrument(skip_all)]
    pub async fn recv(&mut self) -> Result<Option<ControllerMessage>> {
        loop {
            // Check writer health before blocking on read.
            self.check_write_error()?;

            match classify_ws_frame(self.read.next().await) {
                RecvAction::Message(Message::Text(text)) => {
                    return self.decode_text_message(&text);
                }
                RecvAction::Message(Message::Binary(data)) => {
                    return self.decode_binary_message(&data);
                }
                RecvAction::Message(_) => {
                    unreachable!("classify_ws_frame returns Message only for Text/Binary")
                }
                RecvAction::Control(Message::Ping(data)) => {
                    tracing::trace!("received ping from controller");
                    // Push pong through the write channel (non-blocking).
                    if self
                        .write_tx
                        .try_send(OutboundFrame::Pong(data.to_vec()))
                        .is_err()
                    {
                        tracing::warn!("write channel full, dropping pong");
                    }
                    continue;
                }
                RecvAction::Control(_) => continue,
                RecvAction::CloseFrame(frame) => {
                    self.close_reason = frame.as_ref().map(|f| {
                        f.reason
                            .parse::<CloseReason>()
                            .unwrap_or_else(|_| CloseReason::Unknown(f.reason.to_string()))
                    });
                    log_close_frame(frame);
                    return Ok(None);
                }
                RecvAction::PeerClosed => {
                    tracing::info!("connection closed by controller");
                    return Ok(None);
                }
                RecvAction::StreamEnd => {
                    return Ok(None);
                }
                RecvAction::TransportError(e) => {
                    return Err(e).context_to::<EnrollmentError>()?;
                }
            }
        }
    }

    /// Get the close reason from the last WebSocket close frame, if any.
    ///
    /// Used by the agent to distinguish [`CloseReason::CertificateRotated`]
    /// from other close reasons.
    pub fn close_reason(&self) -> Option<&CloseReason> {
        self.close_reason.as_ref()
    }

    /// Returns the agreed capability set computed during `ServiceSettings` processing.
    ///
    /// Only populated after the first `ServiceSettings` message is received.
    /// Contains the intersection of the controller's advertised capabilities and
    /// this service's own capabilities, filtered to known typed variants only.
    pub fn agreed_capabilities(&self) -> &BTreeSet<Capability> {
        &self.agreed_capabilities
    }

    /// Sets the agreed capability set (called by the event loop after negotiation).
    pub(crate) fn set_agreed_capabilities(&mut self, caps: BTreeSet<Capability>) {
        self.agreed_capabilities = caps;
    }

    /// Returns the current per-page report limits from the latest `ServiceSettings`.
    pub fn report_page_limits(&self) -> &ReportPageLimits {
        &self.report_page_limits
    }

    /// Sets the per-page report limits from `ServiceSettings`.
    pub(crate) fn set_report_page_limits(&mut self, limits: ReportPageLimits) {
        self.report_page_limits = limits;
    }

    /// Send a [`Paginatable`] payload, automatically splitting it into pages
    /// when the serialized size exceeds the pagination threshold.
    ///
    /// Small payloads are sent as a single message with no pagination metadata
    /// (zero overhead). Large payloads are split into pages, each pushed as a
    /// separate text frame into the write channel.
    ///
    /// Serialization happens on the caller's task; only the channel push is
    /// async. This means even large paginated payloads do not block the caller
    /// on network I/O.
    pub async fn send_paginated<P: Paginatable>(&mut self, payload: P) -> Result<()> {
        self.check_write_error()?;
        let pages =
            paginate_payload(payload, &self.report_page_limits).context_to::<EnrollmentError>()?;
        let page_count = pages.len();

        for (i, page) in pages.into_iter().enumerate() {
            let msg = page.payload.into_message();
            let envelope = self.out_seq.wrap_service_paginated(
                msg,
                uptrakit_internal_wire::current_trace_context(),
                page.pagination,
            );
            let json = serde_json::to_string(&envelope).context_to::<EnrollmentError>()?;

            if let Err(e) = self.write_tx.send(OutboundFrame::Text(json)).await {
                tracing::error!(
                    page = i + 1,
                    total_pages = page_count,
                    error = %e,
                    "failed to enqueue paginated message page"
                );
                return Err(report!(EnrollmentError::Protocol(
                    ProtocolError::SendTimeout
                )));
            }

            if page_count > 1 {
                tracing::debug!(
                    page = i + 1,
                    total_pages = page_count,
                    "enqueued paginated message page"
                );
            }
        }

        Ok(())
    }

    /// Send a [`ServiceMessage`], automatically paginating paginatable payload
    /// types when they exceed the wire size threshold.
    ///
    /// Paginatable variants (`DiscoveryResults`, `VersionCheckResults`,
    /// `ReportHosts`, `BatchUpdateResult`) are split into pages
    /// transparently. All other variants are sent as a single message.
    ///
    /// This is the recommended method for sending report-style messages.
    pub async fn send_auto_paginate(&mut self, msg: ServiceMessage) -> Result<()> {
        match msg {
            ServiceMessage::DiscoveryResults(payload) => self.send_paginated(payload).await,
            ServiceMessage::VersionCheckResults(payload) => self.send_paginated(payload).await,
            ServiceMessage::ReportHosts(payload) => self.send_paginated(payload).await,
            ServiceMessage::BatchUpdateResult(payload) => self.send_paginated(payload).await,
            other => self.send(other).await,
        }
    }

    /// Send a [`ServiceMessage`] on a best-effort basis (non-blocking).
    ///
    /// Uses `try_send()` to push the frame into the write channel without
    /// blocking. If the channel is full or the writer has errored, the message
    /// is silently dropped with a warning log.
    pub async fn send_best_effort(&mut self, msg: ServiceMessage) {
        if self.write_error.load(Ordering::Relaxed) {
            tracing::warn!("best-effort send skipped: writer has errored");
            return;
        }
        let envelope = self
            .out_seq
            .wrap_service(msg, uptrakit_internal_wire::current_trace_context());
        let json = match serde_json::to_string(&envelope) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "best-effort send: serialization failed");
                return;
            }
        };
        if let Err(e) = self.write_tx.try_send(OutboundFrame::Text(json)) {
            tracing::warn!(error = %e, "best-effort send failed (channel full or closed)");
        }
    }

    /// Graceful WebSocket close.
    ///
    /// Pushes a `Close` frame into the write channel and waits for the writer
    /// task to finish. Tolerates a dead writer (error flag already set).
    pub async fn close(&mut self) -> Result<()> {
        // Push close frame (best-effort — writer may already be dead).
        let _ = self.write_tx.send(OutboundFrame::Close).await;
        // Wait for the writer task to finish.
        if let Some(handle) = self.writer_handle.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Check the writer error flag and return an error if set.
    fn check_write_error(&self) -> Result<()> {
        if self.write_error.load(Ordering::Relaxed) {
            Err(report!(EnrollmentError::Protocol(
                ProtocolError::SendTimeout
            )))
        } else {
            Ok(())
        }
    }

    /// Decode a text WebSocket message into a [`ControllerMessage`].
    fn decode_text_message(&mut self, text: &str) -> Result<Option<ControllerMessage>> {
        let header: EnvelopeHeader = serde_json::from_str(text).context_to::<EnrollmentError>()?;
        self.validate_header(&header)?;
        let envelope: ControllerEnvelope = match serde_json::from_str(text) {
            Ok(env) => env,
            Err(e) => {
                tracing::debug!("ignoring unrecognized controller message: {e}");
                return Ok(None);
            }
        };
        tracing::trace!(msg_type = ?envelope.message, "received message from controller");
        Ok(Some(envelope.message))
    }

    /// Decode a binary WebSocket message into a [`ControllerMessage`].
    fn decode_binary_message(&mut self, data: &[u8]) -> Result<Option<ControllerMessage>> {
        let header: EnvelopeHeader =
            serde_json::from_slice(data).context_to::<EnrollmentError>()?;
        self.validate_header(&header)?;
        let envelope: ControllerEnvelope = match serde_json::from_slice(data) {
            Ok(env) => env,
            Err(e) => {
                tracing::debug!("ignoring unrecognized controller binary message: {e}");
                return Ok(None);
            }
        };
        tracing::trace!(msg_type = ?envelope.message, "received message from controller");
        Ok(Some(envelope.message))
    }

    /// Validate the protocol version and sequence number from a message header.
    fn validate_header(&mut self, header: &EnvelopeHeader) -> Result<()> {
        if header.protocol_version != CURRENT_PROTOCOL_VERSION {
            bail!(EnrollmentError::Protocol(ProtocolError::VersionMismatch {
                expected: CURRENT_PROTOCOL_VERSION,
                received: header.protocol_version,
            }));
        }
        if let Err(e) = self.in_seq.validate(header.seq) {
            bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(
                format!("sequence validation failed: {e}")
            )));
        }
        Ok(())
    }
}

/// Map the cached close reason to a transport close policy.
///
/// Extracted for testability: `ControllerConnection::close_policy()`
/// delegates to this helper, and tests can exercise the mapping without
/// constructing a live WebSocket-backed connection.
fn close_reason_to_policy(
    reason: Option<&CloseReason>,
) -> uptrakit_internal_wire::TransportClosePolicy {
    uptrakit_internal_wire::TransportClosePolicy::Reconnect {
        reason: reason.cloned(),
    }
}

/// Lossy Layer-2 -> Layer-1 bridge for generic transport callers.
///
/// Production lifecycle code does not use this impl for receive-side logic.
/// It calls [`ControllerConnection::recv()`] directly to retain typed
/// `EnrollmentError` information.
#[async_trait::async_trait]
impl uptrakit_internal_wire::ServiceTransport for ControllerConnection {
    async fn transport_send(
        &mut self,
        msg: uptrakit_internal_wire::ServiceMessage,
    ) -> std::result::Result<(), uptrakit_internal_wire::TransportError> {
        self.send(msg)
            .await
            .map_err(|_| uptrakit_internal_wire::TransportError)
    }

    async fn transport_send_best_effort(&mut self, msg: uptrakit_internal_wire::ServiceMessage) {
        self.send_best_effort(msg).await;
    }

    async fn transport_send_auto_paginate(
        &mut self,
        msg: uptrakit_internal_wire::ServiceMessage,
    ) -> std::result::Result<(), uptrakit_internal_wire::TransportError> {
        self.send_auto_paginate(msg)
            .await
            .map_err(|_| uptrakit_internal_wire::TransportError)
    }

    /// Bridge `ControllerConnection::recv()` into Layer 1 `Option` semantics.
    ///
    /// All `Err` values are intentionally erased to `None` for the
    /// `ServiceTransport` contract.
    ///
    /// This method exists for trait compliance. The authenticated event loop
    /// calls [`ControllerConnection::recv()`] directly.
    async fn transport_recv(&mut self) -> Option<uptrakit_internal_wire::ControllerMessage> {
        match self.recv().await {
            Ok(msg) => msg,
            Err(e) => {
                tracing::debug!(error = %e, "transport_recv: connection error");
                None
            }
        }
    }

    fn close_policy(&self) -> uptrakit_internal_wire::TransportClosePolicy {
        close_reason_to_policy(self.close_reason())
    }
}

/// Background task that drains the write channel and sends frames to the
/// WebSocket sink.
///
/// On I/O error or timeout, sets the `write_error` flag and exits. The
/// connection's `recv()` and `send()` methods check this flag and surface
/// the error to the caller.
async fn writer_task(
    mut sink: WsSink,
    mut rx: mpsc::Receiver<OutboundFrame>,
    write_error: Arc<AtomicBool>,
) {
    while let Some(frame) = rx.recv().await {
        let result = match frame {
            OutboundFrame::Text(json) => {
                tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Text(json.into()))).await
            }
            OutboundFrame::Pong(data) => {
                tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Pong(data.into()))).await
            }
            OutboundFrame::Close => {
                // Best-effort close — ignore errors (peer may have already gone).
                let _ = sink.close().await;
                return;
            }
        };

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if !is_peer_closed(&e) {
                    tracing::error!(error = %e, "writer task: WebSocket send failed");
                }
                write_error.store(true, Ordering::Relaxed);
                return;
            }
            Err(_) => {
                tracing::error!("writer task: WebSocket send timed out after {SEND_TIMEOUT:?}");
                write_error.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
    // Channel closed (connection dropped) — clean up.
    let _ = sink.close().await;
}

#[cfg(test)]
mod tests {
    use super::{RecvAction, classify_ws_frame, close_reason_to_policy};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::{Error as WsErr, Message};
    use uptrakit_internal_wire::{CloseReason, TransportClosePolicy};

    struct EmptyReadStream;

    impl futures_util::Stream for EmptyReadStream {
        type Item = std::result::Result<Message, WsErr>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(None)
        }
    }

    #[test]
    fn classify_ws_frame_text_is_message() {
        let action = classify_ws_frame(Some(Ok(Message::Text("hello".into()))));
        assert!(matches!(action, RecvAction::Message(Message::Text(_))));
    }

    #[test]
    fn classify_ws_frame_close_is_close_frame() {
        let action = classify_ws_frame(Some(Ok(Message::Close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "bye".into(),
        })))));
        assert!(matches!(action, RecvAction::CloseFrame(Some(_))));
    }

    #[test]
    fn classify_ws_frame_peer_closed_unexpected_eof() {
        let action = classify_ws_frame(Some(Err(WsErr::Io(std::io::Error::from(
            std::io::ErrorKind::UnexpectedEof,
        )))));
        assert!(matches!(action, RecvAction::PeerClosed));
    }

    #[test]
    fn classify_ws_frame_non_peer_closed_error_is_transport_error() {
        let action = classify_ws_frame(Some(Err(WsErr::Io(std::io::Error::other("other")))));
        assert!(matches!(action, RecvAction::TransportError(_)));
    }

    #[test]
    fn classify_ws_frame_none_is_stream_end() {
        let action = classify_ws_frame(None);
        assert!(matches!(action, RecvAction::StreamEnd));
    }

    #[test]
    fn close_reason_none_returns_reconnect_no_reason() {
        assert_eq!(
            close_reason_to_policy(None),
            TransportClosePolicy::Reconnect { reason: None },
        );
    }

    #[test]
    fn close_reason_cert_rotated_returns_reconnect_with_reason() {
        assert_eq!(
            close_reason_to_policy(Some(&CloseReason::CertificateRotated)),
            TransportClosePolicy::Reconnect {
                reason: Some(CloseReason::CertificateRotated),
            },
        );
    }

    #[test]
    fn close_reason_cert_revoked_returns_reconnect_with_reason() {
        assert_eq!(
            close_reason_to_policy(Some(&CloseReason::CertificateRevoked)),
            TransportClosePolicy::Reconnect {
                reason: Some(CloseReason::CertificateRevoked),
            },
        );
    }

    #[test]
    fn close_reason_unknown_returns_reconnect_with_reason() {
        let reason = CloseReason::Unknown("future reason".to_string());
        assert_eq!(
            close_reason_to_policy(Some(&reason)),
            TransportClosePolicy::Reconnect {
                reason: Some(reason),
            },
        );
    }

    #[test]
    fn close_reason_protocol_error_returns_reconnect_with_reason() {
        assert_eq!(
            close_reason_to_policy(Some(&CloseReason::ProtocolError)),
            TransportClosePolicy::Reconnect {
                reason: Some(CloseReason::ProtocolError),
            },
        );
    }

    #[test]
    fn close_reason_to_policy_never_returns_shutdown() {
        let variants = vec![
            CloseReason::CertificateRotated,
            CloseReason::CertificateRevoked,
            CloseReason::NoValidCertificate,
            CloseReason::InternalError,
            CloseReason::CertificateNotRecognized,
            CloseReason::ServiceDeactivated,
            CloseReason::ServiceNotApproved,
            CloseReason::ServiceNotFound,
            CloseReason::EnrollmentTimeout,
            CloseReason::RateLimitExceeded,
            CloseReason::ProtocolError,
            CloseReason::Superseded,
            CloseReason::Unknown("unknown".to_string()),
        ];

        for variant in variants {
            assert_eq!(
                close_reason_to_policy(Some(&variant)),
                TransportClosePolicy::Reconnect {
                    reason: Some(variant.clone()),
                },
            );
        }
    }

    #[test]
    fn controller_connection_overrides_close_policy() {
        let source = include_str!("connection.rs");
        let impl_marker =
            "\nimpl uptrakit_internal_wire::ServiceTransport for ControllerConnection {";
        let impl_start = source
            .find(impl_marker)
            .expect("ServiceTransport impl for ControllerConnection should exist")
            + impl_marker.len();
        let rest = &source[impl_start..];

        let next_top_level_item = ["\nimpl ", "\nfn ", "\nstruct ", "\nenum ", "\nmod ", "\n#["]
            .iter()
            .filter_map(|marker| rest.find(marker))
            .min()
            .unwrap_or(rest.len());
        let impl_body = &rest[..next_top_level_item];
        let close_policy_marker = "fn close_policy(&self)";
        let close_policy_start = impl_body
            .find(close_policy_marker)
            .expect("ControllerConnection must override close_policy() in ServiceTransport impl");
        let close_policy_tail = &impl_body[close_policy_start..];
        let close_policy_end = close_policy_tail
            .find("\n    }\n")
            .map(|idx| idx + "\n    }\n".len())
            .unwrap_or(close_policy_tail.len());
        let close_policy_body = &close_policy_tail[..close_policy_end];

        assert!(
            close_policy_body.contains("close_reason_to_policy(self.close_reason())"),
            "ControllerConnection close_policy() must delegate to close_reason_to_policy(self.close_reason())"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn new_test_close_exits_writer_task_on_close_frame() {
        let read = Box::pin(EmptyReadStream)
            as Pin<
                Box<dyn futures_util::Stream<Item = std::result::Result<Message, WsErr>> + Send>,
            >;
        let mut conn = super::ControllerConnection::new_test(read);
        let close_result =
            tokio::time::timeout(std::time::Duration::from_secs(5), conn.close()).await;
        assert!(
            close_result.is_ok(),
            "close should complete quickly when writer sees OutboundFrame::Close"
        );
        assert!(close_result.expect("timeout wrapper should be Ok").is_ok());
    }
}
