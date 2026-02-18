use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use uptrakit_internal_wire::{
    CloseReason, ControllerMessage, DisconnectReason, DisconnectingPayload, PingPayload,
    ReportHostsPayload, ServiceMessage, UpdateOutputPayload,
    UpdateResultPayload, UpdateStartedPayload, VersionCheckResult, VersionCheckResultsPayload,
    now_millis,
};
use uptrakit_service_sdk::ca::{CaTlsMode, fetch_ca_certificate};
use uptrakit_service_sdk::{
    CertificateRenewalHandler, ControllerConnection, LoopOutcome, create_renewal_sleep,
    update_renewal_schedule,
};

use crate::error::{Error, Result};

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
    pub identity: &'a mut uptrakit_service_sdk::ServiceIdentityState,
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
    use std::time::Duration;

    const PING_INTERVAL: Duration = Duration::from_secs(300);
    const DEFAULT_SHUTDOWN_TIMEOUT: u32 = 120;

    tracing::info!("connecting to controller (authenticated)");
    let mut conn = ControllerConnection::connect(host, port, &tls_connector, None)
        .await
        .context_to::<Error>()?;

    // Send host info immediately after connecting
    let host_info = crate::host_info::collect_host_info();
    conn.send(ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: vec![host_info],
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: uptrakit_internal_wire::PROTOCOL_VERSION,
    }))
    .await
    .context_to::<Error>()?;
    tracing::debug!(
        "sent ReportHosts with agent_version={}",
        env!("CARGO_PKG_VERSION")
    );

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context_to::<Error>()?;
    #[cfg(unix)]
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .context_to::<Error>()?;

    // First tick completes immediately, sending an initial ping on connect
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await;

    // Renewal timer — initially far-future, reset when ServiceSettings arrives.
    let mut renewal_sleep = create_renewal_sleep();

    // Handles certificate lifecycle messages (CaBundleUpdated,
    // RequestCertRenewal, Certificate) and timer-based renewal.
    let mut cert_handler = CertificateRenewalHandler::new();

    // Shutdown timeout from controller settings
    let mut shutdown_timeout_seconds: u32 = DEFAULT_SHUTDOWN_TIMEOUT;

    // Track in-flight update (only one at a time)
    let mut in_flight_update: Option<InFlightUpdate> = None;

    // Clone directory paths to avoid borrow conflicts with `&mut identity`.
    let config_dir = identity.config_dir().to_path_buf();

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
                        conn.send_best_effort(ServiceMessage::UpdateOutput(UpdateOutputPayload {
                            update_history_id,
                            output: output_msg.output,
                            stream: output_msg.stream,
                        })).await;
                    }
                    UpdateEvent::Completed(result) => {
                        send_update_result(&mut conn, update_history_id, result).await;
                        in_flight_update = None;
                    }
                }
            }

            _ = ping_interval.tick() => {
                let service_ts = now_millis();
                tracing::trace!(service_ts, "sending ping");
                conn.send(ServiceMessage::Ping(PingPayload { service_ts }))
                    .await
                    .context_to::<Error>()?;
            }
            _ = &mut renewal_sleep => {
                if let Some(o) = cert_handler.handle_renewal_timer(identity, &mut conn, &mut renewal_sleep).await {
                    break o;
                }
            }
            msg = conn.recv() => {
                match msg.context_to::<Error>()? {
                    Some(controller_msg) => {
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
                                break cert_handler.handle_certificate(identity, &payload)
                                    .await
                                    .context_to::<Error>()?;
                            }
                            ControllerMessage::ServiceSettings(settings) => {
                                tracing::trace!(
                                    renewal_window_hours = settings.renewal_window_hours,
                                    shutdown_timeout = ?settings.shutdown_timeout_seconds,
                                    "received service settings"
                                );
                                if settings.protocol_version != uptrakit_internal_wire::PROTOCOL_VERSION {
                                    tracing::warn!(
                                        reported = settings.protocol_version,
                                        expected = uptrakit_internal_wire::PROTOCOL_VERSION,
                                        "controller protocol version mismatch"
                                    );
                                }
                                shutdown_timeout_seconds = settings.shutdown_timeout_seconds.unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);
                                update_renewal_schedule(
                                    &mut renewal_sleep,
                                    cert_not_after_ts,
                                    settings.renewal_window_hours,
                                );

                                // Check if CA bundle is stale
                                if !settings.ca_bundle_hash.is_empty() {
                                    let local_hash = compute_local_ca_hash(&config_dir).await;
                                    if local_hash != settings.ca_bundle_hash {
                                        tracing::info!("CA bundle hash mismatch, fetching updated bundle");
                                        let ca_fetch_url = pki_addr.unwrap_or(base_url);
                                        let tls_mode = match ca_pem {
                                            Some(pem) => CaTlsMode::PinnedCa(pem),
                                            None => CaTlsMode::SystemTrust,
                                        };
                                        match fetch_ca_certificate(ca_fetch_url, tls_mode).await {
                                            Ok(pem) => {
                                                let pem_str = String::from_utf8_lossy(&pem);
                                                if let Err(e) = identity.save_ca_cert(&pem_str).await {
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
                                cert_handler.handle_ca_bundle_updated(identity, &payload).await;
                            }
                            ControllerMessage::RequestCertRenewal(payload) => {
                                if let Some(o) = cert_handler.handle_request_cert_renewal(identity, &mut conn, &payload).await {
                                    break o;
                                }
                            }
                            ControllerMessage::CheckVersions(payload) => {
                                tracing::info!(count = payload.assignments.len(), "received CheckVersions request");

                                let executor: std::sync::Arc<dyn uptrakit_command::CommandExecutor> =
                                    std::sync::Arc::new(uptrakit_command::LocalCommandExecutor);

                                // Pre-refresh package indexes for provider types that support it.
                                // Deduplicate by provider type so we only refresh once per type.
                                {
                                    let mut refreshed_types = std::collections::HashSet::new();
                                    for assignment in &payload.assignments {
                                        if refreshed_types.contains(&assignment.provider_type) {
                                            continue;
                                        }
                                        if let Ok(provider) = uptrakit_provider_registry::ProviderRegistry::create_provider(
                                            assignment.provider_type.clone(),
                                            &assignment.config,
                                            std::sync::Arc::clone(&executor),
                                        ) && provider.has_capability(uptrakit_provider_registry::ProviderCapability::RefreshPackageIndex) {
                                            tracing::info!(provider_type = %assignment.provider_type, "refreshing package index");
                                            if let Err(e) = provider.refresh_package_index().await {
                                                tracing::warn!(provider_type = %assignment.provider_type, error = %e, "failed to refresh package index");
                                            }
                                            refreshed_types.insert(assignment.provider_type.clone());
                                        }
                                    }
                                }

                                use futures_util::stream::{self, StreamExt};
                                let results: Vec<VersionCheckResult> = stream::iter(payload.assignments)
                                    .map(|assignment| {
                                        let executor = std::sync::Arc::clone(&executor);
                                        async move {
                                            tracing::debug!(
                                                software_item_id = %assignment.software_item_id,
                                                name = %assignment.name,
                                                provider_type = %assignment.provider_type,
                                                "checking version"
                                            );
                                            let outcome = crate::version_check::check_version(
                                                assignment.provider_type,
                                                &assignment.config,
                                                &assignment.package_identifier,
                                                executor,
                                            ).await;
                                            VersionCheckResult {
                                                software_item_id: assignment.software_item_id,
                                                installed_version: outcome.installed_version,
                                                latest_version: outcome.latest_version,
                                                error: outcome.error,
                                            }
                                        }
                                    })
                                    .buffer_unordered(8)
                                    .collect()
                                    .await;
                                let response = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload {
                                    results,
                                });
                                if let Err(e) = conn.send(response).await {
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
                                    conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                                        update_history_id: payload.update_history_id,
                                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                                        from_version: None,
                                        to_version: None,
                                        output: String::new(),
                                        error: Some("Another update is already in progress".to_string()),
                                    })).await;
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
                                if let Err(e) = conn.send(ServiceMessage::UpdateStarted(UpdateStartedPayload {
                                    update_history_id,
                                    from_version: None,
                                })).await {
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
                    None => {
                        // Connection closed — check close reason
                        match conn.close_reason() {
                            Some(CloseReason::CertificateRotated) => {
                                tracing::info!("connection closed: certificate rotated");
                                break LoopOutcome::Reconnect;
                            }
                            Some(CloseReason::CertificateRevoked) => {
                                tracing::warn!("connection closed: certificate revoked");
                                break LoopOutcome::Disconnected;
                            }
                            Some(reason) => {
                                tracing::warn!(%reason, "connection closed by controller");
                                break LoopOutcome::Disconnected;
                            }
                            None => {
                                tracing::info!("connection closed by controller");
                                break LoopOutcome::Disconnected;
                            }
                        }
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received SIGINT, initiating graceful shutdown");
                break handle_graceful_shutdown(
                    &mut conn,
                    in_flight_update.take(),
                    shutdown_timeout_seconds,
                    DisconnectReason::Shutdown,
                    LoopOutcome::Shutdown,
                ).await;
            }
            _ = async {
                #[cfg(unix)]
                {
                    sigterm.recv().await;
                }
                #[cfg(not(unix))]
                {
                    futures_util::future::pending::<()>().await;
                }
            } => {
                tracing::info!("received SIGTERM, initiating graceful shutdown");
                break handle_graceful_shutdown(
                    &mut conn,
                    in_flight_update.take(),
                    shutdown_timeout_seconds,
                    DisconnectReason::Shutdown,
                    LoopOutcome::Shutdown,
                ).await;
            }
            _ = async {
                #[cfg(unix)]
                {
                    sighup.recv().await;
                }
                #[cfg(not(unix))]
                {
                    futures_util::future::pending::<()>().await;
                }
            } => {
                tracing::info!("received SIGHUP, initiating graceful restart");
                break handle_graceful_shutdown(
                    &mut conn,
                    in_flight_update.take(),
                    shutdown_timeout_seconds,
                    DisconnectReason::Restart,
                    LoopOutcome::Restart,
                ).await;
            }
        }
    };

    // Best-effort close — the peer may have already disconnected.
    let _ = conn.close().await;

    Ok(outcome)
}

/// Events from an in-flight update.
enum UpdateEvent {
    Output(crate::update::UpdateOutputMessage),
    Completed(std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>),
}

/// Send the final update result to the controller.
async fn send_update_result(
    conn: &mut ControllerConnection,
    update_history_id: uuid::Uuid,
    result: std::result::Result<crate::update::UpdateExecutionResult, tokio::task::JoinError>,
) {
    match result {
        Ok(exec_result) => {
            conn.send_best_effort(ServiceMessage::UpdateResult(exec_result.result))
                .await;
            tracing::info!(update_id = %update_history_id, "update execution completed");
        }
        Err(e) => {
            tracing::error!(error = %e, "update task panicked");
            conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id,
                status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some(format!("Update task panicked: {e}")),
            }))
            .await;
        }
    }
}

/// Handle graceful shutdown sequence:
/// 1. Wait for in-flight update to complete (with timeout)
/// 2. Send Disconnecting message to controller
/// 3. Return the appropriate LoopOutcome
async fn handle_graceful_shutdown(
    conn: &mut ControllerConnection,
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
                    conn.send_best_effort(ServiceMessage::UpdateOutput(UpdateOutputPayload {
                        update_history_id: update.update_history_id,
                        output: output_msg.output,
                        stream: output_msg.stream,
                    })).await;
                }
                result = &mut update.handle => {
                    send_update_result(conn, update.update_history_id, result).await;
                    break;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!(
                        update_id = %update.update_history_id,
                        "shutdown timeout reached, abandoning in-flight update"
                    );
                    // Send a timeout failure result
                    conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                        update_history_id: update.update_history_id,
                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                        from_version: None,
                        to_version: None,
                        output: String::new(),
                        error: Some(format!("Agent shutdown timeout ({timeout_seconds}s) reached")),
                    })).await;
                    break;
                }
            }
        }

        // Drain any remaining output messages
        while let Ok(output_msg) = update.output_rx.try_recv() {
            conn.send_best_effort(ServiceMessage::UpdateOutput(UpdateOutputPayload {
                update_history_id: update.update_history_id,
                output: output_msg.output,
                stream: output_msg.stream,
            }))
            .await;
        }
    }

    // Send Disconnecting message to controller
    let disconnecting_msg =
        ServiceMessage::Disconnecting(DisconnectingPayload::new(disconnect_reason));
    if let Err(e) = conn.send(disconnecting_msg).await {
        tracing::debug!(error = %e, "failed to send Disconnecting message");
    } else {
        tracing::debug!(reason = ?disconnect_reason, "sent Disconnecting message to controller");
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
            uptrakit_shared_types::hex::encode(hasher.finalize())
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            uptrakit_shared_types::hex::encode(h.finalize())
        };
        assert_eq!(hash, expected);
    }
}
