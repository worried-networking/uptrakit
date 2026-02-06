use futures_util::{SinkExt, StreamExt};
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use uptrakit_enrollment::ca::{CaTlsMode, fetch_ca_certificate};
use uptrakit_enrollment::identity::generate_keypair_and_csr;
use uptrakit_enrollment::ws::{WsStream, connect_ws, is_peer_closed, log_close_frame};
use uptrakit_internal_wire::{
    CertificatePayload, ControllerMessage, DisconnectReason, DisconnectingPayload, PingPayload,
    RenewCertificatePayload, ReportHostInfoPayload, ServiceMessage, UpdateOutputPayload,
    UpdateResultPayload, UpdateStartedPayload, VersionCheckResult, VersionCheckResultsPayload,
    now_millis,
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
    update_history_id: String,
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
                let client_id_str = extract_service_id(identity);
                let (key_pem, csr_pem) = generate_keypair_and_csr(&client_id_str)
                    .context_to::<Error>()?;
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
                                save_renewed_cert(state_dir, &payload, &key_pem)?;
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
                                            Some(pem) => CaTlsMode::PinnedCa(pem),
                                            None => CaTlsMode::SystemTrust,
                                        };
                                        match fetch_ca_certificate(ca_fetch_url, tls_mode).await {
                                            Ok(pem) => {
                                                if let Err(e) = save_ca_cert_sync(config_dir, &pem) {
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
                                if let Err(e) = save_ca_cert_sync(config_dir, payload.ca_bundle_pem.as_bytes()) {
                                    tracing::warn!("failed to save updated CA bundle: {e}");
                                } else {
                                    tracing::info!("updated CA bundle saved to disk");
                                }
                            }
                            ControllerMessage::RequestCertRenewal(payload) => {
                                tracing::info!(reason = %payload.reason, "controller requested immediate certificate renewal");
                                let client_id_str = extract_service_id(identity);
                                let (key_pem, csr_pem) = match generate_keypair_and_csr(&client_id_str).context_to::<Error>() {
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
    use tokio_tungstenite::tungstenite::Message;

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
fn compute_local_ca_hash(config_dir: &std::path::Path) -> String {
    let ca_path = config_dir.join("ca.pem");
    match std::fs::read(&ca_path) {
        Ok(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        }
        Err(_) => String::new(),
    }
}

/// Extract the service_id string from the identity state.
fn extract_service_id(identity: &uptrakit_enrollment::ServiceIdentityState) -> String {
    identity
        .service_id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Save renewed cert + key to state directory.
fn save_renewed_cert(
    state_dir: &std::path::Path,
    payload: &CertificatePayload,
    key_pem: &str,
) -> Result<()> {
    let cert_path = state_dir.join("service.crt");
    let key_path = state_dir.join("service.key");
    std::fs::write(&cert_path, &payload.cert_pem).context_to::<Error>()?;
    set_secure_permissions(&cert_path)?;
    std::fs::write(&key_path, key_pem).context_to::<Error>()?;
    set_secure_permissions(&key_path)?;
    Ok(())
}

/// Save CA cert bytes to config directory (sync, for use in authenticated loop).
fn save_ca_cert_sync(config_dir: &std::path::Path, pem: &[u8]) -> Result<()> {
    let path = config_dir.join("ca.pem");
    std::fs::write(&path, pem).context_to::<Error>()?;
    set_secure_permissions(&path)?;
    Ok(())
}

fn set_secure_permissions(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .context_to::<Error>()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
