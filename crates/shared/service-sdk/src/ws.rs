//! WebSocket connection and enrollment protocol.
//!
//! Provides shared WebSocket helpers for the enrollment flow used by both
//! agents and MQTT services:
//! - Manual TCP → TLS → WebSocket upgrade (using `tokio-tungstenite::client_async`)
//! - `send_enroll` / `wait_for_approval` / `request_certificate_ws`
//! - Full `run_enrollment` and `resume_enrollment` orchestrators

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http::Uri;
use rootcause::prelude::*;
use rustls::pki_types::ServerName;
use std::collections::BTreeSet;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

use crate::wire_api::{
    CURRENT_PROTOCOL_VERSION, Capability, CertificatePayload, ControllerEnvelope,
    ControllerMessage, EnrollPayload, EnrolledPayload, EnrollmentStatus, IncomingSeq, OutgoingSeq,
    RequestCertificatePayload, SecretString, ServiceMessage,
};

use crate::error::{EnrollmentError, IdentityError, ProtocolError, Result};
use crate::signal::SignalWatcher;

/// Timeout for TCP connection establishment.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for immediate request-response exchanges (enroll, certificate request).
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout for waiting for approval from the controller.
/// Approval may require human interaction, so this is generous.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Type alias for the WebSocket stream produced by [`connect_ws`].
pub(crate) type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

/// Write half of a split [`WsStream`].
pub(crate) type WsSink =
    futures_util::stream::SplitSink<WsStream, tokio_tungstenite::tungstenite::Message>;

/// Read half of a split [`WsStream`].
pub(crate) type WsRead = futures_util::stream::SplitStream<WsStream>;

/// Split a WebSocket stream into write/read halves using shared aliases.
pub(crate) fn split_ws_stream(ws: WsStream) -> (WsSink, WsRead) {
    ws.split()
}

/// Connect TCP → TLS → WebSocket upgrade, with optional Authorization header.
///
/// When `service_id` is `Some`, it is appended as a `service_id` query
/// parameter to the WebSocket URL. The controller uses this to narrow the
/// enrollment-secret lookup to the specific service, preventing cross-tenant
/// secret collisions during the narrow pre-certificate window.
#[expect(
    clippy::map_err_ignore,
    reason = "tokio::time::error::Elapsed carries no additional context beyond the timeout itself"
)]
pub(crate) async fn connect_ws(
    host: &str,
    port: u16,
    tls_connector: &TlsConnector,
    auth_header: Option<&str>,
    service_id: Option<Uuid>,
    signals: &mut SignalWatcher,
) -> Result<WsStream> {
    let ws_url = match service_id {
        Some(id) => format!("wss://{host}:{port}/api/v1/ws/service?service_id={id}"),
        None => format!("wss://{host}:{port}/api/v1/ws/service"),
    };
    tracing::info!(url = %ws_url, "connecting to controller");
    tracing::debug!(host, port, "opening TCP connection");

    let tcp_stream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::ConnectionTimeout)))?
    .context_to::<EnrollmentError>()?;

    // Configure TCP keepalive for faster dead-connection detection.
    // Without keepalive, OS defaults (macOS: 2 h, Linux: ~2 h) apply during
    // idle periods, causing recv() to block for hours on a dead connection.
    // We set idle time and probe interval; the OS default retry count
    // (8-9 on macOS, 9 on Linux) is sufficient for our purposes.
    {
        use socket2::{SockRef, TcpKeepalive};
        let keepalive = TcpKeepalive::new()
            .with_time(Duration::from_secs(30))
            .with_interval(Duration::from_secs(10));
        SockRef::from(&tcp_stream)
            .set_tcp_keepalive(&keepalive)
            .context_to::<EnrollmentError>()?;
    }

    let server_name = ServerName::try_from(host.to_string()).context_to::<EnrollmentError>()?;

    // TLS handshake: bound with CONNECT_TIMEOUT (a wedged-but-accepting peer
    // could otherwise hang here indefinitely) and interruptible by shutdown
    // signals so Ctrl+C exits cleanly during enrollment.
    let tls_stream = tokio::select! {
        biased;
        signal = signals.recv() => {
            tracing::info!(%signal, "received signal during TLS handshake, exiting");
            bail!(EnrollmentError::Cancelled(signal));
        }
        result = tokio::time::timeout(CONNECT_TIMEOUT, tls_connector.connect(server_name, tcp_stream)) => {
            result
                .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::ConnectionTimeout)))?
                .context_to::<EnrollmentError>()?
        }
    };

    let uri: Uri = ws_url.parse().context_to::<EnrollmentError>()?;
    let mut request = uri
        .to_string()
        .into_client_request()
        .context_to::<EnrollmentError>()?;

    tracing::debug!("upgrading to WebSocket");
    if let Some(header_value) = auth_header {
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(header_value).map_err(|e| {
                report!(EnrollmentError::Protocol(ProtocolError::Enrollment(
                    format!("invalid authorization header: {e}")
                )))
            })?,
        );
    }

    // WS upgrade: bound with CONNECT_TIMEOUT and interruptible by shutdown
    // signals — a server that accepts TCP+TLS but never returns the HTTP 101
    // upgrade response would otherwise hang here indefinitely.
    let (ws_stream, _response) = tokio::select! {
        biased;
        signal = signals.recv() => {
            tracing::info!(%signal, "received signal during WebSocket upgrade, exiting");
            bail!(EnrollmentError::Cancelled(signal));
        }
        result = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::client_async(request, tls_stream)) => {
            result
                .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::ConnectionTimeout)))?
                .context_to::<EnrollmentError>()?
        }
    };

    tracing::info!("WebSocket connected");
    Ok(ws_stream)
}

/// Send Enroll message and read Enrolled response.
///
/// Times out after [`RESPONSE_TIMEOUT`] (60 seconds).
#[expect(
    clippy::map_err_ignore,
    reason = "tokio::time::error::Elapsed carries no additional context beyond the timeout itself"
)]
pub(crate) async fn send_enroll(
    ws: &mut WsStream,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    payload: EnrollPayload,
    signals: &mut SignalWatcher,
) -> Result<EnrolledPayload> {
    tracing::trace!("sending Enroll message");
    let msg = ServiceMessage::Enroll(payload);
    let json =
        serde_json::to_string(&out_seq.wrap_service(msg, crate::wire_api::current_trace_context()))
            .context_to::<EnrollmentError>()?;
    ws.send(Message::Text(json.into()))
        .await
        .context_to::<EnrollmentError>()?;

    tracing::info!("sent Enroll, waiting for Enrolled response");

    tokio::time::timeout(RESPONSE_TIMEOUT, async {
        loop {
            let resp = tokio::select! {
                biased;
                signal = signals.recv() => {
                    tracing::info!(%signal, "received signal during enrollment, exiting");
                    bail!(EnrollmentError::Cancelled(signal));
                }
                next = ws.next() => next,
            }
            .ok_or_else(|| report!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed)))?
            .context_to::<EnrollmentError>()?;

            match resp {
                Message::Text(text) => {
                    let envelope: ControllerEnvelope =
                        serde_json::from_str(&text).context_to::<EnrollmentError>()?;

                    if envelope.protocol_version != CURRENT_PROTOCOL_VERSION {
                        bail!(EnrollmentError::Protocol(ProtocolError::VersionMismatch {
                            expected: CURRENT_PROTOCOL_VERSION,
                            received: envelope.protocol_version,
                        }));
                    }

                    if let Err(e) = in_seq.validate(envelope.seq) {
                        bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(format!(
                            "sequence validation failed: {e}"
                        ))));
                    }

                    match envelope.message {
                        ControllerMessage::Enrolled(payload) => return Ok(payload),
                        ControllerMessage::Error(err) => {
                            bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(format!(
                                "{}: {}",
                                err.code, err.message
                            ))));
                        }
                        ControllerMessage::ServerRestarting(payload) => {
                            tracing::info!(reason = %payload.reason, "controller is restarting during enrollment");
                            bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed));
                        }
                        _ => {
                            bail!(EnrollmentError::Protocol(ProtocolError::UnexpectedMessage));
                        }
                    }
                }
                Message::Close(frame) => {
                    log_close_frame(frame);
                    bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed));
                }
                _ => continue,
            }
        }
    })
    .await
    .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::ResponseTimeout)))?
}

/// Wait for Approved/Rejected push from controller.
///
/// Returns the approved `Uuid` on `Approved`, errors on `Rejected`.
/// Times out after [`APPROVAL_TIMEOUT`] (30 minutes).
///
/// Generic over the underlying I/O stream so tests can drive it over an
/// in-process duplex pair instead of a TLS-backed TCP socket.
///
/// # Errors
///
/// Returns an error if the connection is closed, the controller rejects the
/// enrollment, a protocol error occurs, or the approval timeout elapses.
#[expect(
    clippy::map_err_ignore,
    reason = "tokio::time::error::Elapsed carries no additional context beyond the timeout itself"
)]
pub(crate) async fn wait_for_approval<S>(
    ws: &mut WebSocketStream<S>,
    in_seq: &mut IncomingSeq,
    signals: &mut SignalWatcher,
) -> Result<Uuid>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tracing::info!("waiting for approval...");

    tokio::time::timeout(APPROVAL_TIMEOUT, async {
        loop {
            let next = tokio::select! {
                biased;
                signal = signals.recv() => {
                    tracing::info!(%signal, "received signal during enrollment, exiting");
                    bail!(EnrollmentError::Cancelled(signal));
                }
                next = ws.next() => next,
            };
            let msg = match next {
                Some(Ok(m)) => m,
                Some(Err(e)) if is_peer_closed(&e) => {
                    tracing::info!("connection closed by controller while waiting for approval");
                    bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed));
                }
                Some(Err(e)) => return Err(e).context_to::<EnrollmentError>()?,
                None => bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed)),
            };

            match msg {
                Message::Text(text) => {
                    let envelope: ControllerEnvelope =
                        serde_json::from_str(&text).context_to::<EnrollmentError>()?;

                    if envelope.protocol_version != CURRENT_PROTOCOL_VERSION {
                        bail!(EnrollmentError::Protocol(ProtocolError::VersionMismatch {
                            expected: CURRENT_PROTOCOL_VERSION,
                            received: envelope.protocol_version,
                        }));
                    }

                    if let Err(e) = in_seq.validate(envelope.seq) {
                        bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(format!(
                            "sequence validation failed: {e}"
                        ))));
                    }

                    match envelope.message {
                        ControllerMessage::Approved(payload) => {
                            tracing::info!(service_id = %payload.service_id, "enrollment approved");
                            return Ok(payload.service_id);
                        }
                        ControllerMessage::Rejected(payload) => {
                            tracing::error!(service_id = %payload.service_id, "enrollment rejected");
                            bail!(EnrollmentError::Protocol(ProtocolError::EnrollmentRejected));
                        }
                        ControllerMessage::Pong(_) => {
                            tracing::trace!("received pong during enrollment wait");
                            continue;
                        }
                        ControllerMessage::Error(err) => {
                            bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(format!(
                                "{}: {}",
                                err.code, err.message
                            ))));
                        }
                        ControllerMessage::ServerRestarting(payload) => {
                            tracing::info!(reason = %payload.reason, "controller is restarting while waiting for approval");
                            bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed));
                        }
                        _ => continue,
                    }
                }
                Message::Close(frame) => {
                    log_close_frame(frame);
                    bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed));
                }
                _ => continue,
            }
        }
    })
    .await
    .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::ApprovalTimeout)))?
}

/// Send `RequestCertificate` with a CSR and read `Certificate` response.
///
/// Times out after [`RESPONSE_TIMEOUT`] (60 seconds).
///
/// Generic over the underlying I/O stream so tests can drive it over an
/// in-process duplex pair instead of a TLS-backed TCP socket.
///
/// # Errors
///
/// Returns an error if the connection is closed, the controller sends an
/// error or rejection, a protocol error occurs, or the response timeout
/// elapses.
#[expect(
    clippy::map_err_ignore,
    reason = "tokio::time::error::Elapsed carries no additional context beyond the timeout itself"
)]
pub(crate) async fn request_certificate_ws<S>(
    ws: &mut WebSocketStream<S>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    csr_pem: &str,
    signals: &mut SignalWatcher,
) -> Result<CertificatePayload>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tracing::debug!("sending certificate request");
    let msg = ServiceMessage::RequestCertificate(RequestCertificatePayload {
        csr_pem: csr_pem.to_string(),
    });
    let json =
        serde_json::to_string(&out_seq.wrap_service(msg, crate::wire_api::current_trace_context()))
            .context_to::<EnrollmentError>()?;
    ws.send(Message::Text(json.into()))
        .await
        .context_to::<EnrollmentError>()?;

    tracing::info!("sent RequestCertificate, waiting for Certificate response");

    tokio::time::timeout(RESPONSE_TIMEOUT, async {
        loop {
            let resp = tokio::select! {
                biased;
                signal = signals.recv() => {
                    tracing::info!(%signal, "received signal during enrollment, exiting");
                    bail!(EnrollmentError::Cancelled(signal));
                }
                next = ws.next() => next,
            }
            .ok_or_else(|| report!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed)))?
            .context_to::<EnrollmentError>()?;

            match resp {
                Message::Text(text) => {
                    let envelope: ControllerEnvelope =
                        serde_json::from_str(&text).context_to::<EnrollmentError>()?;

                    if envelope.protocol_version != CURRENT_PROTOCOL_VERSION {
                        bail!(EnrollmentError::Protocol(ProtocolError::VersionMismatch {
                            expected: CURRENT_PROTOCOL_VERSION,
                            received: envelope.protocol_version,
                        }));
                    }

                    if let Err(e) = in_seq.validate(envelope.seq) {
                        bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(format!(
                            "sequence validation failed: {e}"
                        ))));
                    }

                    match envelope.message {
                        ControllerMessage::Certificate(payload) => return Ok(payload),
                        ControllerMessage::Approved(payload) => {
                            tracing::debug!(
                                service_id = %payload.service_id,
                                "received approved message while waiting for certificate"
                            );
                            continue;
                        }
                        ControllerMessage::Pong(_) => {
                            tracing::trace!("received pong while waiting for certificate");
                            continue;
                        }
                        ControllerMessage::Error(err) => {
                            bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(format!(
                                "{}: {}",
                                err.code, err.message
                            ))));
                        }
                        ControllerMessage::ServerRestarting(payload) => {
                            tracing::info!(reason = %payload.reason, "controller is restarting during certificate request");
                            bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed));
                        }
                        ControllerMessage::Rejected(_) => {
                            // Bail immediately rather than waiting for the 60s RESPONSE_TIMEOUT
                            // to expire — the service was rejected after approval was confirmed.
                            bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(
                                "service was rejected while waiting for certificate".to_string(),
                            )));
                        }
                        _ => {
                            // The server may push non-enrollment messages (e.g.
                            // HostConnectivityUpdated) to this connection while the cert request
                            // is in flight — the service is registered in service_connections
                            // with its full capability set before the certificate is issued.
                            // Ignore and keep waiting for Certificate.
                            tracing::debug!(
                                "ignoring unexpected push message while waiting for Certificate"
                            );
                            continue;
                        }
                    }
                }
                Message::Close(frame) => {
                    log_close_frame(frame);
                    bail!(EnrollmentError::Protocol(ProtocolError::ReceiveClosed));
                }
                _ => continue,
            }
        }
    })
    .await
    .map_err(|_| report!(EnrollmentError::Protocol(ProtocolError::ResponseTimeout)))?
}

/// Log a WebSocket close frame.
pub(crate) fn log_close_frame(frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>) {
    match frame {
        Some(frame) => {
            tracing::warn!(
                code = %frame.code,
                reason = %frame.reason,
                "connection closed by controller: {}", frame.reason
            );
        }
        None => {
            tracing::info!("connection closed by controller");
        }
    }
}

/// Returns `true` when the error indicates the peer dropped the TCP
/// connection without sending a TLS `close_notify`. This is normal
/// when the controller terminates a connection (e.g. service deactivated,
/// network interruption, or controller restart).
pub(crate) fn is_peer_closed(err: &tokio_tungstenite::tungstenite::Error) -> bool {
    use std::io::ErrorKind;
    use tokio_tungstenite::tungstenite::Error as WsErr;
    use tokio_tungstenite::tungstenite::error::ProtocolError;
    match err {
        WsErr::Io(io) => matches!(
            io.kind(),
            ErrorKind::UnexpectedEof
                | ErrorKind::BrokenPipe
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::NotConnected
        ),
        WsErr::Protocol(
            ProtocolError::ResetWithoutClosingHandshake | ProtocolError::SendAfterClosing,
        ) => true,
        _ => false,
    }
}

/// Parameters for [`run_enrollment`].
pub(crate) struct EnrollmentParams<'a> {
    pub identity: &'a mut crate::identity::ServiceIdentityState,
    pub host: &'a str,
    pub port: u16,
    pub tls_connector: &'a TlsConnector,
    pub hostname: &'a str,
    pub friendly_name: &'a str,
    pub enrollment_token: Option<&'a str>,
    pub capabilities: BTreeSet<Capability>,
    pub service_app_name: &'a str,
    pub signals: &'a mut SignalWatcher,
}

/// Run a fresh enrollment flow: enroll → wait for approval → generate CSR → request certificate.
///
/// On success, the identity is fully certified (service_id, key, and cert persisted).
pub(crate) async fn run_enrollment(params: EnrollmentParams<'_>) -> Result<()> {
    let EnrollmentParams {
        identity,
        host,
        port,
        tls_connector,
        hostname,
        friendly_name,
        enrollment_token,
        capabilities,
        service_app_name,
        signals,
    } = params;
    // Fresh enrollment: no service_id yet, so no query parameter.
    let mut ws = connect_ws(host, port, tls_connector, None, None, signals).await?;
    let mut out_seq = OutgoingSeq::new();
    let mut in_seq = IncomingSeq::new();

    let enrolled = send_enroll(
        &mut ws,
        &mut out_seq,
        &mut in_seq,
        EnrollPayload {
            hostname: hostname.to_string(),
            friendly_name: friendly_name.to_string(),
            enrollment_token: enrollment_token.map(SecretString::new),
            capabilities,
            service_app_name: service_app_name.to_string(),
        },
        signals,
    )
    .await?;

    tracing::info!(
        service_id = %enrolled.service_id,
        status = %enrolled.status,
        "enrollment response received"
    );

    identity
        .save_enrollment(
            enrolled.service_id,
            enrolled.enrollment_secret.expose_secret(),
        )
        .await?;
    tracing::info!("enrollment state persisted");

    // Wait for approval (may come immediately if auto-approved via token).
    // The returned Uuid is the same service_id we enrolled with — no rebind
    // needed on the fresh-enrollment path. Consumed via plain `?` with no
    // binding: `Cargo.toml` pins `let_underscore_must_use = "deny"` so
    // `let _approved_id = …` would be rejected, and `Uuid` is not
    // `#[must_use]` so the `?` expression discarding it is lint-clean.
    if enrolled.status != EnrollmentStatus::Approved {
        wait_for_approval(&mut ws, &mut in_seq, signals).await?;
    }

    // Generate keypair + CSR, request certificate
    tracing::debug!("generating CSR for enrollment");
    identity.ensure_keypair().await?;
    let csr_pem = identity.generate_csr_for_self()?;

    let cert =
        request_certificate_ws(&mut ws, &mut out_seq, &mut in_seq, &csr_pem, signals).await?;
    tracing::info!(not_after = %cert.not_after, "received client certificate");

    identity.save_certificate(&cert.cert_pem).await?;
    tracing::info!("service certificate saved");

    Ok(())
}

/// Resume enrollment for a service that already has a service_id and enrollment secret.
///
/// Reconnects with Bearer auth, waits for approval, generates CSR, and requests certificate.
/// If the controller approves with a different `service_id` (merge redirect), the stored
/// identity is rebound to the new id before the CSR is generated.
pub(crate) async fn resume_enrollment(
    identity: &mut crate::identity::ServiceIdentityState,
    host: &str,
    port: u16,
    tls_connector: &TlsConnector,
    signals: &mut SignalWatcher,
) -> Result<()> {
    tracing::debug!("resuming enrollment with stored credentials");
    let enrollment_secret = identity
        .enrollment_secret()
        .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotEnrolled)))?
        .to_string();

    // Include service_id as a query parameter so the controller can narrow
    // the bearer-secret lookup to this specific service (defence-in-depth).
    let service_id = identity.service_id();
    let auth_header = format!("Bearer {enrollment_secret}");
    let mut ws = connect_ws(
        host,
        port,
        tls_connector,
        Some(&auth_header),
        service_id,
        signals,
    )
    .await?;
    let mut out_seq = OutgoingSeq::new();
    let mut in_seq = IncomingSeq::new();

    resume_enrollment_inner(identity, &mut ws, &mut out_seq, &mut in_seq, signals).await
}

/// Inner enrollment-resume logic operating on an already-handshaken WebSocket stream.
///
/// Extracted to allow unit tests to drive the approval + rebind + cert-request flow
/// over an in-process duplex pair without needing a TLS connection.
///
/// # Errors
///
/// Returns an error if the controller sends a nil `service_id`, if the connection
/// closes unexpectedly, or if any downstream I/O or protocol error occurs.
pub(crate) async fn resume_enrollment_inner<S>(
    identity: &mut crate::identity::ServiceIdentityState,
    ws: &mut WebSocketStream<S>,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    signals: &mut SignalWatcher,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Wait for approval (controller pushes immediately if already approved).
    let approved_id = wait_for_approval(ws, in_seq, signals).await?;
    if approved_id.is_nil() {
        bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(
            "controller sent nil service_id in Approved".to_string()
        )));
    }
    if Some(approved_id) != identity.service_id() {
        let old_id = identity.service_id();
        let secret = identity
            .enrollment_secret()
            .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotEnrolled)))?
            .to_string();
        identity.save_enrollment(approved_id, &secret).await?;
        tracing::info!(
            ?old_id,
            new_id = %approved_id,
            "service identity rebound via merge redirect"
        );
    }

    // Generate keypair + CSR, request certificate
    identity.ensure_keypair().await?;
    let csr_pem = identity.generate_csr_for_self()?;

    let cert = request_certificate_ws(ws, out_seq, in_seq, &csr_pem, signals).await?;
    tracing::info!(not_after = %cert.not_after, "received client certificate");

    identity.save_certificate(&cert.cert_pem).await?;
    tracing::info!("service certificate saved to disk");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use tokio_tungstenite::tungstenite::Error as WsErr;
    use tokio_tungstenite::tungstenite::error::ProtocolError;

    // ── WS-level enrollment tests ─────────────────────────────────────────────

    /// Verify that `wait_for_approval` returns the `service_id` from the
    /// controller's `Approved` payload.
    #[tokio::test(start_paused = true)]
    async fn wait_for_approval_returns_payload_service_id() {
        use crate::test_support::serve_mock_controller;
        use tokio_tungstenite::tungstenite::Message;
        use uuid::Uuid;

        let target_id = Uuid::now_v7();
        // Wire format: ControllerEnvelope flattens ControllerMessage fields.
        // ControllerMessage uses serde(tag = "type", rename_all = "snake_case"),
        // so Approved serialises as `"type": "approved"` at the top level.
        let envelope = serde_json::json!({
            "protocol_version": crate::wire_api::CURRENT_PROTOCOL_VERSION,
            "seq": 1,
            "type": "approved",
            "service_id": target_id,
        });

        let mut ws = serve_mock_controller(vec![Message::Text(envelope.to_string().into())]).await;
        let mut in_seq = IncomingSeq::new();
        let mut signals = crate::signal::SignalWatcher::new().expect("signal watcher");

        let approved = wait_for_approval(&mut ws, &mut in_seq, &mut signals)
            .await
            .expect("approval received");

        assert_eq!(approved, target_id);
    }

    /// When the controller's `Approved.service_id` differs from the stored
    /// identity, `resume_enrollment_inner` must rebind and persist the new id.
    #[tokio::test(start_paused = true)]
    async fn resume_enrollment_rebinds_identity_on_id_mismatch() {
        use crate::identity::ServiceIdentityState;
        use crate::test_support::{mock_certificate_envelope, serve_mock_controller};
        use tokio_tungstenite::tungstenite::Message;
        use uuid::Uuid;

        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        let source_id = Uuid::now_v7();
        let target_id = Uuid::now_v7();
        identity
            .save_enrollment(source_id, "stored-secret")
            .await
            .expect("save");

        let approved_envelope = serde_json::json!({
            "protocol_version": crate::wire_api::CURRENT_PROTOCOL_VERSION,
            "seq": 1,
            "type": "approved",
            "service_id": target_id,
        });
        let cert_envelope = mock_certificate_envelope(2);

        let mut ws = serve_mock_controller(vec![
            Message::Text(approved_envelope.to_string().into()),
            Message::Text(cert_envelope.to_string().into()),
        ])
        .await;
        let mut out_seq = OutgoingSeq::new();
        let mut in_seq = IncomingSeq::new();
        let mut signals = crate::signal::SignalWatcher::new().expect("signal watcher");

        resume_enrollment_inner(
            &mut identity,
            &mut ws,
            &mut out_seq,
            &mut in_seq,
            &mut signals,
        )
        .await
        .expect("resume_enrollment_inner succeeds");

        // Reload from disk and confirm identity is now bound to target_id.
        let mut reloaded = ServiceIdentityState::new_single_dir(tmp.path());
        reloaded.load().await.expect("reload");
        assert_eq!(
            reloaded.service_id(),
            Some(target_id),
            "service_id must be rebound to the controller-approved target_id"
        );
    }

    /// When `Approved.service_id` matches the stored id, no rebind occurs and
    /// the in-memory id is preserved after the call.
    #[tokio::test(start_paused = true)]
    async fn resume_enrollment_noop_when_ids_match() {
        use crate::identity::ServiceIdentityState;
        use crate::test_support::{mock_certificate_envelope, serve_mock_controller};
        use tokio_tungstenite::tungstenite::Message;
        use uuid::Uuid;

        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        let service_id = Uuid::now_v7();
        identity
            .save_enrollment(service_id, "stored-secret")
            .await
            .expect("save");

        let approved_envelope = serde_json::json!({
            "protocol_version": crate::wire_api::CURRENT_PROTOCOL_VERSION,
            "seq": 1,
            "type": "approved",
            "service_id": service_id,
        });
        let cert_envelope = mock_certificate_envelope(2);

        let mut ws = serve_mock_controller(vec![
            Message::Text(approved_envelope.to_string().into()),
            Message::Text(cert_envelope.to_string().into()),
        ])
        .await;
        let mut out_seq = OutgoingSeq::new();
        let mut in_seq = IncomingSeq::new();
        let mut signals = crate::signal::SignalWatcher::new().expect("signal watcher");

        resume_enrollment_inner(
            &mut identity,
            &mut ws,
            &mut out_seq,
            &mut in_seq,
            &mut signals,
        )
        .await
        .expect("resume ok");

        // Identity must still carry the original id — no rebind occurred.
        assert_eq!(
            identity.service_id(),
            Some(service_id),
            "service_id must not change when Approved.service_id matches the stored id"
        );
    }

    /// A nil `service_id` in the `Approved` message must be refused with a
    /// typed `Protocol(Enrollment(…))` error.
    #[tokio::test(start_paused = true)]
    async fn resume_enrollment_rejects_nil_service_id() {
        use crate::identity::ServiceIdentityState;
        use crate::test_support::serve_mock_controller;
        use tokio_tungstenite::tungstenite::Message;
        use uuid::Uuid;

        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        identity
            .save_enrollment(Uuid::now_v7(), "secret")
            .await
            .expect("save");

        let approved_envelope = serde_json::json!({
            "protocol_version": crate::wire_api::CURRENT_PROTOCOL_VERSION,
            "seq": 1,
            "type": "approved",
            "service_id": Uuid::nil(),
        });

        let mut ws =
            serve_mock_controller(vec![Message::Text(approved_envelope.to_string().into())]).await;
        let mut out_seq = OutgoingSeq::new();
        let mut in_seq = IncomingSeq::new();
        let mut signals = crate::signal::SignalWatcher::new().expect("signal watcher");

        let err = resume_enrollment_inner(
            &mut identity,
            &mut ws,
            &mut out_seq,
            &mut in_seq,
            &mut signals,
        )
        .await
        .unwrap_err();

        match err.current_context() {
            EnrollmentError::Protocol(crate::error::ProtocolError::Enrollment(msg)) => {
                assert!(
                    msg.contains("nil service_id"),
                    "expected 'nil service_id' in error message, got: {msg:?}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn is_peer_closed_unexpected_eof() {
        let err = WsErr::Io(std::io::Error::new(ErrorKind::UnexpectedEof, "eof"));
        assert!(is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_broken_pipe() {
        let err = WsErr::Io(std::io::Error::new(ErrorKind::BrokenPipe, "broken pipe"));
        assert!(is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_connection_reset() {
        let err = WsErr::Io(std::io::Error::new(
            ErrorKind::ConnectionReset,
            "connection reset",
        ));
        assert!(is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_connection_aborted() {
        let err = WsErr::Io(std::io::Error::new(
            ErrorKind::ConnectionAborted,
            "connection aborted",
        ));
        assert!(is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_not_connected() {
        let err = WsErr::Io(std::io::Error::new(
            ErrorKind::NotConnected,
            "not connected",
        ));
        assert!(is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_reset_without_closing_handshake() {
        let err = WsErr::Protocol(ProtocolError::ResetWithoutClosingHandshake);
        assert!(is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_send_after_closing() {
        let err = WsErr::Protocol(ProtocolError::SendAfterClosing);
        assert!(is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_other_io_error() {
        let err = WsErr::Io(std::io::Error::new(ErrorKind::PermissionDenied, "denied"));
        assert!(!is_peer_closed(&err));
    }

    #[test]
    fn is_peer_closed_connection_closed() {
        let err = WsErr::ConnectionClosed;
        assert!(!is_peer_closed(&err));
    }

    // ── Signal-cancellation tests ──────────────────────────────────────
    //
    // These tests exercise the `tokio::select! { biased; signal = signals.recv(), .. }`
    // pattern used at every blocking `ws.next().await` site in this module
    // (`send_enroll`, `wait_for_approval`, `request_certificate_ws`). They verify
    // that SIGINT delivery interrupts a future that would otherwise block
    // indefinitely and that the resulting error classifies as `Cancelled` (not
    // transient, not receive-closed) so the lifecycle backoff loop exits
    // cleanly via the `is_cancelled_report` short-circuit.
    //
    // The tests target the select arm directly rather than driving a full WS
    // handshake because constructing a `WsStream` (TLS over TCP) in-process
    // requires a full rustls server config. End-to-end behaviour against a
    // real controller is verified manually per the implementation plan.
    //
    // `start_paused = false` is required: real OS signal delivery does not
    // proceed under tokio's paused clock.
    #[cfg(unix)]
    mod signal_cancellation {
        use crate::error::EnrollmentError;
        use crate::signal::{Signal, SignalWatcher, UNIX_SIGNAL_TEST_SEM};
        use nix::sys::signal::{self as nix_signal, Signal as NixSignal};
        use nix::unistd::getpid;
        use rootcause::prelude::*;
        use std::time::{Duration, Instant};

        /// Replica of the select-arm pattern used at every blocking
        /// `ws.next().await` site. Returns `Cancelled(signal)` when a signal
        /// arrives; otherwise blocks forever on the inner future.
        async fn cancellable_pending(
            signals: &mut SignalWatcher,
        ) -> std::result::Result<(), Report<EnrollmentError>> {
            tokio::select! {
                biased;
                signal = signals.recv() => {
                    bail!(EnrollmentError::Cancelled(signal));
                }
                () = std::future::pending::<()>() => unreachable!(),
            }
        }

        #[tokio::test(flavor = "current_thread", start_paused = false)]
        async fn sigint_short_circuits_blocking_select_arm() {
            let _permit = UNIX_SIGNAL_TEST_SEM
                .acquire()
                .await
                .expect("semaphore not closed");
            let mut signals = SignalWatcher::new().expect("signal watcher");

            // Use a oneshot to synchronize: the spawned task signals it has
            // entered the select before the test delivers SIGINT.
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

            let task = tokio::spawn(async move {
                ready_tx.send(()).expect("ready channel still open");
                let start = Instant::now();
                let result = cancellable_pending(&mut signals).await;
                (start.elapsed(), result)
            });

            ready_rx.await.expect("task reached select arm");
            // Yield once so the task actually polls the select before we
            // deliver SIGINT — otherwise the signal could be coalesced into
            // a prior `recv()` poll boundary.
            tokio::task::yield_now().await;

            nix_signal::kill(getpid(), NixSignal::SIGINT).expect("kill self with SIGINT");

            let (elapsed, result) = tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .expect("task must return within 2 s")
                .expect("task panicked");

            // Latency: the new select arm must short-circuit far below any
            // realistic wait timeout (the production `APPROVAL_TIMEOUT` is
            // 30 min and `RESPONSE_TIMEOUT` is 60 s).
            assert!(
                elapsed < Duration::from_millis(500),
                "select arm did not short-circuit: {elapsed:?}"
            );

            let err = result.expect_err("must return Cancelled");
            assert!(
                matches!(
                    err.current_context(),
                    EnrollmentError::Cancelled(Signal::Interrupt)
                ),
                "expected Cancelled(Interrupt), got: {:?}",
                err.current_context()
            );

            // Lifecycle backoff guard: a cancelled report must NOT be
            // classified as transient or receive-closed. (The full unit test
            // for the classifier matrix lives in `error.rs`.)
            assert!(!err.current_context().is_transient_network());
            assert!(!err.current_context().is_receive_closed());
        }
    }
}
