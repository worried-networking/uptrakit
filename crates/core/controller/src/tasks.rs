//! Background task spawning and coordinated shutdown.
//!
//! Each `spawn_*` function encapsulates one background task's async loop.
//! [`BackgroundTasks`] collects their handles and provides a single
//! [`shutdown`](BackgroundTasks::shutdown) method for orderly teardown.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uptrakit_web_api::AppState;

use crate::durations;

/// Holds all background task handles for orderly shutdown.
pub(crate) struct BackgroundTasks {
    shutdown_token: CancellationToken,
    /// Tasks that respond to the [`CancellationToken`] and are awaited with a per-task timeout.
    cancellable: Vec<(&'static str, JoinHandle<()>, Duration)>,
    /// Tasks that are forcefully aborted.
    abortable: Vec<(&'static str, JoinHandle<()>)>,
}

impl BackgroundTasks {
    pub(crate) fn new(shutdown_token: CancellationToken) -> Self {
        Self {
            shutdown_token,
            cancellable: Vec::new(),
            abortable: Vec::new(),
        }
    }

    /// Create a child token from the shared shutdown token.
    pub(crate) fn child_token(&self) -> CancellationToken {
        self.shutdown_token.child_token()
    }

    /// Register a task that listens for the [`CancellationToken`].
    ///
    /// Uses [`durations::BACKGROUND_TASK_SHUTDOWN_TIMEOUT`] as the shutdown timeout.
    /// For tasks that need more time (e.g. the scheduler), use [`track_with_timeout`](Self::track_with_timeout).
    pub(crate) fn track(&mut self, name: &'static str, handle: JoinHandle<()>) {
        self.cancellable
            .push((name, handle, durations::BACKGROUND_TASK_SHUTDOWN_TIMEOUT));
    }

    /// Register a task to be aborted on shutdown.
    pub(crate) fn track_abort(&mut self, name: &'static str, handle: JoinHandle<()>) {
        self.abortable.push((name, handle));
    }

    /// Execute the graceful shutdown sequence.
    ///
    /// 1. Stop accepting new HTTPS connections.
    /// 2. Scatter `ServerRestarting` notifications to connected agents, then wait for drain.
    /// 3. Cancel all token-based background tasks.
    /// 4. Abort non-token tasks (CRL manager, PKI HTTP).
    /// 5. Await cancellable tasks with a per-task timeout.
    pub(crate) async fn shutdown(
        self,
        server_handle: axum_server::Handle<SocketAddr>,
        service_connections: uptrakit_web_api::service_connections::ServiceConnectionRegistry,
        shutdown_timeout: Duration,
    ) {
        tracing::info!("beginning graceful shutdown");

        // 1. Stop accepting new connections
        tracing::debug!("stopping HTTP server (no new connections accepted)");
        server_handle.graceful_shutdown(Some(shutdown_timeout));

        // 2. Scatter restart notifications, then wait for services to disconnect.
        //    `broadcast_server_restarting_scattered` returns immediately after
        //    scheduling the per-service delayed sends; the actual sends happen
        //    in the background while `wait_for_service_drain` polls below.
        let connected_count = service_connections.connection_count().await;
        if connected_count > 0 {
            tracing::info!(
                connected = connected_count,
                scatter_window_secs = durations::RESTART_NOTIFICATION_SCATTER.as_secs(),
                "sending ServerRestarting notifications to connected services"
            );
            service_connections
                .broadcast_server_restarting_scattered(
                    uptrakit_internal_wire::ServerRestartingPayload {
                        reason: "controller restarting".to_string(),
                    },
                    durations::RESTART_NOTIFICATION_SCATTER,
                )
                .await;
            tracing::debug!(
                count = connected_count,
                timeout_secs = shutdown_timeout.as_secs(),
                "waiting for services to disconnect gracefully"
            );
            wait_for_service_drain(&service_connections, shutdown_timeout).await;
        } else {
            tracing::info!("no services connected, skipping service drain");
        }

        // 3. Cancel all token-based tasks
        tracing::debug!(
            cancellable = self.cancellable.len(),
            abortable = self.abortable.len(),
            "cancelling background tasks"
        );
        self.shutdown_token.cancel();

        // 4. Abort tasks that don't use CancellationToken
        for (name, handle) in self.abortable {
            tracing::debug!("aborting {name}");
            handle.abort();
        }

        // 5. Wait for token-based tasks to complete
        tracing::debug!("waiting for background tasks to complete");
        for (name, handle, task_timeout) in self.cancellable {
            tracing::trace!(
                task = name,
                timeout_secs = task_timeout.as_secs(),
                "awaiting task shutdown"
            );
            if tokio::time::timeout(task_timeout, handle).await.is_err() {
                tracing::warn!(
                    timeout_secs = task_timeout.as_secs(),
                    "{name} did not complete within shutdown timeout"
                );
            } else {
                tracing::trace!(task = name, "task shut down cleanly");
            }
        }

        tracing::info!("graceful shutdown complete");
    }
}

// ---------------------------------------------------------------------------
// Service drain helper
// ---------------------------------------------------------------------------

/// Poll `service_connections` until all services have disconnected or `shutdown_timeout` elapses.
///
/// Logs progress at every change in the connection count and emits a warning if the timeout is
/// reached before all services have disconnected.
pub(crate) async fn wait_for_service_drain(
    service_connections: &uptrakit_web_api::service_connections::ServiceConnectionRegistry,
    shutdown_timeout: Duration,
) {
    use crate::durations::SERVICE_DRAIN_POLL_INTERVAL;

    let start = tokio::time::Instant::now();
    let deadline = tokio::time::sleep(shutdown_timeout);
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(SERVICE_DRAIN_POLL_INTERVAL);
    // First tick fires immediately; skip it so we don't check before any service has had a chance
    // to process the notification.
    poll.tick().await;

    let mut last_count = service_connections.connection_count().await;

    loop {
        tokio::select! {
            biased;
            _ = &mut deadline => {
                let remaining = service_connections.connection_count().await;
                tracing::warn!(
                    remaining,
                    timeout_secs = shutdown_timeout.as_secs(),
                    "graceful shutdown timeout reached, forcing"
                );
                break;
            }
            _ = poll.tick() => {
                let count = service_connections.connection_count().await;
                if count != last_count {
                    let disconnected = last_count.saturating_sub(count);
                    tracing::info!(
                        disconnected,
                        remaining = count,
                        "services disconnected during graceful shutdown"
                    );
                    last_count = count;
                }
                if count == 0 {
                    let elapsed = start.elapsed();
                    tracing::info!(
                        elapsed_secs = format!("{:.1}", elapsed.as_secs_f64()),
                        "all services disconnected, proceeding with shutdown"
                    );
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Individual background task spawn functions
// ---------------------------------------------------------------------------

/// Periodic purge of the in-memory token denylist.
///
/// This is per-instance (not in the centralised scheduler) because the denylist
/// is an in-memory data structure. DB-backed auth store cleanup is handled by the
/// scheduler's `AuthCleanupExecutor`.
pub(crate) fn spawn_denylist_cleanup(
    token: CancellationToken,
    token_denylist: Arc<uptrakit_web_api::auth::token_denylist::TokenDenylist>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(durations::AUTH_CLEANUP_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    token_denylist.purge_expired().await;
                }
                _ = token.cancelled() => {
                    tracing::debug!("token denylist cleanup task shutting down");
                    break;
                }
            }
        }
    })
}

/// Periodic settings version check for cross-instance cache invalidation.
pub(crate) fn spawn_settings_reload(
    token: CancellationToken,
    app_state: Arc<AppState>,
) -> JoinHandle<()> {
    let settings = app_state.settings.clone();
    let db = app_state.db().clone();
    let tid = app_state.default_tenant_id;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(durations::SETTINGS_POLL_INTERVAL);
        // Skip the first immediate tick — settings were just loaded
        interval.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match settings.check_version_and_reload(&db, tid).await {
                        Ok(true) => tracing::info!("settings reloaded from database (version changed)"),
                        Ok(false) => tracing::debug!("settings version unchanged"),
                        Err(e) => tracing::warn!(error = ?e, "periodic settings version check failed"),
                    }
                }
                _ = token.cancelled() => {
                    tracing::debug!("settings reload task shutting down");
                    break;
                }
            }
        }
    })
}

/// Polls the CA version counter in the database to detect cross-instance CA updates.
pub(crate) fn spawn_ca_reload(
    token: CancellationToken,
    app_state: Arc<AppState>,
    ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    crl_manager: Arc<crate::crl_manager::CrlManager>,
    initial_ca_version: i64,
) -> JoinHandle<()> {
    let db = app_state.db().clone();
    let settings = app_state.settings.clone();
    let ca_key_store = Arc::clone(&app_state.cert.ca_key_store);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(durations::SETTINGS_POLL_INTERVAL);
        let mut cached_version = initial_ca_version;
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = token.cancelled() => {
                    tracing::debug!("CA reload task shutting down");
                    break;
                }
            }

            let Ok(db_version) = crate::pki::load_ca_version(&db).await else {
                continue;
            };
            if db_version == cached_version {
                continue;
            }

            let state = match crate::pki::load_managed_ca_state(&db).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = ?e, "failed to reload CA state from database");
                    continue;
                }
            };

            let pki_addr = settings.pki_addr();
            let (snapshot, new_key_store) = match state.to_snapshot(pki_addr) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = ?e, "failed to build CA snapshot after reload");
                    continue;
                }
            };

            if let Err(e) = crl_manager.update_ca(&snapshot, &new_key_store).await {
                tracing::error!(error = ?e, "failed to update CRL manager after CA reload");
                continue;
            }

            // Update the shared key store
            *ca_key_store.write().await = new_key_store;
            let _ = ca_tx.send(snapshot);

            if let Err(e) = crl_manager.reload_tls_config().await {
                tracing::error!(error = ?e, "failed to reload TLS after CA reload");
            }

            cached_version = db_version;
        }
    })
}

/// Trigger-based CA rotation for managed CAs.
///
/// Periodic CA rotation checking is handled by the centralised scheduler
/// (`CaRotationCheckExecutor`), which fires `ca_rotation_trigger` when rotation
/// is needed. This task only listens for that trigger and API-initiated rotations.
pub(crate) fn spawn_ca_rotation(
    token: CancellationToken,
    app_state: Arc<AppState>,
    ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    crl_manager: Arc<crate::crl_manager::CrlManager>,
) -> JoinHandle<()> {
    let db = app_state.db().clone();
    let settings = app_state.settings.clone();
    let ca_key_store = Arc::clone(&app_state.cert.ca_key_store);
    let notification_service = app_state.notification_service.clone();
    let trigger = Arc::clone(&app_state.cert.ca_rotation_trigger);

    tokio::spawn(async move {
        loop {
            // Wait for a rotation trigger (from scheduler or API) or shutdown
            tokio::select! {
                () = trigger.notified() => {}
                _ = token.cancelled() => {
                    tracing::debug!("CA rotation task shutting down");
                    return;
                }
            };

            tracing::info!("CA rotation triggered");

            let current_pki_addr = settings.pki_addr();
            let snapshot = ca_tx.borrow().clone();
            let expected_fp = snapshot.active_fingerprint.clone();

            match crate::pki::rotate_managed_ca(&db, current_pki_addr.as_deref(), &expected_fp)
                .await
            {
                Ok(rotation) => {
                    if !rotation.rotated {
                        tracing::info!("CA rotation skipped (another controller already rotated)");
                        continue;
                    }

                    let rotation_pki_addr = current_pki_addr.clone();
                    match rotation.state.to_snapshot(rotation_pki_addr) {
                        Ok((new_snapshot, new_key_store)) => {
                            if let Err(e) =
                                crl_manager.update_ca(&new_snapshot, &new_key_store).await
                            {
                                tracing::error!(error = ?e, "failed to update CRL manager after CA rotation");
                                continue;
                            }

                            // Update the shared key store
                            *ca_key_store.write().await = new_key_store;

                            // Broadcast CA bundle update to all connected services
                            let ca_payload = uptrakit_internal_wire::CaBundleUpdatedPayload {
                                ca_bundle_pem: new_snapshot.bundle_pem.clone(),
                            };
                            notification_service
                                .broadcast(
                                    uptrakit_internal_wire::ControllerMessage::CaBundleUpdated(
                                        ca_payload,
                                    ),
                                )
                                .await;

                            // Request all services to renew their certificates
                            let renewal_payload =
                                uptrakit_internal_wire::RequestCertRenewalPayload {
                                    reason: "CA rotation".to_string(),
                                };
                            notification_service
                                .broadcast(
                                    uptrakit_internal_wire::ControllerMessage::RequestCertRenewal(
                                        renewal_payload,
                                    ),
                                )
                                .await;

                            let _ = ca_tx.send(new_snapshot);

                            if let Err(e) = crl_manager.reload_tls_config().await {
                                tracing::error!(
                                    error = ?e,
                                    "failed to reload TLS after CA rotation"
                                );
                            }

                            tracing::info!("CA rotation completed successfully");
                        }
                        Err(e) => {
                            tracing::error!(
                                error = ?e,
                                "failed to build snapshot after CA rotation"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = ?e, "CA rotation failed");
                }
            }
        }
    })
}

/// Periodic server certificate auto-renewal (internally generated certs only).
pub(crate) fn spawn_server_cert_renewal(
    token: CancellationToken,
    app_state: Arc<AppState>,
    crl_manager: Arc<crate::crl_manager::CrlManager>,
) -> JoinHandle<()> {
    let pki_path = app_state.pki_path.clone();
    let ca_key_store = Arc::clone(&app_state.cert.ca_key_store);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(durations::SERVER_CERT_RENEWAL_CHECK_INTERVAL);
        // Skip the first immediate tick
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = token.cancelled() => {
                    tracing::debug!("server cert renewal task shutting down");
                    return;
                }
            }

            tracing::debug!("checking server certificate renewal status");

            // Read current server cert from disk
            let cert_path = pki_path.join("server.crt");
            let Ok(cert_pem) = std::fs::read_to_string(&cert_path) else {
                continue;
            };

            if !crate::pki::should_renew_server_cert(&cert_pem) {
                continue;
            }

            tracing::info!("server certificate is within renewal window, renewing");

            // Get current active CA from watch channel and key store
            let snapshot = app_state.cert.ca_snapshot.borrow().clone();
            let key_store = ca_key_store.read().await;

            // Build a temporary CaBundle for renewal
            let ca_key = match rcgen::KeyPair::from_pem(&key_store.active_key_pem) {
                Ok(k) => k,
                Err(e) => {
                    tracing::error!(error = %e, "failed to parse CA key for server cert renewal");
                    continue;
                }
            };
            let ca_issuer = match rcgen::Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, ca_key)
            {
                Ok(i) => i,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "failed to create CA issuer for server cert renewal"
                    );
                    continue;
                }
            };

            let ca_bundle = crate::pki::CaBundle {
                cert_pem: snapshot.active_cert_pem.clone(),
                key_pem: key_store.active_key_pem.to_string(),
                issuer: ca_issuer,
            };
            // Drop the key store lock before proceeding with I/O
            drop(key_store);

            let sans = app_state.settings.sans();
            match crate::pki::renew_server_cert(&pki_path, &ca_bundle, &sans).await {
                Ok(new_cert) => {
                    crl_manager
                        .update_server_cert(new_cert.cert_pem.clone(), new_cert.key_pem.clone())
                        .await;

                    if let Err(e) = crl_manager.reload_tls_config().await {
                        tracing::error!(
                            error = ?e,
                            "failed to reload TLS after server cert renewal"
                        );
                    }

                    tracing::info!("server certificate auto-renewed successfully");
                }
                Err(e) => {
                    tracing::error!(error = ?e, "server certificate renewal failed");
                }
            }
        }
    })
}

/// NATS consumer for cross-controller event delivery.
///
/// Pulls messages from JetStream, filters self-originated events, and delivers
/// to local services via the shared event delivery routing logic.
#[cfg(feature = "nats")]
pub(crate) fn spawn_nats_consumer(
    token: CancellationToken,
    nats: uptrakit_web_api::nats_transport::NatsTransport,
    config: uptrakit_web_api::nats_transport::NatsConsumerConfig,
) -> JoinHandle<()> {
    tokio::spawn(nats.run_consumer(config, token))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use uptrakit_web_api::service_connections::ServiceConnectionRegistry;
    use uuid::Uuid;

    use super::wait_for_service_drain;

    /// Early exit: the drain loop should return as soon as the last service disconnects,
    /// well before the full `shutdown_timeout` elapses.
    #[tokio::test(start_paused = true)]
    async fn drain_exits_early_when_services_disconnect() {
        let registry = ServiceConnectionRegistry::new();
        let service_id = Uuid::nil();
        registry
            .register(service_id, BTreeSet::new(), None, None)
            .await;

        // Unregister the service after 5 s (simulated).
        let registry_clone = registry.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            registry_clone.unregister(&service_id).await;
        });

        let start = tokio::time::Instant::now();
        wait_for_service_drain(&registry, Duration::from_secs(30)).await;
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_secs(5),
            "should have waited at least 5 s, elapsed = {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(30),
            "should have exited before the 30 s timeout, elapsed = {elapsed:?}"
        );
    }

    /// Timeout path: when no service disconnects the loop should wait exactly the timeout duration.
    #[tokio::test(start_paused = true)]
    async fn drain_waits_full_timeout_when_service_never_disconnects() {
        let registry = ServiceConnectionRegistry::new();
        let service_id = Uuid::nil();
        registry
            .register(service_id, BTreeSet::new(), None, None)
            .await;

        let start = tokio::time::Instant::now();
        wait_for_service_drain(&registry, Duration::from_secs(5)).await;
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_secs(5),
            "should have waited the full 5 s timeout, elapsed = {elapsed:?}"
        );
    }
}
