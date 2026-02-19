use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::{
    CloseReason, ControllerMessage, DisconnectReason, DisconnectingPayload, HostInfo, PingPayload,
    ReportHostsPayload, ServiceMessage, now_millis,
};
use uptrakit_service_sdk::ca::{CaTlsMode, fetch_ca_certificate};
use uptrakit_service_sdk::{
    CertificateRenewalHandler, ControllerConnection, LoopOutcome, create_renewal_sleep,
    update_renewal_schedule,
};

use crate::error::{Error, Result};
use crate::host_info::collect_remote_host_info;
use crate::host_ops::list_hosts;
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig, SshSession};

/// Create a [`CommandExecutor`] that runs commands on a remote host via
/// the given SSH session.
///
/// This is the bridge between providers (transport-agnostic) and the SSH
/// transport. Called when processing `CheckVersions`/`ExecuteUpdate` wire
/// protocol messages for a specific host.
pub fn create_ssh_executor(session: Arc<SshSession>) -> Arc<dyn CommandExecutor> {
    Arc::new(SshCommandExecutor::new(session))
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
    pub state_dir: &'a std::path::Path,
}

/// Authenticated event loop (mTLS connection) with renewal timer.
///
/// Maintains the connection, handles certificate lifecycle, and keeps
/// the local database open for SSH operations. Version checks and
/// update execution over SSH will be added when the corresponding
/// wire protocol handlers are implemented.
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
        state_dir,
    } = params;

    const PING_INTERVAL: Duration = Duration::from_secs(300);
    const DEFAULT_SHUTDOWN_TIMEOUT: u32 = 120;

    tracing::info!("connecting to controller (authenticated)");
    let mut conn = ControllerConnection::connect(host, port, &tls_connector, None)
        .await
        .context_to::<Error>()?;

    // Open (or create) the local SSH host database.
    let local_db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(sea_orm::DbErr::Custom(format!(
            "failed to initialize local database: {e}"
        ))))
    })?;
    tracing::debug!("local SSH host database initialized");

    // Executor factory for per-host SSH command execution. Used when
    // handling CheckVersions/ExecuteUpdate messages (future work).
    let _executor_factory = create_ssh_executor;

    // ── Report enrolled hosts to controller ──────────────────────────
    report_enrolled_hosts(&local_db, &mut conn).await;

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context_to::<Error>()?;

    // First tick completes immediately, sending an initial ping on connect.
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await;

    // Renewal timer — initially far-future, reset when ServiceSettings arrives.
    let mut renewal_sleep = create_renewal_sleep();

    let mut cert_handler = CertificateRenewalHandler::new();

    let mut shutdown_timeout_seconds: u32 = DEFAULT_SHUTDOWN_TIMEOUT;

    // Clone directory paths to avoid borrow conflicts with `&mut identity`.
    let config_dir = identity.config_dir().to_path_buf();

    let outcome = loop {
        tokio::select! {
            biased;

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

                                // Check if CA bundle is stale.
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
                            ControllerMessage::ServerRestarting(payload) => {
                                tracing::info!(reason = %payload.reason, "controller is restarting");
                            }
                            _ => {
                                tracing::debug!("ignoring unrecognized message in authenticated loop");
                                continue;
                            }
                        }
                    }
                    None => {
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
                let disconnecting_msg =
                    ServiceMessage::Disconnecting(DisconnectingPayload::new(DisconnectReason::Shutdown));
                if let Err(e) = conn.send(disconnecting_msg).await {
                    tracing::debug!(error = %e, "failed to send Disconnecting message");
                } else {
                    tracing::debug!("sent Disconnecting message to controller");
                }
                break LoopOutcome::Shutdown;
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
                let disconnecting_msg =
                    ServiceMessage::Disconnecting(DisconnectingPayload::new(DisconnectReason::Shutdown));
                if let Err(e) = conn.send(disconnecting_msg).await {
                    tracing::debug!(error = %e, "failed to send Disconnecting message");
                } else {
                    tracing::debug!("sent Disconnecting message to controller");
                }
                break LoopOutcome::Shutdown;
            }
        }
    };

    let _ = shutdown_timeout_seconds; // used for future expansion

    // Best-effort close — the peer may have already disconnected.
    let _ = conn.close().await;

    Ok(outcome)
}

/// Connect to each enrolled SSH host, collect system info, and send a
/// `ReportHosts` message to the controller.
///
/// Errors for individual hosts are logged as warnings and skipped.
async fn report_enrolled_hosts(
    local_db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
) {
    let hosts = match list_hosts(local_db).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list SSH hosts for reporting");
            return;
        }
    };

    let mut host_infos: Vec<HostInfo> = Vec::with_capacity(hosts.len());

    for host in &hosts {
        tracing::debug!(host_name = %host.name, hostname = %host.hostname, "collecting host info");

        let config = SshConnectionConfig {
            hostname: host.hostname.clone(),
            port: host.port as u16,
            connect_timeout: Duration::from_secs(10),
        };

        let private_key_pem = host.private_key.expose_secret();
        let auth = AuthMethod::PrivateKey(private_key_pem);

        let (session, _fingerprint) = match crate::ssh_transport::connect_and_authenticate(
            &config,
            &host.username,
            &auth,
            host.host_key_fingerprint.as_deref(),
        )
        .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(
                    host_name = %host.name,
                    hostname = %host.hostname,
                    error = %e,
                    "failed to connect to SSH host for reporting, skipping"
                );
                continue;
            }
        };

        let mut info = collect_remote_host_info(&session).await;
        // Set the SSH target address as the host's ip_address.
        info.ip_address = Some(host.hostname.clone());

        session.disconnect().await;

        tracing::debug!(
            host_name = %host.name,
            machine_id = %info.machine_id,
            hostname = ?info.hostname,
            "collected remote host info"
        );

        host_infos.push(info);
    }

    let agent_version = env!("CARGO_PKG_VERSION").to_string();
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: host_infos,
        agent_version,
        protocol_version: uptrakit_internal_wire::PROTOCOL_VERSION,
    });

    if let Err(e) = conn.send(msg).await {
        tracing::warn!(error = %e, "failed to send ReportHosts message");
    } else {
        tracing::info!(
            host_count = hosts.len(),
            "reported enrolled hosts to controller"
        );
    }
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
        let expected = {
            let mut h = Sha256::new();
            h.update(b"test-ca-content");
            uptrakit_shared_types::hex::encode(h.finalize())
        };
        assert_eq!(hash, expected);
    }
}
