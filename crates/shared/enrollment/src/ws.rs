//! WebSocket connection and enrollment protocol.
//!
//! Provides shared WebSocket helpers for the enrollment flow used by both
//! agents and MQTT services:
//! - Manual TCP → TLS → WebSocket upgrade (using `tokio-tungstenite::client_async`)
//! - `send_enroll` / `wait_for_approval` / `request_certificate_ws`
//! - Full `run_enrollment` and `resume_enrollment` orchestrators

use futures_util::{SinkExt, StreamExt};
use http::Uri;
use rootcause::prelude::*;
use rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uptrakit_internal_wire::{
    CertificatePayload, ControllerMessage, EnrollPayload, EnrolledPayload, EnrollmentStatus,
    HostInfo, RequestCertificatePayload, ServiceMessage, ServiceType,
};

use crate::error::{EnrollmentError, Result};

/// Type alias for the WebSocket stream produced by [`connect_ws`].
pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

/// Connect TCP → TLS → WebSocket upgrade, with optional Authorization header.
pub async fn connect_ws(
    host: &str,
    port: u16,
    tls_connector: &TlsConnector,
    auth_header: Option<&str>,
) -> Result<WsStream> {
    let ws_url = format!("wss://{host}:{port}/api/v1/ws/service");
    tracing::info!(url = %ws_url, "connecting to controller");

    let tcp_stream = tokio::net::TcpStream::connect((host, port))
        .await
        .context_to::<EnrollmentError>()?;

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

    if let Some(header_value) = auth_header {
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(header_value).map_err(|e| {
                report!(EnrollmentError::Enrollment(format!(
                    "invalid authorization header: {e}"
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
pub async fn send_enroll(
    ws: &mut WsStream,
    hostname: &str,
    friendly_name: &str,
    enrollment_token: Option<&str>,
    service_type: ServiceType,
    host_info: Option<HostInfo>,
) -> Result<EnrolledPayload> {
    let msg = ServiceMessage::Enroll(EnrollPayload {
        hostname: hostname.to_string(),
        friendly_name: friendly_name.to_string(),
        enrollment_token: enrollment_token.map(|s| s.to_string()),
        service_type,
        host_info,
    });
    let json = serde_json::to_string(&msg).context_to::<EnrollmentError>()?;
    ws.send(Message::Text(json.into()))
        .await
        .context_to::<EnrollmentError>()?;

    tracing::info!("sent Enroll, waiting for Enrolled response");

    loop {
        let resp = ws
            .next()
            .await
            .ok_or_else(|| report!(EnrollmentError::ReceiveClosed))?
            .context_to::<EnrollmentError>()?;

        match resp {
            Message::Text(text) => {
                let controller_msg: ControllerMessage =
                    serde_json::from_str(&text).context_to::<EnrollmentError>()?;

                match controller_msg {
                    ControllerMessage::Enrolled(payload) => return Ok(payload),
                    ControllerMessage::Error(err) => {
                        return Err(report!(EnrollmentError::Enrollment(format!(
                            "{}: {}",
                            err.code, err.message
                        ))));
                    }
                    ControllerMessage::ServerRestarting(payload) => {
                        tracing::info!(reason = %payload.reason, "controller is restarting during enrollment");
                        return Err(report!(EnrollmentError::ReceiveClosed));
                    }
                    _ => {
                        return Err(report!(EnrollmentError::UnexpectedMessage));
                    }
                }
            }
            Message::Close(frame) => {
                log_close_frame(frame);
                return Err(report!(EnrollmentError::ReceiveClosed));
            }
            _ => continue,
        }
    }
}

/// Wait for Approved/Rejected push from controller.
///
/// Returns `Ok(())` on `Approved`, errors on `Rejected`.
pub async fn wait_for_approval(ws: &mut WsStream) -> Result<()> {
    tracing::info!("waiting for approval...");

    loop {
        let msg = match ws.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) if is_peer_closed(&e) => {
                tracing::info!("connection closed by controller while waiting for approval");
                return Err(report!(EnrollmentError::ReceiveClosed));
            }
            Some(Err(e)) => return Err(e).context_to::<EnrollmentError>()?,
            None => return Err(report!(EnrollmentError::ReceiveClosed)),
        };

        match msg {
            Message::Text(text) => {
                let controller_msg: ControllerMessage =
                    serde_json::from_str(&text).context_to::<EnrollmentError>()?;

                match controller_msg {
                    ControllerMessage::Approved(payload) => {
                        tracing::info!(service_id = %payload.service_id, "enrollment approved");
                        return Ok(());
                    }
                    ControllerMessage::Rejected(payload) => {
                        tracing::error!(service_id = %payload.service_id, "enrollment rejected");
                        return Err(report!(EnrollmentError::EnrollmentRejected));
                    }
                    ControllerMessage::Pong(_) => continue,
                    ControllerMessage::Error(err) => {
                        return Err(report!(EnrollmentError::Enrollment(format!(
                            "{}: {}",
                            err.code, err.message
                        ))));
                    }
                    ControllerMessage::ServerRestarting(payload) => {
                        tracing::info!(reason = %payload.reason, "controller is restarting while waiting for approval");
                        return Err(report!(EnrollmentError::ReceiveClosed));
                    }
                    _ => continue,
                }
            }
            Message::Close(frame) => {
                log_close_frame(frame);
                return Err(report!(EnrollmentError::ReceiveClosed));
            }
            _ => continue,
        }
    }
}

/// Send `RequestCertificate` with a CSR and read `Certificate` response.
pub async fn request_certificate_ws(
    ws: &mut WsStream,
    csr_pem: &str,
) -> Result<CertificatePayload> {
    let msg = ServiceMessage::RequestCertificate(RequestCertificatePayload {
        csr_pem: csr_pem.to_string(),
    });
    let json = serde_json::to_string(&msg).context_to::<EnrollmentError>()?;
    ws.send(Message::Text(json.into()))
        .await
        .context_to::<EnrollmentError>()?;

    tracing::info!("sent RequestCertificate, waiting for Certificate response");

    loop {
        let resp = ws
            .next()
            .await
            .ok_or_else(|| report!(EnrollmentError::ReceiveClosed))?
            .context_to::<EnrollmentError>()?;

        match resp {
            Message::Text(text) => {
                let controller_msg: ControllerMessage =
                    serde_json::from_str(&text).context_to::<EnrollmentError>()?;

                match controller_msg {
                    ControllerMessage::Certificate(payload) => return Ok(payload),
                    ControllerMessage::Error(err) => {
                        return Err(report!(EnrollmentError::Enrollment(format!(
                            "{}: {}",
                            err.code, err.message
                        ))));
                    }
                    ControllerMessage::ServerRestarting(payload) => {
                        tracing::info!(reason = %payload.reason, "controller is restarting during certificate request");
                        return Err(report!(EnrollmentError::ReceiveClosed));
                    }
                    _ => {
                        return Err(report!(EnrollmentError::UnexpectedMessage));
                    }
                }
            }
            Message::Close(frame) => {
                log_close_frame(frame);
                return Err(report!(EnrollmentError::ReceiveClosed));
            }
            _ => continue,
        }
    }
}

/// Log a WebSocket close frame.
pub fn log_close_frame(frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>) {
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
/// when the controller terminates a connection (e.g. service deactivated).
pub fn is_peer_closed(err: &tokio_tungstenite::tungstenite::Error) -> bool {
    use tokio_tungstenite::tungstenite::Error as WsErr;
    use tokio_tungstenite::tungstenite::error::ProtocolError;
    match err {
        WsErr::Io(io) => io.kind() == std::io::ErrorKind::UnexpectedEof,
        WsErr::Protocol(
            ProtocolError::ResetWithoutClosingHandshake | ProtocolError::SendAfterClosing,
        ) => true,
        _ => false,
    }
}

/// Parameters for [`run_enrollment`].
pub struct EnrollmentParams<'a> {
    pub identity: &'a mut crate::identity::ServiceIdentityState,
    pub host: &'a str,
    pub port: u16,
    pub tls_connector: &'a TlsConnector,
    pub hostname: &'a str,
    pub friendly_name: &'a str,
    pub enrollment_token: Option<&'a str>,
    pub service_type: ServiceType,
    pub host_info: Option<HostInfo>,
}

/// Run a fresh enrollment flow: enroll → wait for approval → generate CSR → request certificate.
///
/// On success, the identity is fully certified (service_id, key, and cert persisted).
pub async fn run_enrollment(params: EnrollmentParams<'_>) -> Result<()> {
    let EnrollmentParams {
        identity,
        host,
        port,
        tls_connector,
        hostname,
        friendly_name,
        enrollment_token,
        service_type,
        host_info,
    } = params;
    let mut ws = connect_ws(host, port, tls_connector, None).await?;

    let enrolled = send_enroll(
        &mut ws,
        hostname,
        friendly_name,
        enrollment_token,
        service_type,
        host_info,
    )
    .await?;

    tracing::info!(
        service_id = %enrolled.service_id,
        status = %enrolled.status,
        "enrollment response received"
    );

    identity
        .save_enrollment(enrolled.service_id, &enrolled.enrollment_secret)
        .await?;
    tracing::info!("enrollment state persisted");

    // Wait for approval (may come immediately if auto-approved via token)
    if enrolled.status != EnrollmentStatus::Approved {
        wait_for_approval(&mut ws).await?;
    }

    // Generate keypair + CSR, request certificate
    identity.ensure_keypair().await?;
    let csr_pem = identity.generate_csr_for_self()?;

    let cert = request_certificate_ws(&mut ws, &csr_pem).await?;
    tracing::info!(not_after = %cert.not_after, "received client certificate");

    identity.save_certificate(&cert.cert_pem).await?;
    tracing::info!("service certificate saved to disk");

    Ok(())
}

/// Resume enrollment for a service that already has a service_id and enrollment secret.
///
/// Reconnects with Bearer auth, waits for approval, generates CSR, and requests certificate.
pub async fn resume_enrollment(
    identity: &mut crate::identity::ServiceIdentityState,
    host: &str,
    port: u16,
    tls_connector: &TlsConnector,
) -> Result<()> {
    let enrollment_secret = identity
        .enrollment_secret()
        .ok_or_else(|| report!(EnrollmentError::NotEnrolled))?
        .to_string();

    tracing::info!("reconnecting with enrollment secret");
    let auth_header = format!("Bearer {enrollment_secret}");
    let mut ws = connect_ws(host, port, tls_connector, Some(&auth_header)).await?;

    // Wait for approval (controller pushes immediately if already approved)
    wait_for_approval(&mut ws).await?;

    // Generate keypair + CSR, request certificate
    identity.ensure_keypair().await?;
    let csr_pem = identity.generate_csr_for_self()?;

    let cert = request_certificate_ws(&mut ws, &csr_pem).await?;
    tracing::info!(not_after = %cert.not_after, "received client certificate");

    identity.save_certificate(&cert.cert_pem).await?;
    tracing::info!("service certificate saved to disk");

    Ok(())
}
