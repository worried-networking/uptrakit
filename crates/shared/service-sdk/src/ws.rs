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
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

use crate::wire_api::{
    CURRENT_PROTOCOL_VERSION, Capability, CertificatePayload, ControllerEnvelope,
    ControllerMessage, EnrollPayload, EnrolledPayload, EnrollmentStatus, IncomingSeq, OutgoingSeq,
    RequestCertificatePayload, SecretString, ServiceMessage,
};

use crate::error::{EnrollmentError, IdentityError, ProtocolError, Result};

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

    let tls_stream = tls_connector
        .connect(server_name, tcp_stream)
        .await
        .context_to::<EnrollmentError>()?;

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

    let (ws_stream, _response) = tokio_tungstenite::client_async(request, tls_stream)
        .await
        .context_to::<EnrollmentError>()?;

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
            let resp = ws
                .next()
                .await
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
/// Returns `Ok(())` on `Approved`, errors on `Rejected`.
/// Times out after [`APPROVAL_TIMEOUT`] (30 minutes).
#[expect(
    clippy::map_err_ignore,
    reason = "tokio::time::error::Elapsed carries no additional context beyond the timeout itself"
)]
pub(crate) async fn wait_for_approval(ws: &mut WsStream, in_seq: &mut IncomingSeq) -> Result<()> {
    tracing::info!("waiting for approval...");

    tokio::time::timeout(APPROVAL_TIMEOUT, async {
        loop {
            let msg = match ws.next().await {
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
                            return Ok(());
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
#[expect(
    clippy::map_err_ignore,
    reason = "tokio::time::error::Elapsed carries no additional context beyond the timeout itself"
)]
pub(crate) async fn request_certificate_ws(
    ws: &mut WsStream,
    out_seq: &mut OutgoingSeq,
    in_seq: &mut IncomingSeq,
    csr_pem: &str,
) -> Result<CertificatePayload> {
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
            let resp = ws
                .next()
                .await
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
    } = params;
    // Fresh enrollment: no service_id yet, so no query parameter.
    let mut ws = connect_ws(host, port, tls_connector, None, None).await?;
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

    // Wait for approval (may come immediately if auto-approved via token)
    if enrolled.status != EnrollmentStatus::Approved {
        wait_for_approval(&mut ws, &mut in_seq).await?;
    }

    // Generate keypair + CSR, request certificate
    tracing::debug!("generating CSR for enrollment");
    identity.ensure_keypair().await?;
    let csr_pem = identity.generate_csr_for_self()?;

    let cert = request_certificate_ws(&mut ws, &mut out_seq, &mut in_seq, &csr_pem).await?;
    tracing::info!(not_after = %cert.not_after, "received client certificate");

    identity.save_certificate(&cert.cert_pem).await?;
    tracing::info!("service certificate saved");

    Ok(())
}

/// Resume enrollment for a service that already has a service_id and enrollment secret.
///
/// Reconnects with Bearer auth, waits for approval, generates CSR, and requests certificate.
pub(crate) async fn resume_enrollment(
    identity: &mut crate::identity::ServiceIdentityState,
    host: &str,
    port: u16,
    tls_connector: &TlsConnector,
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
    let mut ws = connect_ws(host, port, tls_connector, Some(&auth_header), service_id).await?;
    let mut out_seq = OutgoingSeq::new();
    let mut in_seq = IncomingSeq::new();

    // Wait for approval (controller pushes immediately if already approved)
    wait_for_approval(&mut ws, &mut in_seq).await?;

    // Generate keypair + CSR, request certificate
    identity.ensure_keypair().await?;
    let csr_pem = identity.generate_csr_for_self()?;

    let cert = request_certificate_ws(&mut ws, &mut out_seq, &mut in_seq, &csr_pem).await?;
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
}
