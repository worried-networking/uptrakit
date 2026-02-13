//! Shared authenticated WebSocket connection to the controller.
//!
//! [`ControllerConnection`] wraps a TLS WebSocket stream with automatic
//! envelope serialization, sequence validation, and WebSocket frame handling.
//! Both the agent and the MQTT service use this type for their authenticated
//! event loops.

use futures_util::{SinkExt, StreamExt};
use rootcause::prelude::*;
use tokio_rustls::TlsConnector;
use uptrakit_internal_wire::{
    ControllerEnvelope, ControllerMessage, IncomingSeq, OutgoingSeq, ServiceMessage,
};

use crate::error::{EnrollmentError, Result};
use crate::ws::{WsStream, connect_ws, is_peer_closed, log_close_frame};

/// Authenticated WebSocket connection to the controller.
///
/// Handles envelope wrapping, sequence validation, and WebSocket frame
/// processing (Ping/Pong, Close frames, peer-closed detection).
pub struct ControllerConnection {
    ws: WsStream,
    out_seq: OutgoingSeq,
    in_seq: IncomingSeq,
    close_reason: Option<String>,
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
        let ws = connect_ws(host, port, tls_connector, auth_header).await?;
        Ok(Self {
            ws,
            out_seq: OutgoingSeq::new(),
            in_seq: IncomingSeq::new(),
            close_reason: None,
        })
    }

    /// Send a [`ServiceMessage`] to the controller.
    ///
    /// Wraps the message in a [`ServiceEnvelope`](uptrakit_internal_wire::ServiceEnvelope)
    /// with the next sequence number, serializes to JSON, and sends as a
    /// WebSocket text frame.
    pub async fn send(&mut self, msg: ServiceMessage) -> Result<()> {
        use tokio_tungstenite::tungstenite::Message;

        let envelope = self.out_seq.wrap_service(msg);
        let json = serde_json::to_string(&envelope).context_to::<EnrollmentError>()?;
        self.ws
            .send(Message::Text(json.into()))
            .await
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
    pub async fn recv(&mut self) -> Result<Option<ControllerMessage>> {
        use tokio_tungstenite::tungstenite::Message;

        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let envelope: ControllerEnvelope = match serde_json::from_str(&text) {
                        Ok(env) => env,
                        Err(e) => {
                            tracing::debug!("ignoring unrecognized controller message: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = self.in_seq.validate(envelope.seq) {
                        return Err(report!(EnrollmentError::Enrollment(format!(
                            "sequence validation failed: {e}"
                        ))));
                    }
                    return Ok(Some(envelope.message));
                }
                Some(Ok(Message::Binary(data))) => {
                    let envelope: ControllerEnvelope =
                        serde_json::from_slice(&data).context_to::<EnrollmentError>()?;
                    if let Err(e) = self.in_seq.validate(envelope.seq) {
                        return Err(report!(EnrollmentError::Enrollment(format!(
                            "sequence validation failed: {e}"
                        ))));
                    }
                    return Ok(Some(envelope.message));
                }
                Some(Ok(Message::Ping(data))) => {
                    self.ws
                        .send(Message::Pong(data))
                        .await
                        .context_to::<EnrollmentError>()?;
                    continue;
                }
                Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(Message::Close(frame))) => {
                    self.close_reason =
                        frame.as_ref().map(|f| f.reason.to_string());
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
    /// Used by the agent to distinguish "certificate rotated" from other
    /// close reasons.
    pub fn close_reason(&self) -> Option<&str> {
        self.close_reason.as_deref()
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
