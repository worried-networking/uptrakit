use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use http::Uri;
use rootcause::prelude::*;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName, pem::PemObject};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use sha2::{Digest, Sha256};
use uptrakit_internal_wire::{
    AgentMessage, CertificatePayload, ControllerMessage, EnrollPayload, EnrolledPayload,
    PingPayload, RenewCertificatePayload, RequestCertificatePayload, now_millis,
};

use crate::error::{Error, Result};

/// Outcome of the authenticated event loop.
pub enum LoopOutcome {
    /// SIGINT/SIGTERM received — shut down cleanly.
    Shutdown,
    /// Certificate rotated — reload from disk and reconnect.
    Reconnect,
    /// Connection closed by controller — no special action.
    Disconnected,
}

pub async fn fetch_ca_certificate(host: &str, http_port: u16) -> Result<Vec<u8>> {
    let url = format!("http://{host}:{http_port}/api/v1/ca.crt");
    tracing::info!(url = %url, "fetching CA certificate");

    let stream = tokio::net::TcpStream::connect((host, http_port))
        .await
        .context_to::<Error>()?;

    let request = format!(
        "GET /api/v1/ca.crt HTTP/1.1\r\nHost: {host}:{http_port}\r\nConnection: close\r\n\r\n"
    );

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = stream;
    stream
        .write_all(request.as_bytes())
        .await
        .context_to::<Error>()?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .context_to::<Error>()?;

    // Parse HTTP response - find body after \r\n\r\n
    let response_str = String::from_utf8_lossy(&response);
    let body_start = response_str
        .find("\r\n\r\n")
        .ok_or_else(|| report!(Error::FetchCaHttp("invalid HTTP response".to_string())))?
        + 4;

    let body = &response[body_start..];

    // Check for HTTP error status
    if !response_str.starts_with("HTTP/1.1 200") && !response_str.starts_with("HTTP/1.0 200") {
        let status_line = response_str.lines().next().unwrap_or("unknown");
        return Err(report!(Error::FetchCaHttp(format!(
            "HTTP error: {status_line}"
        ))));
    }

    tracing::info!(bytes = body.len(), "CA certificate fetched");
    Ok(body.to_vec())
}

pub fn build_tls_connector(ca_pem: &[u8]) -> Result<TlsConnector> {
    let root_store = build_root_store(ca_pem)?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

pub fn build_tls_connector_with_client_cert(
    ca_pem: &[u8],
    cert_pem: &str,
    key_pem: &str,
) -> Result<TlsConnector> {
    use rustls::pki_types::PrivateKeyDer;

    let root_store = build_root_store(ca_pem)?;

    let client_certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_to::<Error>()?;

    let client_key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).context_to::<Error>()?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(client_certs, client_key)
        .context_to::<Error>()?;

    Ok(TlsConnector::from(Arc::new(config)))
}

fn build_root_store(ca_pem: &[u8]) -> Result<RootCertStore> {
    let certs = CertificateDer::pem_slice_iter(ca_pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_to::<Error>()?;

    if certs.is_empty() {
        return Err(report!(Error::NoCertificates));
    }

    let mut root_store = RootCertStore::empty();
    for cert in certs {
        root_store.add(cert).context_to::<Error>()?;
    }

    Ok(root_store)
}

/// Type alias for the WebSocket stream produced by `connect_ws`.
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

/// Connect TCP → TLS → WebSocket upgrade, with optional Authorization header.
pub async fn connect_ws(
    host: &str,
    port: u16,
    tls_connector: &TlsConnector,
    auth_header: Option<&str>,
) -> Result<WsStream> {
    let ws_url = format!("wss://{host}:{port}/api/v1/ws/agent");
    tracing::info!(url = %ws_url, "connecting to controller");

    let tcp_stream = tokio::net::TcpStream::connect((host, port))
        .await
        .context_to::<Error>()?;

    let server_name = ServerName::try_from(host.to_string()).context_to::<Error>()?;

    let tls_stream = tls_connector
        .connect(server_name, tcp_stream)
        .await
        .context_to::<Error>()?;

    let uri: Uri = ws_url.parse().context_to::<Error>()?;
    let mut request = uri
        .to_string()
        .into_client_request()
        .context_to::<Error>()?;

    if let Some(header_value) = auth_header {
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(header_value).map_err(|e| {
                report!(Error::Enrollment(format!(
                    "invalid authorization header: {e}"
                )))
            })?,
        );
    }

    let (ws_stream, _response) = tokio_tungstenite::client_async(request, tls_stream)
        .await
        .context_to::<Error>()?;

    tracing::info!("WebSocket connected");
    Ok(ws_stream)
}

/// Send Enroll message and read Enrolled response.
pub async fn send_enroll(
    ws: &mut WsStream,
    hostname: &str,
    friendly_name: &str,
    enrollment_token: Option<&str>,
) -> Result<EnrolledPayload> {
    let msg = AgentMessage::Enroll(EnrollPayload {
        hostname: hostname.to_string(),
        friendly_name: friendly_name.to_string(),
        enrollment_token: enrollment_token.map(|s| s.to_string()),
    });
    let json = serde_json::to_string(&msg).context_to::<Error>()?;
    ws.send(Message::Text(json.into()))
        .await
        .context_to::<Error>()?;

    tracing::info!("sent Enroll, waiting for Enrolled response");

    loop {
        let resp = ws
            .next()
            .await
            .ok_or_else(|| report!(Error::ReceiveClosed))?
            .context_to::<Error>()?;

        match resp {
            Message::Text(text) => {
                let controller_msg: ControllerMessage =
                    serde_json::from_str(&text).context_to::<Error>()?;

                match controller_msg {
                    ControllerMessage::Enrolled(payload) => return Ok(payload),
                    ControllerMessage::Error(err) => {
                        return Err(report!(Error::Enrollment(format!(
                            "{}: {}",
                            err.code, err.message
                        ))));
                    }
                    _ => {
                        return Err(report!(Error::UnexpectedMessage));
                    }
                }
            }
            Message::Close(frame) => {
                log_close_frame(frame);
                return Err(report!(Error::ReceiveClosed));
            }
            _ => continue,
        }
    }
}

/// Wait for Approved/Rejected push from controller. Returns on Approved, errors on Rejected.
pub async fn wait_for_approval(ws: &mut WsStream) -> Result<()> {
    tracing::info!("waiting for approval...");

    loop {
        let msg = match ws.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) if is_peer_closed(&e) => {
                tracing::info!("connection closed by controller while waiting for approval");
                return Err(report!(Error::ReceiveClosed));
            }
            Some(Err(e)) => return Err(e).context_to::<Error>()?,
            None => return Err(report!(Error::ReceiveClosed)),
        };

        match msg {
            Message::Text(text) => {
                let controller_msg: ControllerMessage =
                    serde_json::from_str(&text).context_to::<Error>()?;

                match controller_msg {
                    ControllerMessage::Approved(payload) => {
                        tracing::info!(agent_id = %payload.agent_id, "enrollment approved");
                        return Ok(());
                    }
                    ControllerMessage::Rejected(payload) => {
                        tracing::error!(agent_id = %payload.agent_id, "enrollment rejected");
                        return Err(report!(Error::EnrollmentRejected));
                    }
                    ControllerMessage::Pong(_) => {
                        // Ignore pongs while waiting
                        continue;
                    }
                    ControllerMessage::Error(err) => {
                        return Err(report!(Error::Enrollment(format!(
                            "{}: {}",
                            err.code, err.message
                        ))));
                    }
                    _ => continue,
                }
            }
            Message::Close(frame) => {
                log_close_frame(frame);
                return Err(report!(Error::ReceiveClosed));
            }
            _ => continue,
        }
    }
}

/// Send RequestCertificate and read Certificate response.
pub async fn request_certificate_ws(ws: &mut WsStream) -> Result<CertificatePayload> {
    let msg = AgentMessage::RequestCertificate(RequestCertificatePayload {});
    let json = serde_json::to_string(&msg).context_to::<Error>()?;
    ws.send(Message::Text(json.into()))
        .await
        .context_to::<Error>()?;

    tracing::info!("sent RequestCertificate, waiting for Certificate response");

    loop {
        let resp = ws
            .next()
            .await
            .ok_or_else(|| report!(Error::ReceiveClosed))?
            .context_to::<Error>()?;

        match resp {
            Message::Text(text) => {
                let controller_msg: ControllerMessage =
                    serde_json::from_str(&text).context_to::<Error>()?;

                match controller_msg {
                    ControllerMessage::Certificate(payload) => return Ok(payload),
                    ControllerMessage::Error(err) => {
                        return Err(report!(Error::Enrollment(format!(
                            "{}: {}",
                            err.code, err.message
                        ))));
                    }
                    _ => {
                        return Err(report!(Error::UnexpectedMessage));
                    }
                }
            }
            Message::Close(frame) => {
                log_close_frame(frame);
                return Err(report!(Error::ReceiveClosed));
            }
            _ => continue,
        }
    }
}

/// Far-future delay used when no renewal is scheduled (30 days).
const FAR_FUTURE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

/// Compute how long until the renewal window opens.
fn compute_renewal_delay(cert_not_after_ts: Option<i64>, window_hours: u16) -> std::time::Duration {
    match cert_not_after_ts {
        Some(not_after) => {
            let renew_at = not_after - i64::from(window_hours) * 3600 * 1000;
            let delay_ms = (renew_at - now_millis()).max(0) as u64;
            std::time::Duration::from_millis(delay_ms)
        }
        None => FAR_FUTURE,
    }
}

/// Authenticated Ping/Pong event loop (mTLS connection) with renewal timer.
pub async fn run_authenticated_loop(
    host: &str,
    port: u16,
    tls_connector: TlsConnector,
    cert_not_after_ts: Option<i64>,
    data_dir: &std::path::Path,
) -> Result<LoopOutcome> {
    use std::pin::Pin;
    use std::time::Duration;

    const PING_INTERVAL: Duration = Duration::from_secs(300);

    let mut ws_stream = connect_ws(host, port, &tls_connector, None).await?;

    let mut shutdown = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context_to::<Error>()?;

    // First tick completes immediately, sending an initial ping on connect
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await;

    // Renewal timer — initially far-future, reset when AgentSettings arrives
    let mut renewal_sleep: Pin<Box<tokio::time::Sleep>> = Box::pin(tokio::time::sleep(FAR_FUTURE));

    // Every `break` arm below assigns `outcome` before exiting; the
    // initial value is a safety fallback only.
    #[allow(unused_assignments)]
    let mut outcome = LoopOutcome::Shutdown;

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                let agent_ts = now_millis();
                let ping = AgentMessage::Ping(PingPayload { agent_ts });
                let ping_json = serde_json::to_string(&ping).context_to::<Error>()?;

                tracing::trace!(agent_ts, "sending ping");
                ws_stream
                    .send(Message::Text(ping_json.into()))
                    .await
                    .context_to::<Error>()?;
            }
            _ = &mut renewal_sleep => {
                tracing::info!("renewal window reached, requesting certificate renewal");
                let msg = AgentMessage::RenewCertificate(RenewCertificatePayload {});
                let json = serde_json::to_string(&msg).context_to::<Error>()?;
                ws_stream.send(Message::Text(json.into())).await.context_to::<Error>()?;
                // Reset to far-future so it doesn't fire again
                renewal_sleep.as_mut().reset(
                    tokio::time::Instant::now() + FAR_FUTURE
                );
            }
            msg = ws_stream.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) if is_peer_closed(&e) => {
                        tracing::info!("connection closed by controller");
                        outcome = LoopOutcome::Disconnected;
                        break;
                    }
                    Some(Err(e)) => return Err(e).context_to::<Error>()?,
                    None => {
                        tracing::info!("connection closed by controller");
                        outcome = LoopOutcome::Disconnected;
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => {
                        let controller_msg: ControllerMessage = match serde_json::from_str(&text) {
                            Ok(msg) => msg,
                            Err(e) => {
                                tracing::debug!("ignoring unrecognized controller message: {e}");
                                continue;
                            }
                        };

                        match controller_msg {
                            ControllerMessage::Pong(pong) => {
                                let now = now_millis();
                                let rtt = now - pong.agent_ts;
                                tracing::trace!(
                                    agent_ts = pong.agent_ts,
                                    controller_ts = pong.controller_ts,
                                    rtt_ms = rtt,
                                    "received pong"
                                );
                            }
                            ControllerMessage::Certificate(payload) => {
                                // Save new cert to disk
                                let cert_state = crate::state::AgentCertState {
                                    cert_pem: payload.cert_pem,
                                    key_pem: payload.key_pem,
                                };
                                cert_state.save(data_dir)?;
                                let not_after_ms = payload.not_after.unix_timestamp() * 1000
                                    + i64::from(payload.not_after.millisecond());
                                crate::state::save_cert_not_after_ts(data_dir, not_after_ms)?;
                                tracing::info!("renewed certificate saved, reconnecting");
                                outcome = LoopOutcome::Reconnect;
                                break;
                            }
                            ControllerMessage::AgentSettings(settings) => {
                                tracing::trace!(
                                    renewal_window_hours = settings.renewal_window_hours,
                                    "received agent settings"
                                );
                                renewal_sleep.as_mut().reset(
                                    tokio::time::Instant::now()
                                        + compute_renewal_delay(
                                            cert_not_after_ts,
                                            settings.renewal_window_hours,
                                        ),
                                );

                                // Check if CA bundle is stale
                                if !settings.ca_bundle_hash.is_empty() {
                                    let local_hash = compute_local_ca_hash(data_dir);
                                    if local_hash != settings.ca_bundle_hash {
                                        tracing::info!("CA bundle hash mismatch, fetching updated bundle");
                                        match fetch_ca_certificate(host, port).await {
                                            Ok(pem) => {
                                                if let Err(e) = crate::state::save_ca_cert(data_dir, &pem) {
                                                    tracing::warn!("failed to save updated CA: {e}");
                                                } else {
                                                    tracing::info!("updated CA bundle saved to disk");
                                                }
                                            }
                                            Err(e) => tracing::warn!("failed to fetch updated CA: {e}"),
                                        }
                                    }
                                }
                            }
                            ControllerMessage::CaBundleUpdated(payload) => {
                                tracing::info!("received CA bundle update from controller");
                                if let Err(e) = crate::state::save_ca_cert(data_dir, payload.ca_bundle_pem.as_bytes()) {
                                    tracing::warn!("failed to save updated CA bundle: {e}");
                                } else {
                                    tracing::info!("updated CA bundle saved to disk");
                                }
                            }
                            _ => {
                                tracing::debug!("ignoring unrecognized message in authenticated loop");
                                continue;
                            }
                        }
                    }
                    Message::Close(frame) => {
                        let reason = frame.as_ref().map(|f| f.reason.as_ref()).unwrap_or("");
                        if reason == "certificate rotated" {
                            tracing::info!("connection closed: certificate rotated");
                            outcome = LoopOutcome::Reconnect;
                        } else if reason == "certificate revoked" {
                            tracing::warn!("connection closed: certificate revoked");
                            outcome = LoopOutcome::Disconnected;
                        } else {
                            log_close_frame(frame);
                            outcome = LoopOutcome::Disconnected;
                        }
                        break;
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received SIGINT, shutting down");
                outcome = LoopOutcome::Shutdown;
                break;
            }
            _ = shutdown.recv() => {
                tracing::info!("received SIGTERM, shutting down");
                outcome = LoopOutcome::Shutdown;
                break;
            }
        }
    }

    // Best-effort close — the peer may have already disconnected.
    match ws_stream.close(None).await {
        Ok(()) => tracing::info!("websocket closed gracefully"),
        Err(e) if is_peer_closed(&e) => tracing::info!("websocket already closed by peer"),
        Err(e) => return Err(e).context_to::<Error>()?,
    }

    Ok(outcome)
}

/// Compute SHA-256 hex hash of the local CA certificate file.
fn compute_local_ca_hash(data_dir: &std::path::Path) -> String {
    match crate::state::load_ca_cert(data_dir) {
        Ok(Some(bytes)) => {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        }
        _ => String::new(),
    }
}

fn log_close_frame(frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>) {
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
/// connection without sending a TLS `close_notify`.  This is normal
/// when the controller terminates a connection (e.g. agent deactivated).
fn is_peer_closed(err: &tokio_tungstenite::tungstenite::Error) -> bool {
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
