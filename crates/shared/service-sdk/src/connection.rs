//! Shared authenticated WebSocket connection to the controller.
//!
//! [`ControllerConnection`] wraps a TLS WebSocket stream with automatic
//! envelope serialization, sequence validation, and WebSocket frame handling.
//! Both the agent and the MQTT service use this type for their authenticated
//! event loops.

use std::collections::BTreeSet;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rootcause::prelude::*;
use serde::Deserialize;
use tokio_rustls::TlsConnector;
use uptrakit_internal_wire::{
    CURRENT_PROTOCOL_VERSION, Capability, CloseReason, ControllerEnvelope, ControllerMessage,
    IncomingSeq, OutgoingSeq, Paginatable, ServiceMessage, paginate::paginate_payload,
};

use crate::error::{EnrollmentError, ProtocolError, Result};
use crate::ws::{WsStream, connect_ws, is_peer_closed, log_close_frame};

/// Maximum time to wait for a single WebSocket write to complete.
///
/// If the controller stops consuming data (e.g. after a restart), the TCP
/// send buffer fills and writes block indefinitely. This timeout bounds
/// the worst-case hang to 30 seconds, after which the connection is treated
/// as dead and the agent reconnects.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

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
pub struct ControllerConnection {
    ws: WsStream,
    out_seq: OutgoingSeq,
    in_seq: IncomingSeq,
    close_reason: Option<CloseReason>,
    agreed_capabilities: BTreeSet<Capability>,
}

impl ControllerConnection {
    /// Connect TCP -> TLS -> WebSocket to the controller.
    ///
    /// Reuses the shared [`connect_ws`] from `ws.rs`. Pass `auth_header` for
    /// bearer-token connections during enrollment resume.
    pub async fn connect(
        host: &str,
        port: u16,
        tls_connector: &TlsConnector,
        auth_header: Option<&str>,
    ) -> Result<Self> {
        tracing::debug!(host, port, "connecting to controller");
        // Authenticated connections use mTLS identity — no service_id query param needed.
        let ws = connect_ws(host, port, tls_connector, auth_header, None).await?;
        tracing::debug!("WebSocket connection established");
        Ok(Self {
            ws,
            out_seq: OutgoingSeq::new(),
            in_seq: IncomingSeq::new(),
            close_reason: None,
            agreed_capabilities: BTreeSet::new(),
        })
    }

    /// Send a [`ServiceMessage`] to the controller.
    ///
    /// Wraps the message in a [`ServiceEnvelope`](uptrakit_internal_wire::ServiceEnvelope)
    /// with the next sequence number, serializes to JSON, and sends as a
    /// WebSocket text frame.
    #[tracing::instrument(skip_all)]
    pub async fn send(&mut self, msg: ServiceMessage) -> Result<()> {
        use tokio_tungstenite::tungstenite::Message;

        tracing::trace!("sending message to controller");
        let envelope = self
            .out_seq
            .wrap_service(msg, uptrakit_internal_wire::current_trace_context());
        let json = serde_json::to_string(&envelope).context_to::<EnrollmentError>()?;
        tokio::time::timeout(SEND_TIMEOUT, self.ws.send(Message::Text(json.into())))
            .await
            .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::SendTimeout)))?
            .context_to::<EnrollmentError>()?;
        Ok(())
    }

    /// Receive the next [`ControllerMessage`] from the controller.
    ///
    /// Returns `Ok(None)` on clean close or peer-closed.
    ///
    /// Automatically:
    /// - Responds to WebSocket `Ping` with `Pong`
    /// - Skips `Pong` and `Frame` messages
    /// - Stores close frame reason (accessible via [`close_reason`](Self::close_reason))
    /// - Validates incoming sequence numbers
    #[tracing::instrument(skip_all)]
    pub async fn recv(&mut self) -> Result<Option<ControllerMessage>> {
        use tokio_tungstenite::tungstenite::Message;

        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    // Extract and validate protocol version and sequence number
                    // first, before attempting to deserialize the full message.
                    // This ensures the counter stays in sync even when the
                    // payload contains an unknown variant.
                    let header: EnvelopeHeader =
                        serde_json::from_str(&text).context_to::<EnrollmentError>()?;
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
                    let envelope: ControllerEnvelope = match serde_json::from_str(&text) {
                        Ok(env) => env,
                        Err(e) => {
                            tracing::debug!("ignoring unrecognized controller message: {e}");
                            continue;
                        }
                    };
                    tracing::trace!(msg_type = ?envelope.message, "received message from controller");
                    return Ok(Some(envelope.message));
                }
                Some(Ok(Message::Binary(data))) => {
                    let header: EnvelopeHeader =
                        serde_json::from_slice(&data).context_to::<EnrollmentError>()?;
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
                    let envelope: ControllerEnvelope = match serde_json::from_slice(&data) {
                        Ok(env) => env,
                        Err(e) => {
                            tracing::debug!("ignoring unrecognized controller binary message: {e}");
                            continue;
                        }
                    };
                    tracing::trace!(msg_type = ?envelope.message, "received message from controller");
                    return Ok(Some(envelope.message));
                }
                Some(Ok(Message::Ping(data))) => {
                    tracing::trace!("received ping from controller");
                    tokio::time::timeout(SEND_TIMEOUT, self.ws.send(Message::Pong(data)))
                        .await
                        .map_err(|_| {
                            report!(EnrollmentError::Protocol(ProtocolError::SendTimeout))
                        })?
                        .context_to::<EnrollmentError>()?;
                    continue;
                }
                Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(Message::Close(frame))) => {
                    self.close_reason = frame.as_ref().map(|f| {
                        f.reason
                            .parse::<CloseReason>()
                            .unwrap_or_else(|_| CloseReason::Unknown(f.reason.to_string()))
                    });
                    log_close_frame(frame);
                    return Ok(None);
                }
                Some(Ok(Message::Frame(_))) => continue,
                Some(Err(e)) if is_peer_closed(&e) => {
                    tracing::info!("connection closed by controller");
                    return Ok(None);
                }
                Some(Err(e)) => {
                    return Err(e).context_to::<EnrollmentError>()?;
                }
                None => {
                    return Ok(None);
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

    /// Send a [`Paginatable`] payload, automatically splitting it into pages
    /// when the serialized size exceeds the pagination threshold.
    ///
    /// Small payloads are sent as a single message with no pagination metadata
    /// (zero overhead). Large payloads are split into pages, each sent as a
    /// separate WebSocket frame with pagination metadata attached.
    ///
    /// Each page is a complete, independently processable message. The
    /// controller tracks page arrival and defers only lightweight finalization
    /// (e.g. notification emission) until the final page.
    pub async fn send_paginated<P: Paginatable>(&mut self, payload: P) -> Result<()> {
        let pages = paginate_payload(payload).context_to::<EnrollmentError>()?;
        let page_count = pages.len();

        for (i, page) in pages.into_iter().enumerate() {
            let msg = page.payload.into_message();
            let envelope = self.out_seq.wrap_service_paginated(
                msg,
                uptrakit_internal_wire::current_trace_context(),
                page.pagination,
            );
            let json = serde_json::to_string(&envelope).context_to::<EnrollmentError>()?;

            if let Err(e) = tokio::time::timeout(
                SEND_TIMEOUT,
                self.ws
                    .send(tokio_tungstenite::tungstenite::Message::Text(json.into())),
            )
            .await
            .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::SendTimeout)))
            {
                tracing::error!(
                    page = i + 1,
                    total_pages = page_count,
                    error = %e,
                    "failed to send paginated message page"
                );
                return Err(e);
            }

            if page_count > 1 {
                tracing::debug!(
                    page = i + 1,
                    total_pages = page_count,
                    "sent paginated message page"
                );
            }
        }

        Ok(())
    }

    /// Send a [`ServiceMessage`], automatically paginating paginatable payload
    /// types when they exceed the wire size threshold.
    ///
    /// Paginatable variants (`DiscoveryResults`, `VersionCheckResults`,
    /// `ReportHosts`, `BatchHostPackageUpdateResult`) are split into pages
    /// transparently. All other variants are sent as a single message.
    ///
    /// This is the recommended method for sending report-style messages.
    pub async fn send_auto_paginate(&mut self, msg: ServiceMessage) -> Result<()> {
        match msg {
            ServiceMessage::DiscoveryResults(payload) => self.send_paginated(payload).await,
            ServiceMessage::VersionCheckResults(payload) => self.send_paginated(payload).await,
            ServiceMessage::ReportHosts(payload) => self.send_paginated(payload).await,
            ServiceMessage::BatchHostPackageUpdateResult(payload) => {
                self.send_paginated(payload).await
            }
            other => self.send(other).await,
        }
    }

    /// Send a [`ServiceMessage`] on a best-effort basis.
    ///
    /// Logs a warning on failure instead of propagating the error. Useful for
    /// status/output messages where a send failure should not terminate the
    /// connection loop.
    pub async fn send_best_effort(&mut self, msg: ServiceMessage) {
        if let Err(e) = self.send(msg).await {
            tracing::warn!(error = %e, "best-effort send failed");
        }
    }

    /// Graceful WebSocket close (best-effort, tolerates peer-closed).
    pub async fn close(&mut self) -> Result<()> {
        match self.ws.close(None).await {
            Ok(()) => {
                tracing::info!("websocket closed gracefully");
                Ok(())
            }
            Err(e) if is_peer_closed(&e) => {
                tracing::info!("websocket already closed by peer");
                Ok(())
            }
            Err(e) => Err(e).context_to::<EnrollmentError>()?,
        }
    }
}
