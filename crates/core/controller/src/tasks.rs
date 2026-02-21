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
pub struct BackgroundTasks {
    shutdown_token: CancellationToken,
    /// Tasks that respond to the [`CancellationToken`] and are awaited with a timeout.
    cancellable: Vec<(&'static str, JoinHandle<()>)>,
    /// Tasks that are forcefully aborted.
    abortable: Vec<(&'static str, JoinHandle<()>)>,
}

impl BackgroundTasks {
    pub fn new(shutdown_token: CancellationToken) -> Self {
        Self {
            shutdown_token,
            cancellable: Vec::new(),
            abortable: Vec::new(),
        }
    }

    /// Create a child token from the shared shutdown token.
    pub fn child_token(&self) -> CancellationToken {
        self.shutdown_token.child_token()
    }

    /// Register a task that listens for the [`CancellationToken`].
    pub fn track(&mut self, name: &'static str, handle: JoinHandle<()>) {
        self.cancellable.push((name, handle));
    }

    /// Register a task to be aborted on shutdown.
    pub fn track_abort(&mut self, name: &'static str, handle: JoinHandle<()>) {
        self.abortable.push((name, handle));
    }

    /// Execute the graceful shutdown sequence.
    ///
    /// 1. Stop accepting new HTTPS connections.
    /// 2. Scatter `ServerRestarting` notifications to connected agents.
    /// 3. Cancel all token-based background tasks.
    /// 4. Abort non-token tasks (CRL manager, PKI HTTP).
    /// 5. Await cancellable tasks with a per-task timeout.
    pub async fn shutdown(
        self,
        server_handle: axum_server::Handle<SocketAddr>,
        service_connections: uptrakit_web_api::service_connections::ServiceConnectionRegistry,
        shutdown_timeout: Duration,
    ) {
        tracing::info!("beginning graceful shutdown");

        // 1. Stop accepting new connections
        server_handle.graceful_shutdown(Some(shutdown_timeout));

        // 2. Scatter restart notifications to avoid thundering herd
        let connected_count = service_connections.connection_count().await;
        if connected_count > 0 {
            tracing::info!(
                connected_agents = connected_count,
                "sending server restarting notifications"
            );
            service_connections
                .broadcast_server_restarting_scattered(
                    uptrakit_internal_wire::ServerRestartingPayload {
                        reason: "controller restarting".to_string(),
                    },
                    durations::RESTART_NOTIFICATION_SCATTER,
                )
                .await;
            tokio::time::sleep(durations::RESTART_NOTIFICATION_SCATTER).await;
        }

        // 3. Cancel all token-based tasks
        self.shutdown_token.cancel();

        // 4. Abort tasks that don't use CancellationToken
        for (name, handle) in self.abortable {
            tracing::debug!("aborting {name}");
            handle.abort();
        }

        // 5. Wait for token-based tasks to complete
        tracing::debug!("waiting for background tasks to complete");
        for (name, handle) in self.cancellable {
            if tokio::time::timeout(durations::BACKGROUND_TASK_SHUTDOWN_TIMEOUT, handle)
                .await
                .is_err()
            {
                tracing::warn!("{name} did not complete within shutdown timeout");
            }
        }

        tracing::info!("graceful shutdown complete");
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
pub fn spawn_denylist_cleanup(
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
pub fn spawn_settings_reload(token: CancellationToken, app_state: Arc<AppState>) -> JoinHandle<()> {
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
pub fn spawn_ca_reload(
    token: CancellationToken,
    app_state: Arc<AppState>,
    ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    crl_manager: Arc<crate::crl_manager::CrlManager>,
    initial_ca_version: i64,
) -> JoinHandle<()> {
    let db = app_state.db().clone();
    let settings = app_state.settings.clone();
    let ca_key_store = Arc::clone(&app_state.ca_key_store);
    let tenant_id = app_state.default_tenant_id;

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

            let Ok(db_version) = crate::pki::load_ca_version(&db, tenant_id).await else {
                continue;
            };
            if db_version == cached_version {
                continue;
            }

            let state = match crate::pki::load_managed_ca_state(&db, tenant_id).await {
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

/// Cross-controller notification delivery via event polling.
pub fn spawn_event_poller(token: CancellationToken, app_state: Arc<AppState>) -> JoinHandle<()> {
    let event_poller = uptrakit_web_api::event_poller::EventPoller::new(
        app_state.db().clone(),
        app_state.service_connections.clone(),
        app_state.controller_id,
    );
    tokio::spawn(event_poller.run(token))
}

/// Trigger-based CA rotation for managed CAs.
///
/// Periodic CA rotation checking is handled by the centralised scheduler
/// (`CaRotationCheckExecutor`), which fires `ca_rotation_trigger` when rotation
/// is needed. This task only listens for that trigger and API-initiated rotations.
pub fn spawn_ca_rotation(
    token: CancellationToken,
    app_state: Arc<AppState>,
    ca_tx: tokio::sync::watch::Sender<crate::pki::CaSnapshot>,
    crl_manager: Arc<crate::crl_manager::CrlManager>,
) -> JoinHandle<()> {
    let db = app_state.db().clone();
    let settings = app_state.settings.clone();
    let ca_key_store = Arc::clone(&app_state.ca_key_store);
    let notification_service = app_state.notification_service.clone();
    let trigger = Arc::clone(&app_state.ca_rotation_trigger);
    let tenant_id = app_state.default_tenant_id;

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

            match crate::pki::rotate_managed_ca(
                &db,
                tenant_id,
                current_pki_addr.as_deref(),
                &expected_fp,
            )
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
pub fn spawn_server_cert_renewal(
    token: CancellationToken,
    app_state: Arc<AppState>,
    crl_manager: Arc<crate::crl_manager::CrlManager>,
) -> JoinHandle<()> {
    let pki_path = app_state.pki_path.clone();
    let ca_key_store = Arc::clone(&app_state.ca_key_store);

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
            let snapshot = app_state.ca_snapshot.borrow().clone();
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

            let extra_sans = app_state.settings.extra_sans();
            match crate::pki::renew_server_cert(&pki_path, &ca_bundle, &extra_sans).await {
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
