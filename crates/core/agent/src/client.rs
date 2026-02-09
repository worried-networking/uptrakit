use futures_util::{SinkExt, StreamExt};
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use uptrakit_enrollment::ca::{CaTlsMode, fetch_ca_certificate};
use uptrakit_enrollment::identity::generate_keypair_and_csr;
use uptrakit_enrollment::ws::{WsStream, connect_ws, is_peer_closed, log_close_frame};
use uptrakit_internal_wire::{
    CertificatePayload, ControllerEnvelope, ControllerMessage, DisconnectReason,
    DisconnectingPayload, IncomingSeq, OutgoingSeq, PingPayload, RenewCertificatePayload,
    ReportHostInfoPayload, ServiceMessage, UpdateOutputPayload, UpdateResultPayload,
    UpdateStartedPayload, VersionCheckResult, VersionCheckResultsPayload, now_millis,
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
    /// SIGHUP received — exit for external restart.
    Restart,
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
    update_history_id: uuid::Uuid,
    handle: tokio::task::JoinHandle<crate::update::UpdateExecutionResult>,
    output_rx: tokio::sync::mpsc::Receiver<crate::update::UpdateOutputMessage>,
}

/// Parameters for [`run_authenticated_loop`].
pub struct AuthenticatedLoopParams<'a> {
    pub host: &'a str,
    pub port: u16,
    pub base_url: &'a str,
    pub pki_addr: Option<&'a str>,
    pub ca_pem: Option<&'a [u8]>,
    pub tls_connector: tokio_rustls::TlsConnector,
    pub cert_not_after_ts: Option<i64>,
    pub identity: &'a uptrakit_enrollment::ServiceIdentityState,
}

/// Authenticated Ping/Pong event loop (mTLS connection) with renewal timer.
pub async fn run_authenticated_loop(params: AuthenticatedLoopParams<'_>) -> Result<LoopOutcome> {
    let AuthenticatedLoopParams {
        host,
        port,
        base_url,
        pki_addr,
        ca_pem,
        tls_connector,
        cert_not_after_ts,
        identity,
    } = params;
    use std::pin::Pin;
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;

    const PING_INTERVAL: Duration = Duration::from_secs(300);
    const DEFAULT_SHUTDOWN_TIMEOUT: u32 = 120;

    let mut ws_stream = connect_ws(host, port, &tls_connector, None)
        .await
        .context_to::<Error>()?;

    let mut out_seq = OutgoingSeq::new();
    let mut in_seq = IncomingSeq::new();

    // Send host info immediately after connecting
    let host_info = crate::host_info::collect_host_info();
    let report_msg = ServiceMessage::ReportHostInfo(ReportHostInfoPayload {
        host_info,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    });
    let report_json =
        serde_json::to_string(&out_seq.wrap_service(report_msg)).context_to::<Error>()?;
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

    let config_dir = identity.config_dir();
    let state_dir = identity.state_dir();

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
                let Some(ref update) = in_flight_update else {
                    tracing::error!("received update event but no in-flight update exists");
                    continue;
                };
                let update_history_id = update.update_history_id;

                match event {
                    UpdateEvent::Output(output_msg) => {
                        let output = ServiceMessage::UpdateOutput(UpdateOutputPayload {
                            update_history_id,
                            output: output_msg.output,
                            stream: output_msg.stream,
                        });
                        if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(output)) {
                            let _ = ws_stream.send(Message::Text(json.into())).await;
                        }
                    }
                    UpdateEvent::Completed(result) => {
                        send_update_result(&mut ws_stream, &mut out_seq, update_history_id, result).await;
                        in_flight_update = None;
                    }
                }
            }

            _ = ping_interval.tick() => {
                let service_ts = now_millis();
                let ping = ServiceMessage::Ping(PingPayload { service_ts });
                let ping_json = serde_json::to_string(&out_seq.wrap_service(ping)).context_to::<Error>()?;

                tracing::trace!(service_ts, "sending ping");
                ws_stream
                    .send(Message::Text(ping_json.into()))
                    .await
                    .context_to::<Error>()?;
            }
            _ = &mut renewal_sleep => {
                tracing::info!("renewal window reached, requesting certificate renewal");
                let client_id_str = match extract_service_id(identity) {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::error!(error = %e, "cannot renew certificate: no service ID");
                        break LoopOutcome::Disconnected;
                    }
                };
                let (key_pem, csr_pem) = generate_keypair_and_csr(&client_id_str)
                    .context_to::<Error>()?;
                pending_renewal_key = Some(key_pem);
                let msg = ServiceMessage::RenewCertificate(RenewCertificatePayload {
                    csr_pem,
                });
                let json = serde_json::to_string(&out_seq.wrap_service(msg)).context_to::<Error>()?;
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
                        let envelope: ControllerEnvelope = match serde_json::from_str(&text) {
                            Ok(env) => env,
                            Err(e) => {
                                tracing::debug!("ignoring unrecognized controller message: {e}");
                                continue;
                            }
                        };
                        if let Err(e) = in_seq.validate(envelope.seq) {
                            tracing::error!("sequence validation failed: {e}");
                            break LoopOutcome::Disconnected;
                        }
                        let controller_msg = envelope.message;

                        match controller_msg {
                            ControllerMessage::Pong(pong) => {
                                let now = now_millis();
                                let rtt = now - pong.service_ts;
                                tracing::trace!(
                                    service_ts = pong.service_ts,
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
                                save_renewed_cert(state_dir, &payload, &key_pem).await?;
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
                                    let local_hash = compute_local_ca_hash(config_dir).await;
                                    if local_hash != settings.ca_bundle_hash {
                                        tracing::info!("CA bundle hash mismatch, fetching updated bundle");
                                        let ca_fetch_url = pki_addr.unwrap_or(base_url);
                                        let tls_mode = match ca_pem {
                                            Some(pem) => CaTlsMode::PinnedCa(pem),
                                            None => CaTlsMode::SystemTrust,
                                        };
                                        match fetch_ca_certificate(ca_fetch_url, tls_mode).await {
                                            Ok(pem) => {
                                                if let Err(e) = save_ca_cert(config_dir, &pem).await {
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
                                if let Err(e) = save_ca_cert(config_dir, payload.ca_bundle_pem.as_bytes()).await {
                                    tracing::warn!("failed to save updated CA bundle: {e}");
                                } else {
                                    tracing::info!("updated CA bundle saved to disk");
                                }
                            }
                            ControllerMessage::RequestCertRenewal(payload) => {
                                tracing::info!(reason = %payload.reason, "controller requested immediate certificate renewal");
                                let client_id_str = match extract_service_id(identity) {
                                    Ok(id) => id,
                                    Err(e) => {
                                        tracing::error!(error = %e, "cannot renew certificate: no service ID");
                                        break LoopOutcome::Disconnected;
                                    }
                                };
                                let (key_pem, csr_pem) = match generate_keypair_and_csr(&client_id_str).context_to::<Error>() {
                                    Ok(pair) => pair,
                                    Err(e) => {
                                        tracing::error!(error = %e, "failed to generate keypair for renewal");
                                        break LoopOutcome::Disconnected;
                                    }
                                };
                                pending_renewal_key = Some(key_pem);
                                let renew_msg = serde_json::to_string(
                                    &out_seq.wrap_service(ServiceMessage::RenewCertificate(RenewCertificatePayload {
                                        csr_pem,
                                    })),
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
                                use futures_util::stream::{self, StreamExt};
                                let results: Vec<VersionCheckResult> = stream::iter(&payload.assignments)
                                    .map(|assignment| async move {
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
                                        VersionCheckResult {
                                            software_item_id: assignment.software_item_id,
                                            installed_version,
                                            error,
                                        }
                                    })
                                    .buffer_unordered(8)
                                    .collect()
                                    .await;
                                let response = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload {
                                    results,
                                });
                                let response_json = serde_json::to_string(&out_seq.wrap_service(response)).context_to::<Error>()?;
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
                                    if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(result_msg)) {
                                        let _ = ws_stream.send(Message::Text(json.into())).await;
                                    }
                                    continue;
                                }

                                // Create a channel for output streaming
                                let (output_tx, output_rx) = tokio::sync::mpsc::channel::<crate::update::UpdateOutputMessage>(100);

                                // Copy what we need for the spawned task
                                let update_history_id = payload.update_history_id;

                                // Spawn update execution task
                                let handle = tokio::spawn(async move {
                                    crate::update::execute_update(payload, output_tx).await
                                });

                                // Send UpdateStarted
                                let started_msg = ServiceMessage::UpdateStarted(UpdateStartedPayload {
                                    update_history_id,
                                    from_version: None,
                                });
                                let started_json = serde_json::to_string(&out_seq.wrap_service(started_msg)).context_to::<Error>()?;
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
                    &mut out_seq,
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
                    &mut out_seq,
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
                    &mut out_seq,
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
    out_seq: &mut OutgoingSeq,
    update_history_id: uuid::Uuid,
    result: std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>,
) {
    use tokio_tungstenite::tungstenite::Message;

    match result {
        Ok(exec_result) => {
            let result_msg = ServiceMessage::UpdateResult(exec_result.result);
            if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(result_msg)) {
                let _ = ws_stream.send(Message::Text(json.into())).await;
            }
            tracing::info!(update_id = %update_history_id, "update execution completed");
        }
        Err(e) => {
            tracing::error!(error = %e, "update task panicked");
            let result_msg = ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id,
                status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some("Update task panicked".to_string()),
            });
            if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(result_msg)) {
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
    out_seq: &mut OutgoingSeq,
    in_flight_update: Option<InFlightUpdate>,
    timeout_seconds: u32,
    disconnect_reason: DisconnectReason,
    outcome: LoopOutcome,
) -> LoopOutcome {
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;

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
                        update_history_id: update.update_history_id,
                        output: output_msg.output,
                        stream: output_msg.stream,
                    });
                    if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(output)) {
                        let _ = ws_stream.send(Message::Text(json.into())).await;
                    }
                }
                result = &mut update.handle => {
                    send_update_result(ws_stream, out_seq, update.update_history_id, result).await;
                    break;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!(
                        update_id = %update.update_history_id,
                        "shutdown timeout reached, abandoning in-flight update"
                    );
                    // Send a timeout failure result
                    let result_msg = ServiceMessage::UpdateResult(UpdateResultPayload {
                        update_history_id: update.update_history_id,
                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                        from_version: None,
                        to_version: None,
                        output: String::new(),
                        error: Some(format!("Agent shutdown timeout ({timeout_seconds}s) reached")),
                    });
                    if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(result_msg)) {
                        let _ = ws_stream.send(Message::Text(json.into())).await;
                    }
                    break;
                }
            }
        }

        // Drain any remaining output messages
        while let Ok(output_msg) = update.output_rx.try_recv() {
            let output = ServiceMessage::UpdateOutput(UpdateOutputPayload {
                update_history_id: update.update_history_id,
                output: output_msg.output,
                stream: output_msg.stream,
            });
            if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(output)) {
                let _ = ws_stream.send(Message::Text(json.into())).await;
            }
        }
    }

    // Send Disconnecting message to controller
    let disconnecting_msg =
        ServiceMessage::Disconnecting(DisconnectingPayload::new(disconnect_reason));
    if let Ok(json) = serde_json::to_string(&out_seq.wrap_service(disconnecting_msg)) {
        if let Err(e) = ws_stream.send(Message::Text(json.into())).await {
            tracing::debug!(error = %e, "failed to send Disconnecting message");
        } else {
            tracing::debug!(reason = ?disconnect_reason, "sent Disconnecting message to controller");
        }
    }

    outcome
}

/// Compute SHA-256 hex hash of the local CA certificate file.
async fn compute_local_ca_hash(config_dir: &std::path::Path) -> String {
    let ca_path = config_dir.join("ca.pem");
    match tokio::fs::read(&ca_path).await {
        Ok(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        }
        Err(_) => String::new(),
    }
}

/// Extract the service_id string from the identity state.
///
/// Returns an error if the identity has no service ID, since this is only
/// called during certificate renewal when the identity must be enrolled.
fn extract_service_id(identity: &uptrakit_enrollment::ServiceIdentityState) -> Result<String> {
    identity
        .service_id()
        .map(|id| id.to_string())
        .ok_or_else(|| {
            report!(Error::Enrollment(
                uptrakit_enrollment::EnrollmentError::NotEnrolled
            ))
        })
}

/// Save renewed cert + key to state directory.
async fn save_renewed_cert(
    state_dir: &std::path::Path,
    payload: &CertificatePayload,
    key_pem: &str,
) -> Result<()> {
    let cert_path = state_dir.join("service.crt");
    let key_path = state_dir.join("service.key");
    tokio::fs::write(&cert_path, &payload.cert_pem)
        .await
        .context_to::<Error>()?;
    set_secure_permissions(&cert_path).await?;
    tokio::fs::write(&key_path, key_pem)
        .await
        .context_to::<Error>()?;
    set_secure_permissions(&key_path).await?;
    Ok(())
}

/// Save CA cert bytes to config directory.
async fn save_ca_cert(config_dir: &std::path::Path, pem: &[u8]) -> Result<()> {
    let path = config_dir.join("ca.pem");
    tokio::fs::write(&path, pem).await.context_to::<Error>()?;
    set_secure_permissions(&path).await?;
    Ok(())
}

async fn set_secure_permissions(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .await
            .context_to::<Error>()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_internal_wire::now_millis;

    // ── compute_renewal_delay ───────────────────────────────────────────

    #[test]
    fn renewal_delay_no_cert() {
        // When no cert_not_after is known, return FAR_FUTURE.
        let delay = compute_renewal_delay(None, 168);
        assert_eq!(delay, FAR_FUTURE);
    }

    #[test]
    fn renewal_delay_future_cert() {
        // Cert expires in 30 days, window is 7 days (168h) → delay ≈ 23 days.
        let thirty_days_ms = 30 * 24 * 3600 * 1000_i64;
        let not_after = now_millis() + thirty_days_ms;
        let delay = compute_renewal_delay(Some(not_after), 168);
        let twenty_three_days = std::time::Duration::from_millis(23 * 24 * 3600 * 1000);
        // Should be roughly 23 days (±1 second for timing jitter).
        assert!(delay >= twenty_three_days - std::time::Duration::from_secs(1));
        assert!(delay <= twenty_three_days + std::time::Duration::from_secs(1));
    }

    #[test]
    fn renewal_delay_already_in_window() {
        // Cert expires in 3 days, window is 168h (7 days) → already past, delay = 0.
        let three_days_ms = 3 * 24 * 3600 * 1000_i64;
        let not_after = now_millis() + three_days_ms;
        let delay = compute_renewal_delay(Some(not_after), 168);
        assert_eq!(delay, std::time::Duration::ZERO);
    }

    #[test]
    fn renewal_delay_expired_cert() {
        // Cert already expired → delay is 0 (clamped via max(0)).
        let not_after = now_millis() - 1000;
        let delay = compute_renewal_delay(Some(not_after), 168);
        assert_eq!(delay, std::time::Duration::ZERO);
    }

    #[test]
    fn renewal_delay_zero_window() {
        // Window of 0 hours → renew only at exact expiry time.
        let one_hour_ms = 3600 * 1000_i64;
        let not_after = now_millis() + one_hour_ms;
        let delay = compute_renewal_delay(Some(not_after), 0);
        // Should be roughly 1 hour.
        assert!(delay >= std::time::Duration::from_secs(3599));
        assert!(delay <= std::time::Duration::from_secs(3601));
    }

    // ── compute_local_ca_hash ───────────────────────────────────────────

    #[tokio::test]
    async fn local_ca_hash_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hash = compute_local_ca_hash(dir.path()).await;
        assert!(hash.is_empty());
    }

    #[tokio::test]
    async fn local_ca_hash_valid_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        tokio::fs::write(&ca_path, b"test-ca-content")
            .await
            .expect("write");
        let hash = compute_local_ca_hash(dir.path()).await;
        // SHA-256 of "test-ca-content"
        let expected = {
            let mut h = Sha256::new();
            h.update(b"test-ca-content");
            hex::encode(h.finalize())
        };
        assert_eq!(hash, expected);
    }

    // ── extract_service_id ──────────────────────────────────────────────

    #[test]
    fn extract_service_id_without_id_returns_error() {
        let dir = std::path::Path::new("/tmp/nonexistent-test-dir");
        let identity = uptrakit_enrollment::ServiceIdentityState::new(dir, dir);
        // No service.json loaded → service_id is None → returns error
        assert!(extract_service_id(&identity).is_err());
    }

    #[tokio::test]
    async fn extract_service_id_with_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_path = dir.path().join("service.json");
        let id = uuid::Uuid::now_v7();
        let json = serde_json::json!({
            "service_id": id.to_string(),
            "enrollment_secret": "test-secret"
        });
        tokio::fs::write(&state_path, json.to_string())
            .await
            .expect("write");
        let mut identity = uptrakit_enrollment::ServiceIdentityState::new(dir.path(), dir.path());
        identity.load().await.expect("load");
        assert_eq!(
            extract_service_id(&identity).expect("should have id"),
            id.to_string()
        );
    }

    // ── save_renewed_cert ───────────────────────────────────────────────

    #[tokio::test]
    async fn save_renewed_cert_writes_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Construct CertificatePayload via deserialization to avoid direct time dependency.
        let payload: CertificatePayload = serde_json::from_value(serde_json::json!({
            "cert_pem": "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----",
            "not_after": 0
        }))
        .expect("deserialize");
        save_renewed_cert(dir.path(), &payload, "key-pem-data")
            .await
            .expect("save");
        let cert = tokio::fs::read_to_string(dir.path().join("service.crt"))
            .await
            .expect("read cert");
        let key = tokio::fs::read_to_string(dir.path().join("service.key"))
            .await
            .expect("read key");
        assert_eq!(cert, payload.cert_pem);
        assert_eq!(key, "key-pem-data");
    }

    // ── save_ca_cert ────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_ca_cert_writes_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        save_ca_cert(dir.path(), b"ca-pem-data")
            .await
            .expect("save");
        let content = tokio::fs::read_to_string(dir.path().join("ca.pem"))
            .await
            .expect("read");
        assert_eq!(content, "ca-pem-data");
    }
}
