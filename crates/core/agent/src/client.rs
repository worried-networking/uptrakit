use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use http::Uri;
use rcgen::{CertificateParams, DnType, KeyPair};
use rootcause::prelude::*;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName, pem::PemObject};
use sha2::{Digest, Sha256};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uptrakit_internal_wire::{
    CertificatePayload, ControllerMessage, DisconnectReason, DisconnectingPayload, EnrollPayload,
    EnrolledPayload, HostInfo, PingPayload, RenewCertificatePayload, ReportHostInfoPayload,
    RequestCertificatePayload, ServiceMessage, UpdateOutputPayload, UpdateResultPayload,
    UpdateStartedPayload, VersionCheckResult, VersionCheckResultsPayload, now_millis,
};

use crate::error::{Error, Result};

/// Generate an ECDSA P-256 keypair and a CSR with CN=agent_id.
/// Returns `(key_pem, csr_pem)`.
pub fn generate_keypair_and_csr(agent_id: &str) -> Result<(String, String)> {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| report!(Error::CsrGeneration(format!("key generation failed: {e}"))))?;

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, agent_id.to_string());
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Uptrakit Agent");

    let csr = params.serialize_request(&key_pair).map_err(|e| {
        report!(Error::CsrGeneration(format!(
            "CSR serialization failed: {e}"
        )))
    })?;

    let csr_pem = csr.pem().map_err(|e| {
        report!(Error::CsrGeneration(format!(
            "CSR PEM encoding failed: {e}"
        )))
    })?;

    Ok((key_pair.serialize_pem(), csr_pem))
}

/// Outcome of the authenticated event loop.
pub enum LoopOutcome {
    /// SIGINT/SIGTERM received — shut down cleanly.
    Shutdown,
    /// Certificate rotated — reload from disk and reconnect.
    Reconnect,
    /// Connection closed by controller — no special action.
    Disconnected,
    /// SIGHUP received — exit for external restart.
    Restart,
}

/// TLS mode for the CA certificate fetch via reqwest.
pub enum TlsMode<'a> {
    /// Use system/built-in root certificates (for https:// pki_addr).
    SystemTrust,
    /// Accept any server cert (TOFU).
    TrustOnFirstUse,
    /// Use a pinned CA certificate.
    PinnedCa(&'a [u8]),
}

/// Fetch the CA certificate bundle using reqwest.
///
/// The caller passes the correct `base_url` (either the main controller URL
/// or the `--pki-addr` value). If `base_url` starts with `http://`, plain
/// HTTP is used (no TLS configuration needed). Otherwise the provided
/// `tls_mode` applies.
pub async fn fetch_ca_certificate(base_url: &str, tls_mode: TlsMode<'_>) -> Result<Vec<u8>> {
    let fetch_url = format!("{base_url}/api/v1/pki/ca.crt");
    let use_plain_http = base_url.starts_with("http://");

    tracing::info!(url = %fetch_url, "fetching CA certificate");

    let mut builder = reqwest::Client::builder();
    if use_plain_http {
        // Plain HTTP — no TLS configuration needed
    } else {
        match tls_mode {
            TlsMode::SystemTrust => {
                // reqwest defaults to system/built-in roots — nothing to configure
            }
            TlsMode::TrustOnFirstUse => {
                builder = builder.tls_danger_accept_invalid_certs(true);
            }
            TlsMode::PinnedCa(ca_pem) => {
                let cert = reqwest::Certificate::from_pem(ca_pem)
                    .map_err(|e| report!(Error::FetchCa(format!("invalid CA PEM: {e}"))))?;
                builder = builder.tls_certs_only([cert]);
            }
        }
    }

    let client = builder
        .build()
        .map_err(|e| report!(Error::FetchCa(e.to_string())))?;

    let resp = client
        .get(&fetch_url)
        .send()
        .await
        .map_err(|e| report!(Error::FetchCa(e.to_string())))?;

    if !resp.status().is_success() {
        return Err(report!(Error::FetchCa(format!("HTTP {}", resp.status()))));
    }

    let body = resp
        .bytes()
        .await
        .map_err(|e| report!(Error::FetchCa(e.to_string())))?;

    tracing::info!(bytes = body.len(), "CA certificate fetched");
    Ok(body.to_vec())
}

/// Build a TLS connector that trusts only the given CA PEM (no client auth).
pub fn build_tls_connector(ca_pem: &[u8]) -> Result<TlsConnector> {
    let root_store = build_root_store(ca_pem)?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

/// Build a TLS connector that trusts only the given CA PEM, with client cert (mTLS).
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

/// Build a TLS connector using system/webpki root certificates (no client auth).
pub fn build_system_trust_tls_connector() -> Result<TlsConnector> {
    let root_store = build_webpki_root_store();

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

/// Build a TLS connector using system/webpki root certs with client cert (mTLS).
pub fn build_system_trust_tls_connector_with_client_cert(
    cert_pem: &str,
    key_pem: &str,
) -> Result<TlsConnector> {
    use rustls::pki_types::PrivateKeyDer;

    let root_store = build_webpki_root_store();

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

fn build_webpki_root_store() -> RootCertStore {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    root_store
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
    let ws_url = format!("wss://{host}:{port}/api/v1/ws/service");
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
    host_info: HostInfo,
) -> Result<EnrolledPayload> {
    let msg = ServiceMessage::Enroll(EnrollPayload {
        hostname: hostname.to_string(),
        friendly_name: friendly_name.to_string(),
        enrollment_token: enrollment_token.map(|s| s.to_string()),
        service_type: "agent".to_string(),
        host_info: Some(host_info),
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
                    ControllerMessage::ServerRestarting(payload) => {
                        tracing::info!(reason = %payload.reason, "controller is restarting during enrollment");
                        return Err(report!(Error::ReceiveClosed));
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
                        tracing::info!(service_id = %payload.service_id, "enrollment approved");
                        return Ok(());
                    }
                    ControllerMessage::Rejected(payload) => {
                        tracing::error!(service_id = %payload.service_id, "enrollment rejected");
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
                    ControllerMessage::ServerRestarting(payload) => {
                        tracing::info!(reason = %payload.reason, "controller is restarting while waiting for approval");
                        return Err(report!(Error::ReceiveClosed));
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

/// Send RequestCertificate with a CSR and read Certificate response.
pub async fn request_certificate_ws(
    ws: &mut WsStream,
    csr_pem: &str,
) -> Result<CertificatePayload> {
    let msg = ServiceMessage::RequestCertificate(RequestCertificatePayload {
        csr_pem: csr_pem.to_string(),
    });
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
                    ControllerMessage::ServerRestarting(payload) => {
                        tracing::info!(reason = %payload.reason, "controller is restarting during certificate request");
                        return Err(report!(Error::ReceiveClosed));
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

/// State for an in-flight update execution.
struct InFlightUpdate {
    update_history_id: String,
    handle: tokio::task::JoinHandle<crate::update::UpdateExecutionResult>,
    output_rx: tokio::sync::mpsc::Receiver<crate::update::UpdateOutputMessage>,
}

/// Authenticated Ping/Pong event loop (mTLS connection) with renewal timer.
#[allow(clippy::too_many_arguments)]
pub async fn run_authenticated_loop(
    host: &str,
    port: u16,
    base_url: &str,
    pki_addr: Option<&str>,
    ca_pem: Option<&[u8]>,
    tls_connector: TlsConnector,
    cert_not_after_ts: Option<i64>,
    config_dir: &std::path::Path,
    state_dir: &std::path::Path,
) -> Result<LoopOutcome> {
    use std::pin::Pin;
    use std::time::Duration;

    const PING_INTERVAL: Duration = Duration::from_secs(300);
    const DEFAULT_SHUTDOWN_TIMEOUT: u32 = 120;

    let mut ws_stream = connect_ws(host, port, &tls_connector, None).await?;

    // Send host info immediately after connecting
    let host_info = crate::host_info::collect_host_info();
    let report_msg = ServiceMessage::ReportHostInfo(ReportHostInfoPayload {
        host_info,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    });
    let report_json = serde_json::to_string(&report_msg).context_to::<Error>()?;
    ws_stream
        .send(Message::Text(report_json.into()))
        .await
        .context_to::<Error>()?;
    tracing::debug!(
        "sent ReportHostInfo with agent_version={}",
        env!("CARGO_PKG_VERSION")
    );

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context_to::<Error>()?;
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .context_to::<Error>()?;

    // First tick completes immediately, sending an initial ping on connect
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await;

    // Renewal timer — initially far-future, reset when AgentSettings arrives
    let mut renewal_sleep: Pin<Box<tokio::time::Sleep>> = Box::pin(tokio::time::sleep(FAR_FUTURE));

    // Holds the private key for a pending renewal CSR until the cert arrives
    let mut pending_renewal_key: Option<String> = None;

    // Shutdown timeout from controller settings
    let mut shutdown_timeout_seconds: u32 = DEFAULT_SHUTDOWN_TIMEOUT;

    // Track in-flight update (only one at a time)
    let mut in_flight_update: Option<InFlightUpdate> = None;

    let outcome = loop {
        // If there's an in-flight update, poll it alongside other events
        let update_poll = async {
            if let Some(ref mut update) = in_flight_update {
                tokio::select! {
                    biased;
                    Some(output_msg) = update.output_rx.recv() => {
                        Some(UpdateEvent::Output(output_msg))
                    }
                    result = &mut update.handle => {
                        Some(UpdateEvent::Completed(result))
                    }
                }
            } else {
                std::future::pending::<Option<UpdateEvent>>().await
            }
        };

        tokio::select! {
            biased;

            // Handle in-flight update events first
            Some(event) = update_poll => {
                let update = in_flight_update.as_ref().expect("update must exist");
                let update_history_id = update.update_history_id.clone();

                match event {
                    UpdateEvent::Output(output_msg) => {
                        let output = ServiceMessage::UpdateOutput(UpdateOutputPayload {
                            update_history_id,
                            output: output_msg.output,
                            stream: output_msg.stream,
                        });
                        if let Ok(json) = serde_json::to_string(&output) {
                            let _ = ws_stream.send(Message::Text(json.into())).await;
                        }
                    }
                    UpdateEvent::Completed(result) => {
                        send_update_result(&mut ws_stream, &update_history_id, result).await;
                        in_flight_update = None;
                    }
                }
            }

            _ = ping_interval.tick() => {
                let agent_ts = now_millis();
                let ping = ServiceMessage::Ping(PingPayload { agent_ts });
                let ping_json = serde_json::to_string(&ping).context_to::<Error>()?;

                tracing::trace!(agent_ts, "sending ping");
                ws_stream
                    .send(Message::Text(ping_json.into()))
                    .await
                    .context_to::<Error>()?;
            }
            _ = &mut renewal_sleep => {
                tracing::info!("renewal window reached, requesting certificate renewal");
                // Extract client_id from the current cert CN
                let client_id_str = extract_agent_id_from_state(state_dir);
                let (key_pem, csr_pem) = generate_keypair_and_csr(&client_id_str)?;
                pending_renewal_key = Some(key_pem);
                let msg = ServiceMessage::RenewCertificate(RenewCertificatePayload {
                    csr_pem,
                });
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
                        break LoopOutcome::Disconnected;
                    }
                    Some(Err(e)) => return Err(e).context_to::<Error>()?,
                    None => {
                        tracing::info!("connection closed by controller");
                        break LoopOutcome::Disconnected;
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
                                // Save new cert + new key to disk
                                let key_pem = match pending_renewal_key.take() {
                                    Some(k) => k,
                                    None => {
                                        tracing::error!("received certificate but no pending renewal key");
                                        break LoopOutcome::Disconnected;
                                    }
                                };
                                let cert_state = crate::state::AgentCertState {
                                    cert_pem: payload.cert_pem,
                                    key_pem,
                                };
                                cert_state.save(state_dir)?;
                                tracing::info!("renewed certificate saved, reconnecting");
                                break LoopOutcome::Reconnect;
                            }
                            ControllerMessage::ServiceSettings(settings) => {
                                tracing::trace!(
                                    renewal_window_hours = settings.renewal_window_hours,
                                    shutdown_timeout = ?settings.shutdown_timeout_seconds,
                                    "received service settings"
                                );
                                shutdown_timeout_seconds = settings.shutdown_timeout_seconds.unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);
                                renewal_sleep.as_mut().reset(
                                    tokio::time::Instant::now()
                                        + compute_renewal_delay(
                                            cert_not_after_ts,
                                            settings.renewal_window_hours,
                                        ),
                                );

                                // Check if CA bundle is stale
                                if !settings.ca_bundle_hash.is_empty() {
                                    let local_hash = compute_local_ca_hash(config_dir);
                                    if local_hash != settings.ca_bundle_hash {
                                        tracing::info!("CA bundle hash mismatch, fetching updated bundle");
                                        let ca_fetch_url = pki_addr.unwrap_or(base_url);
                                        let tls_mode = match ca_pem {
                                            Some(pem) => TlsMode::PinnedCa(pem),
                                            None => TlsMode::SystemTrust,
                                        };
                                        match fetch_ca_certificate(ca_fetch_url, tls_mode).await {
                                            Ok(pem) => {
                                                if let Err(e) = crate::state::save_ca_cert(config_dir, &pem) {
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
                                if let Err(e) = crate::state::save_ca_cert(config_dir, payload.ca_bundle_pem.as_bytes()) {
                                    tracing::warn!("failed to save updated CA bundle: {e}");
                                } else {
                                    tracing::info!("updated CA bundle saved to disk");
                                }
                            }
                            ControllerMessage::RequestCertRenewal(payload) => {
                                tracing::info!(reason = %payload.reason, "controller requested immediate certificate renewal");
                                let client_id_str = extract_agent_id_from_state(state_dir);
                                let (key_pem, csr_pem) = match generate_keypair_and_csr(&client_id_str) {
                                    Ok(pair) => pair,
                                    Err(e) => {
                                        tracing::error!(error = %e, "failed to generate keypair for renewal");
                                        break LoopOutcome::Disconnected;
                                    }
                                };
                                pending_renewal_key = Some(key_pem);
                                let renew_msg = serde_json::to_string(
                                    &ServiceMessage::RenewCertificate(RenewCertificatePayload {
                                        csr_pem,
                                    }),
                                )
                                .context_to::<Error>()?;
                                if let Err(e) = ws_stream.send(Message::Text(renew_msg.into())).await {
                                    tracing::error!(error = %e, "failed to send renewal request");
                                    break LoopOutcome::Disconnected;
                                }
                                tracing::debug!("sent RenewCertificate in response to RequestCertRenewal");
                            }
                            ControllerMessage::CheckVersions(payload) => {
                                tracing::info!(count = payload.assignments.len(), "received CheckVersions request");
                                let mut results = Vec::with_capacity(payload.assignments.len());
                                for assignment in &payload.assignments {
                                    tracing::debug!(
                                        software_item_id = %assignment.software_item_id,
                                        name = %assignment.name,
                                        provider_type = %assignment.provider_type,
                                        "checking version"
                                    );
                                    let (installed_version, error) = crate::version_check::check_version(
                                        assignment.provider_type.clone(),
                                        &assignment.package_identifier,
                                        &assignment.config,
                                    ).await;
                                    results.push(VersionCheckResult {
                                        software_item_id: assignment.software_item_id.clone(),
                                        installed_version,
                                        error,
                                    });
                                }
                                let response = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload {
                                    results,
                                });
                                let response_json = serde_json::to_string(&response).context_to::<Error>()?;
                                if let Err(e) = ws_stream.send(Message::Text(response_json.into())).await {
                                    tracing::error!(error = %e, "failed to send VersionCheckResults");
                                    break LoopOutcome::Disconnected;
                                }
                                tracing::debug!("sent VersionCheckResults");
                            }
                            ControllerMessage::ExecuteUpdate(payload) => {
                                let payload = *payload;
                                tracing::info!(
                                    update_id = %payload.update_history_id,
                                    software = %payload.software_item_name,
                                    version = %payload.to_version,
                                    "received update request"
                                );

                                // If there's already an in-flight update, reject this one
                                if in_flight_update.is_some() {
                                    tracing::warn!(
                                        update_id = %payload.update_history_id,
                                        "rejecting update: another update is already in progress"
                                    );
                                    let result_msg = ServiceMessage::UpdateResult(UpdateResultPayload {
                                        update_history_id: payload.update_history_id,
                                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                                        from_version: None,
                                        to_version: None,
                                        output: String::new(),
                                        error: Some("Another update is already in progress".to_string()),
                                    });
                                    if let Ok(json) = serde_json::to_string(&result_msg) {
                                        let _ = ws_stream.send(Message::Text(json.into())).await;
                                    }
                                    continue;
                                }

                                // Create a channel for output streaming
                                let (output_tx, output_rx) = tokio::sync::mpsc::channel::<crate::update::UpdateOutputMessage>(100);

                                // Clone what we need for the spawned task
                                let update_history_id = payload.update_history_id.clone();

                                // Spawn update execution task
                                let handle = tokio::spawn(async move {
                                    crate::update::execute_update(payload, output_tx).await
                                });

                                // Send UpdateStarted
                                let started_msg = ServiceMessage::UpdateStarted(UpdateStartedPayload {
                                    update_history_id: update_history_id.clone(),
                                    from_version: None,
                                });
                                let started_json = serde_json::to_string(&started_msg).context_to::<Error>()?;
                                if let Err(e) = ws_stream.send(Message::Text(started_json.into())).await {
                                    tracing::error!(error = %e, "failed to send UpdateStarted");
                                }

                                // Track the in-flight update
                                in_flight_update = Some(InFlightUpdate {
                                    update_history_id,
                                    handle,
                                    output_rx,
                                });
                            }
                            ControllerMessage::ServerRestarting(payload) => {
                                tracing::info!(reason = %payload.reason, "controller is restarting");
                                // Connection will close, agent's reconnect logic handles the rest
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
                            break LoopOutcome::Reconnect;
                        } else if reason == "certificate revoked" {
                            tracing::warn!("connection closed: certificate revoked");
                            break LoopOutcome::Disconnected;
                        } else {
                            log_close_frame(frame);
                            break LoopOutcome::Disconnected;
                        }
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received SIGINT, initiating graceful shutdown");
                break handle_graceful_shutdown(
                    &mut ws_stream,
                    in_flight_update.take(),
                    shutdown_timeout_seconds,
                    DisconnectReason::Shutdown,
                    LoopOutcome::Shutdown,
                ).await;
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, initiating graceful shutdown");
                break handle_graceful_shutdown(
                    &mut ws_stream,
                    in_flight_update.take(),
                    shutdown_timeout_seconds,
                    DisconnectReason::Shutdown,
                    LoopOutcome::Shutdown,
                ).await;
            }
            _ = sighup.recv() => {
                tracing::info!("received SIGHUP, initiating graceful restart");
                break handle_graceful_shutdown(
                    &mut ws_stream,
                    in_flight_update.take(),
                    shutdown_timeout_seconds,
                    DisconnectReason::Restart,
                    LoopOutcome::Restart,
                ).await;
            }
        }
    };

    // Best-effort close — the peer may have already disconnected.
    match ws_stream.close(None).await {
        Ok(()) => tracing::info!("websocket closed gracefully"),
        Err(e) if is_peer_closed(&e) => tracing::info!("websocket already closed by peer"),
        Err(e) => return Err(e).context_to::<Error>()?,
    }

    Ok(outcome)
}

/// Events from an in-flight update.
enum UpdateEvent {
    Output(crate::update::UpdateOutputMessage),
    Completed(std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>),
}

/// Send the final update result to the controller.
async fn send_update_result(
    ws_stream: &mut WsStream,
    update_history_id: &str,
    result: std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>,
) {
    match result {
        Ok(exec_result) => {
            let result_msg = ServiceMessage::UpdateResult(exec_result.result);
            if let Ok(json) = serde_json::to_string(&result_msg) {
                let _ = ws_stream.send(Message::Text(json.into())).await;
            }
            tracing::info!(update_id = %update_history_id, "update execution completed");
        }
        Err(e) => {
            tracing::error!(error = %e, "update task panicked");
            let result_msg = ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id: update_history_id.to_string(),
                status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some("Update task panicked".to_string()),
            });
            if let Ok(json) = serde_json::to_string(&result_msg) {
                let _ = ws_stream.send(Message::Text(json.into())).await;
            }
        }
    }
}

/// Handle graceful shutdown sequence:
/// 1. Wait for in-flight update to complete (with timeout)
/// 2. Send Disconnecting message to controller
/// 3. Return the appropriate LoopOutcome
async fn handle_graceful_shutdown(
    ws_stream: &mut WsStream,
    in_flight_update: Option<InFlightUpdate>,
    timeout_seconds: u32,
    disconnect_reason: DisconnectReason,
    outcome: LoopOutcome,
) -> LoopOutcome {
    use std::time::Duration;

    if let Some(mut update) = in_flight_update {
        tracing::info!(
            update_id = %update.update_history_id,
            timeout_seconds,
            "waiting for in-flight update to complete before shutdown"
        );

        let timeout = Duration::from_secs(u64::from(timeout_seconds));
        let deadline = tokio::time::Instant::now() + timeout;

        // Continue processing output and wait for completion
        loop {
            tokio::select! {
                biased;

                Some(output_msg) = update.output_rx.recv() => {
                    let output = ServiceMessage::UpdateOutput(UpdateOutputPayload {
                        update_history_id: update.update_history_id.clone(),
                        output: output_msg.output,
                        stream: output_msg.stream,
                    });
                    if let Ok(json) = serde_json::to_string(&output) {
                        let _ = ws_stream.send(Message::Text(json.into())).await;
                    }
                }
                result = &mut update.handle => {
                    send_update_result(ws_stream, &update.update_history_id, result).await;
                    break;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!(
                        update_id = %update.update_history_id,
                        "shutdown timeout reached, abandoning in-flight update"
                    );
                    // Send a timeout failure result
                    let result_msg = ServiceMessage::UpdateResult(UpdateResultPayload {
                        update_history_id: update.update_history_id.clone(),
                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                        from_version: None,
                        to_version: None,
                        output: String::new(),
                        error: Some(format!("Agent shutdown timeout ({timeout_seconds}s) reached")),
                    });
                    if let Ok(json) = serde_json::to_string(&result_msg) {
                        let _ = ws_stream.send(Message::Text(json.into())).await;
                    }
                    break;
                }
            }
        }

        // Drain any remaining output messages
        while let Ok(output_msg) = update.output_rx.try_recv() {
            let output = ServiceMessage::UpdateOutput(UpdateOutputPayload {
                update_history_id: update.update_history_id.clone(),
                output: output_msg.output,
                stream: output_msg.stream,
            });
            if let Ok(json) = serde_json::to_string(&output) {
                let _ = ws_stream.send(Message::Text(json.into())).await;
            }
        }
    }

    // Send Disconnecting message to controller
    let disconnecting_msg = ServiceMessage::Disconnecting(DisconnectingPayload {
        reason: disconnect_reason,
        active_tenants: vec![],
    });
    if let Ok(json) = serde_json::to_string(&disconnecting_msg) {
        if let Err(e) = ws_stream.send(Message::Text(json.into())).await {
            tracing::debug!(error = %e, "failed to send Disconnecting message");
        } else {
            tracing::debug!(reason = ?disconnect_reason, "sent Disconnecting message to controller");
        }
    }

    outcome
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

/// Extract the agent_id (CN) from the agent's current state on disk.
fn extract_agent_id_from_state(data_dir: &std::path::Path) -> String {
    if let Ok(Some(state)) = crate::state::AgentState::load(data_dir) {
        return state.agent_id;
    }
    // Fallback: this shouldn't happen in normal flow
    "unknown".to_string()
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
