mod cli;
mod host_cli;

use clap::Parser;
use rootcause::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

pub(crate) use uptrakit_agent_ssh::{
    HOST_RELOAD_INTERVAL, UPDATE_COOLDOWN, client, db, diff_host_snapshots, error, extension,
    handle_set_update_freeze, host_ops, init_ssh_data_key_ring, operations, reencrypt_ssh_to_v3,
    register_ssh_column_aad, ssh_key, ssh_pool,
};
use uptrakit_internal_wire::{Capability, ControllerMessage, RegisterPayload, ServiceMessage};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
    ShutdownCause, default_resolve_shutdown,
};

use cli::{Args, Commands};

// ---------------------------------------------------------------------------
// Typed error for initialization helpers
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum InitError {
    #[error("{0}")]
    Directory(String),
    #[error("{0}")]
    MasterKey(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Hex(String),
}

type InitResult<T> = std::result::Result<T, rootcause::Report<InitError>>;

// ---------------------------------------------------------------------------
// Service event enum
// ---------------------------------------------------------------------------

/// Events produced by the SSH agent's service loop.
///
/// Extends `client::UpdateEvent` with a `host_machine_id` tag and a
/// host-config-changed trigger so all internal events flow through the same
/// `poll_service_event` / `on_service_event` contract.
enum SshAgentEvent {
    /// Progress from an in-flight update task (output line or completion).
    ///
    /// The first field is the `host_machine_id` that identifies which entry in
    /// `SshAgentHandler::in_flight_updates` the event belongs to.
    Update(String, client::UpdateEvent),
    /// The host-config reload ticker fired; the handler will diff the DB
    /// snapshot and send `ReportHosts` if anything changed.
    HostConfigChanged,
    /// A background operation (discovery, version check, batch update)
    /// completed and produced a [`ServiceMessage`] that should be forwarded to
    /// the controller.
    ///
    /// Long-running operations are spawned as tokio tasks so they do not block
    /// the event loop. This variant delivers the result back into the loop for
    /// sending.
    BackgroundResult(uptrakit_internal_wire::ServiceMessage),
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

struct SshAgentHandler {
    local_db: Option<sea_orm::DatabaseConnection>,
    /// State directory path for the SSH agent.
    state_dir: std::path::PathBuf,
    /// Service UUID assigned by the controller (populated in `on_connected`).
    service_id: Option<uuid::Uuid>,
    /// Tenant UUID that this service belongs to (populated from `ServiceSettings`).
    ///
    /// Used for tenant-aware external provisioning such as PVE API credential
    /// naming (`uptrakit-{tenant_id}@pve`).
    tenant_id: Option<uuid::Uuid>,
    /// PKCS#8 DER-encoded P-256 private key for ECIES decryption of sensitive
    /// extension parameters. Populated in `on_connected` from the identity.
    private_key_der: Option<Vec<u8>>,
    /// Base64-encoded uncompressed P-256 public key for the `ExtensionRegister`
    /// payload. Populated in `on_connected` and sent in `on_settings` after
    /// capability negotiation confirms `UiExtensions` is in the agreed set.
    encryption_public_key: Option<String>,
    /// Path to the operator-controlled freeze file.
    ///
    /// When this file exists, the agent rejects all `ExecuteUpdate` and
    /// `ExecuteBatchUpdate` messages without executing them.
    /// Operators can create the file with `touch <path>` to halt update
    /// execution from the agent side, independent of the controller.
    ///
    /// Default path: `<state-dir>/update-freeze`.
    freeze_file_path: std::path::PathBuf,
    /// Per-host in-flight update state, keyed by `host_machine_id`.
    ///
    /// The architectural invariant is **no overlapping update actions per
    /// host**: two concurrent updates for the same host are forbidden, but
    /// different hosts may update simultaneously.
    in_flight_updates: HashMap<String, client::SshInFlightUpdate>,
    /// Receiving end of the aggregate event channel.
    ///
    /// Each update's forwarder task holds a clone of `aggregate_tx` and sends
    /// `(host_machine_id, UpdateEvent)` tuples here.  The service loop drains
    /// this channel in `poll_service_event`.
    aggregate_rx: tokio::sync::mpsc::Receiver<(String, client::UpdateEvent)>,
    /// Sending end of the aggregate event channel, cloned into each forwarder.
    aggregate_tx: tokio::sync::mpsc::Sender<(String, client::UpdateEvent)>,
    pool: ssh_pool::SshConnectionPool,
    /// Periodic ticker for host-config change detection.
    ///
    /// `None` until the first successful `on_connected`; reset on every
    /// reconnect so the first tick fires `HOST_RELOAD_INTERVAL` after connect,
    /// not sooner.
    reload_ticker: Option<tokio::time::Interval>,
    /// Last-known snapshot of `(id, updated_at)` pairs from `ssh_hosts`.
    ///
    /// Used to detect additions, removals, and updates without a full model
    /// load on every tick.  Populated in `on_connected` after
    /// `report_enrolled_hosts` completes.
    host_snapshot: Vec<host_ops::HostSnapshot>,
    /// Per-host timestamp of the last accepted update execution, for rate
    /// limiting. Keyed by `host_machine_id`.
    last_update_per_host: HashMap<String, std::time::Instant>,
    /// Receiving end of the background-result channel.
    ///
    /// Background tasks (discovery, version checks, batch updates) send their
    /// completed [`ServiceMessage`] here so the event loop can forward them to
    /// the controller without blocking on long-running SSH operations.
    bg_rx: tokio::sync::mpsc::Receiver<uptrakit_internal_wire::ServiceMessage>,
    /// Sending end of the background-result channel, cloned into each spawned
    /// background task.
    bg_tx: tokio::sync::mpsc::Sender<uptrakit_internal_wire::ServiceMessage>,
    /// Proxy for service-initiated extension action invocations.
    ///
    /// Enables the SSH agent to invoke controller-side plugin actions (e.g.,
    /// Proxmox plugin's `list-all-unmatched` for discovered guest bootstrap).
    extension_proxy: std::sync::Arc<uptrakit_service_sdk::ServiceExtensionProxy>,
    /// Registry of agent-side infrastructure plugins.
    infra_plugins:
        std::sync::Arc<Vec<std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::PluginBase>>>,
    /// Ensures the initial `ReportHosts` runs only after the first
    /// `ServiceSettings` so pagination honors controller-provided limits.
    pending_initial_host_report: bool,
}

impl SshAgentHandler {
    /// Await the next event from the aggregate update channel.
    ///
    /// Returns `pending()` when no updates are in-flight, so the `select!` in
    /// `poll_service_event` can safely park this arm without polling an empty
    /// channel.  Once at least one update is running, any event produced by any
    /// forwarder task will wake this future.
    async fn poll_updates(
        aggregate_rx: &mut tokio::sync::mpsc::Receiver<(String, client::UpdateEvent)>,
        in_flight_updates: &HashMap<String, client::SshInFlightUpdate>,
    ) -> (String, client::UpdateEvent) {
        if in_flight_updates.is_empty() {
            std::future::pending().await
        } else {
            match aggregate_rx.recv().await {
                Some(event) => event,
                // Channel closed — should never happen while forwarders are alive,
                // but park indefinitely rather than busy-looping.
                None => std::future::pending().await,
            }
        }
    }

    /// Wait for the next reload ticker tick.
    ///
    /// Returns `pending()` when the ticker has not yet been initialized (i.e.
    /// before the first `on_connected`).
    async fn poll_reload_tick(ticker: &mut Option<tokio::time::Interval>) -> tokio::time::Instant {
        if let Some(t) = ticker {
            t.tick().await
        } else {
            std::future::pending::<tokio::time::Instant>().await
        }
    }
}

#[async_trait::async_trait]
impl ServiceHandler for SshAgentHandler {
    const DIR_NAME: &'static str = "agent-ssh";
    const SERVICE_LABEL: &'static str = "uptrakit-agent-ssh service";
    const SERVICE_APP_NAME: &'static str = env!("CARGO_PKG_NAME");

    type ServiceEvent = SshAgentEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        // Declare capabilities immediately so the controller can set session
        // flags correctly even on first connect (before DB has stored caps).
        conn.send(ServiceMessage::Register(RegisterPayload::new(
            client::ssh_agent_capabilities(),
        )))
        .await
        .context_to::<LoopError>()?;

        // Store identity state for extension use.
        self.service_id = identity.service_id();
        self.private_key_der = identity.private_key_pkcs8_der();

        // Store the encryption public key; the actual ExtensionRegister message
        // is sent from on_settings once the agreed capabilities are known.
        self.encryption_public_key = identity.public_key_raw().map(|bytes| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        });
        self.pending_initial_host_report = true;
        self.host_snapshot.clear();
        self.reload_ticker = None;

        Ok(())
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        let db = self.local_db.as_ref().ok_or_else(|| {
            report!(LoopError::Other(
                "local_db not initialized: on_connected must be called before on_message"
                    .to_string()
            ))
        })?;
        match msg {
            ControllerMessage::CheckVersions(payload) => {
                client::spawn_check_versions_ssh(payload, db, &self.pool, &self.bg_tx);
                Ok(None)
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                if !self.is_update_allowed(&payload.host_machine_id).await {
                    return Ok(None);
                }
                self.last_update_per_host
                    .insert(payload.host_machine_id.clone(), std::time::Instant::now());
                client::handle_execute_update_ssh(
                    *payload,
                    db,
                    &mut self.in_flight_updates,
                    &self.aggregate_tx,
                    conn,
                    &self.pool,
                )
                .await;
                Ok(None)
            }
            ControllerMessage::ExecuteBatchUpdate(payload) => {
                if !self.is_update_allowed(&payload.host_machine_id).await {
                    return Ok(None);
                }
                self.last_update_per_host
                    .insert(payload.host_machine_id.clone(), std::time::Instant::now());
                client::spawn_execute_batch_update_ssh(*payload, db, &self.pool, &self.bg_tx);
                Ok(None)
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                client::spawn_discover_software_ssh(payload, db, &self.pool, &self.bg_tx);
                Ok(None)
            }
            ControllerMessage::SetUpdateFreeze(payload) => {
                handle_set_update_freeze(&self.freeze_file_path, payload).await;
                Ok(None)
            }
            ControllerMessage::TestPluginConfig(payload) => {
                client::spawn_config_test_ssh(payload, db, &self.pool, &self.bg_tx);
                Ok(None)
            }
            #[cfg(feature = "interactive")]
            ControllerMessage::UpdateStdinData(payload) => {
                client::handle_update_stdin_data_ssh(payload, &self.in_flight_updates);
                Ok(None)
            }
            ControllerMessage::ReportPluginConfigResponse(payload) => {
                self.handle_report_plugin_config_response(payload, db).await;
                Ok(None)
            }
            ControllerMessage::ResetData => {
                let db = db.clone();
                self.handle_reset_data(&db).await;
                Ok(None)
            }
            _ => {
                tracing::debug!("ignoring unrecognized message in authenticated loop");
                Ok(None)
            }
        }
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        // Borrow separate fields by name — Rust's field-projection rules allow
        // all borrows simultaneously, sidestepping a double-borrow of `self`.
        tokio::select! {
            biased;
            (host_machine_id, event) = Self::poll_updates(
                &mut self.aggregate_rx,
                &self.in_flight_updates,
            ) => {
                SshAgentEvent::Update(host_machine_id, event)
            }
            Some(msg) = self.bg_rx.recv() => {
                SshAgentEvent::BackgroundResult(msg)
            }
            _ = Self::poll_reload_tick(&mut self.reload_ticker) => {
                SshAgentEvent::HostConfigChanged
            }
        }
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match event {
            SshAgentEvent::Update(host_machine_id, update_event) => {
                let Some(update) = self.in_flight_updates.get(&host_machine_id) else {
                    tracing::error!(
                        %host_machine_id,
                        "received update event but no in-flight update found for this host"
                    );
                    return Ok(None);
                };
                let update_history_id = update.update_history_id;

                match update_event {
                    client::UpdateEvent::Output(output_msg) => {
                        client::send_update_output(conn, update_history_id, output_msg).await;
                    }
                    client::UpdateEvent::Completed(result) => {
                        if let Err(e) =
                            client::send_update_result(conn, update_history_id, result).await
                        {
                            tracing::error!(error = %e, "failed to send UpdateResult; disconnecting");
                            self.in_flight_updates.remove(&host_machine_id);
                            return Ok(Some(LoopOutcome::Disconnected));
                        }
                        self.in_flight_updates.remove(&host_machine_id);
                    }
                    client::UpdateEvent::Attention(uid) => {
                        conn.send_best_effort(
                            uptrakit_internal_wire::ServiceMessage::StdinAttention(
                                uptrakit_internal_wire::StdinAttentionPayload::new(uid),
                            ),
                        )
                        .await;
                    }
                }
                Ok(None)
            }

            SshAgentEvent::HostConfigChanged => {
                self.handle_host_config_changed(conn).await;
                Ok(None)
            }

            SshAgentEvent::BackgroundResult(msg) => {
                if let Some(outcome) = uptrakit_agent_core::send_background_result(conn, msg).await
                {
                    return Ok(Some(outcome));
                }
                Ok(None)
            }
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        client::ssh_agent_capabilities()
    }

    async fn on_settings(
        &mut self,
        settings: &uptrakit_internal_wire::ServiceSettingsPayload,
        conn: &mut ControllerConnection,
    ) {
        if self.pending_initial_host_report {
            let Some(local_db) = self.local_db.as_ref() else {
                tracing::warn!(
                    "local_db not initialized during on_settings; skipping initial SSH host report"
                );
                return;
            };

            client::report_enrolled_hosts(local_db, conn, &self.pool).await;

            self.host_snapshot = match host_ops::list_host_snapshots(local_db).await {
                Ok(snapshot) => snapshot,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to initialize host snapshot; dynamic reload will trigger \
                         a full re-report on the first tick"
                    );
                    Vec::new()
                }
            };

            let start = tokio::time::Instant::now() + HOST_RELOAD_INTERVAL;
            let mut ticker = tokio::time::interval_at(start, HOST_RELOAD_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            self.reload_ticker = Some(ticker);
            self.pending_initial_host_report = false;

            // Re-announce any updates that were in-flight before the disconnect.
            // The controller tracks update state by `UpdateStarted` → `UpdateResult`;
            // if the WS drops between those two messages the update stays in `pending`
            // forever. Re-sending `UpdateStarted` on reconnect lets the controller
            // transition it to `in_progress` correctly before the result arrives.
            for (host_machine_id, update) in &self.in_flight_updates {
                #[allow(unused_assignments, unused_mut)]
                let mut interactive = false;
                #[cfg(feature = "interactive")]
                {
                    interactive = update.stdin_tx.is_some();
                }

                tracing::debug!(
                    %host_machine_id,
                    update_history_id = %update.update_history_id,
                    "re-sending UpdateStarted on reconnect for in-flight update"
                );
                conn.send_best_effort(uptrakit_internal_wire::ServiceMessage::UpdateStarted(
                    uptrakit_internal_wire::UpdateStartedPayload {
                        update_history_id: update.update_history_id,
                        from_version: None,
                        interactive,
                    },
                ))
                .await;
            }
        }

        // Store tenant_id for PVE credential provisioning and persist it
        // to service.json so CLI commands can read it without connecting to
        // the controller.
        if let Some(tid) = settings.tenant_id {
            self.tenant_id = Some(tid);

            let state_dir = self.state_dir.clone();
            tokio::spawn(async move {
                let mut identity = ServiceIdentityState::new_single_dir(&state_dir);
                if let Err(e) = identity.load().await {
                    tracing::warn!(error = %e, "failed to load identity for tenant_id persistence");
                    return;
                }
                if let Err(e) = identity.save_tenant_id(tid).await {
                    tracing::warn!(error = %e, "failed to persist tenant_id to service.json");
                }
            });
        }

        // Register UI extensions only when the agreed capability set includes
        // UiExtensions. The controller refreshes its gating flags from the
        // Register message before delivering ServiceSettings, so the controller
        // has already updated its flags by the time ExtensionRegister is received.
        if conn
            .agreed_capabilities()
            .contains(&Capability::UiExtensions)
        {
            let register_payload = extension::build_register_payload(
                self.encryption_public_key.clone(),
                &self.infra_plugins,
            );
            if let Err(e) = conn
                .send(uptrakit_internal_wire::ServiceMessage::ExtensionRegister(
                    register_payload,
                ))
                .await
            {
                tracing::warn!(error = %e, "failed to register UI extensions");
            }

            // Register the action library (separate from manifests).
            let actions_payload = uptrakit_internal_wire::extension::ExtensionActionsPayload::new(
                extension::build_actions(),
            );
            if let Err(e) = conn
                .send(
                    uptrakit_internal_wire::ServiceMessage::ExtensionActionsRegister(
                        actions_payload,
                    ),
                )
                .await
            {
                tracing::warn!(error = %e, "failed to register extension actions");
            }
        }
    }

    fn on_extension_response(
        &mut self,
        response: uptrakit_internal_wire::extension::ExtensionResponsePayload,
    ) {
        let request_id = response.request_id.clone();
        self.extension_proxy.complete(&request_id, response);
    }

    async fn on_extension_request(
        &mut self,
        request: uptrakit_internal_wire::extension::ExtensionRequestPayload,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        let db = self.local_db.as_ref().ok_or_else(|| {
            report!(LoopError::Other(
                "local_db not initialized: on_connected must be called before on_extension_request"
                    .to_string()
            ))
        })?;

        let ctx = extension::ExtensionContext {
            db,
            state_dir: &self.state_dir,
            private_key_der: self.private_key_der.as_deref(),
            service_id: self.service_id,
            tenant_id: self.tenant_id,
            bg_tx: &self.bg_tx,
            extension_proxy: &self.extension_proxy,
            infra_plugins: std::sync::Arc::clone(&self.infra_plugins),
        };
        extension::handle_extension_request(request, &ctx, conn).await;

        Ok(())
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        shutdown_timeout: Duration,
    ) -> LoopOutcome {
        use uptrakit_internal_wire::{
            DisconnectingPayload, ServiceMessage, UpdateFinalStatus, UpdateResultPayload,
        };

        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);

        if !self.in_flight_updates.is_empty() {
            let count = self.in_flight_updates.len();
            tracing::info!(
                count,
                timeout = ?shutdown_timeout,
                "waiting for in-flight updates to complete before shutdown"
            );

            let deadline = tokio::time::Instant::now() + shutdown_timeout;

            while !self.in_flight_updates.is_empty() {
                tokio::select! {
                    biased;
                    Some((host_id, event)) = self.aggregate_rx.recv() => {
                        if let Some(update) = self.in_flight_updates.get(&host_id) {
                            let uid = update.update_history_id;
                            match event {
                                client::UpdateEvent::Output(msg) => {
                                    client::send_update_output(conn, uid, msg).await;
                                }
                                client::UpdateEvent::Completed(result) => {
                                    if let Err(e) = client::send_update_result(conn, uid, result).await {
                                        tracing::warn!(error = %e, "failed to send UpdateResult during shutdown");
                                    }
                                    self.in_flight_updates.remove(&host_id);
                                }
                                client::UpdateEvent::Attention(_) => {
                                    // Ignore attention during shutdown.
                                }
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        tracing::warn!(
                            remaining = self.in_flight_updates.len(),
                            "shutdown timeout reached, abandoning remaining in-flight updates"
                        );
                        for (_, update) in self.in_flight_updates.drain() {
                            conn.send_best_effort(ServiceMessage::UpdateResult(
                                UpdateResultPayload {
                                    update_history_id: update.update_history_id,
                                    status: UpdateFinalStatus::Failed,
                                    from_version: None,
                                    to_version: None,
                                    output: String::new(),
                                    error: Some(format!(
                                        "Agent shutdown timeout ({}s) reached",
                                        shutdown_timeout.as_secs()
                                    )),
                                },
                            ))
                            .await;
                            update.forwarder.abort();
                        }
                        break;
                    }
                }
            }

            // Drain any remaining buffered events that forwarders sent before
            // aborting or completing under the timeout.
            while let Ok((host_id, event)) = self.aggregate_rx.try_recv() {
                if let Some(update) = self.in_flight_updates.get(&host_id)
                    && let client::UpdateEvent::Output(msg) = event
                {
                    client::send_update_output(conn, update.update_history_id, msg).await;
                }
            }
        }

        // Send Disconnecting and close pooled SSH connections.
        let reason_dbg = format!("{disconnect_reason:?}");
        let disconnecting_msg =
            ServiceMessage::Disconnecting(DisconnectingPayload::new(disconnect_reason));
        if let Err(e) = conn.send(disconnecting_msg).await {
            tracing::debug!(error = %e, "failed to send Disconnecting message");
        } else {
            tracing::debug!(
                reason = %reason_dbg,
                "sent Disconnecting message to controller"
            );
        }

        // Gracefully close all pooled SSH connections so remote hosts receive
        // a clean disconnect rather than a silent socket drop.
        self.pool.disconnect_all().await;

        outcome
    }
}

impl SshAgentHandler {
    /// Returns `true` if the update is allowed to proceed.
    async fn is_update_allowed(&self, host_machine_id: &str) -> bool {
        if tokio::fs::try_exists(&self.freeze_file_path)
            .await
            .unwrap_or(false)
        {
            tracing::warn!(
                freeze_file = %self.freeze_file_path.display(),
                "update execution is frozen; ignoring update message. \
                 Remove the freeze file to re-enable update execution."
            );
            return false;
        }
        if let Some(last) = self.last_update_per_host.get(host_machine_id)
            && last.elapsed() < UPDATE_COOLDOWN
        {
            tracing::warn!(
                host = %host_machine_id,
                cooldown_secs = UPDATE_COOLDOWN.as_secs(),
                elapsed_ms = last.elapsed().as_millis() as u64,
                "security_audit: update rate limit exceeded; ignoring update"
            );
            return false;
        }
        true
    }

    async fn handle_report_plugin_config_response(
        &self,
        payload: uptrakit_internal_wire::ReportPluginConfigResponsePayload,
        db: &sea_orm::DatabaseConnection,
    ) {
        if payload.success {
            if let Some(config_id_str) = &payload.plugin_config_id {
                tracing::info!(
                    request_id = %payload.request_id,
                    config_id = %config_id_str,
                    "plugin config reported successfully"
                );
                {
                    let config_id = *config_id_str;
                    let request_id = payload.request_id.clone();
                    for plugin in self.infra_plugins.iter() {
                        if let Some(report) = plugin.as_host_report()
                            && let Err(e) = report
                                .on_plugin_config_reported(db, config_id, &request_id)
                                .await
                        {
                            tracing::warn!(
                                error = %e,
                                plugin_type = %plugin.plugin_type_id(),
                                "plugin on_plugin_config_reported failed"
                            );
                        }
                    }
                }
            }
        } else {
            tracing::warn!(
                request_id = %payload.request_id,
                error = ?payload.error,
                "plugin config report failed"
            );
        }
    }

    async fn handle_reset_data(&mut self, db: &sea_orm::DatabaseConnection) {
        if cfg!(feature = "reset-data") {
            tracing::info!("received ResetData: truncating local data stores");
            use sea_orm::{ConnectionTrait, EntityTrait, TransactionTrait};
            match db.begin().await {
                Ok(txn) => {
                    // Delete in FK-safe order:
                    // 1. pending_proxmox_matches (references ssh_hosts)
                    if let Err(e) = crate::db::entity::pending_proxmox_match::Entity::delete_many()
                        .exec(&txn)
                        .await
                    {
                        tracing::error!(error = %e, "failed to truncate pending_proxmox_matches");
                    }
                    // 2. proxmox_host_state (references ssh_hosts; entity owned by
                    //    the proxmox plugin crate, so use raw SQL)
                    if let Err(e) = txn
                        .execute_unprepared("DELETE FROM proxmox_host_state")
                        .await
                    {
                        tracing::error!(error = %e, "failed to truncate proxmox_host_state");
                    }
                    // 3. ssh_hosts
                    if let Err(e) = crate::db::entity::ssh_host::Entity::delete_many()
                        .exec(&txn)
                        .await
                    {
                        tracing::error!(error = %e, "failed to truncate ssh_hosts");
                    }
                    match txn.commit().await {
                        Ok(()) => {
                            tracing::info!("local data stores truncated successfully");
                            // Clear in-memory state so the agent does not
                            // keep stale host references.
                            self.host_snapshot.clear();
                            self.last_update_per_host.clear();
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "failed to commit ResetData transaction");
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to begin ResetData transaction");
                }
            }
        } else {
            tracing::warn!("received ResetData but reset-data feature is disabled; ignoring");
        }
    }

    fn spawn_post_report_hooks(&self, db: sea_orm::DatabaseConnection) {
        let proxy = std::sync::Arc::clone(&self.extension_proxy);
        let bg_tx = self.bg_tx.clone();
        let infra_plugins = std::sync::Arc::clone(&self.infra_plugins);
        let state_dir = self.state_dir.clone();
        let tenant_id = self.tenant_id;
        let service_id = self.service_id;
        let private_key_der = self.private_key_der.clone();
        tokio::spawn(async move {
            let action_invoker = crate::extension::InfraActionInvokerImpl::new(&proxy, &bg_tx);
            let tenant_id_str = tenant_id.map(|t| t.to_string());
            let ctx = uptrakit_plugin_infrastructure_core::agent_infra::InfraPluginContext {
                db: &db,
                tenant_id: tenant_id_str.as_deref(),
                service_id,
                state_dir: &state_dir,
                private_key_der: private_key_der.as_deref(),
                action_invoker: &action_invoker,
                guest_bootstrap: &crate::operations::bootstrap_proxmox::NoopGuestBootstrapExecutor,
            };
            for plugin in infra_plugins.iter() {
                if let Some(report) = plugin.as_host_report()
                    && let Err(e) = report.on_post_report_hosts(&ctx).await
                {
                    tracing::warn!(
                        error = %e,
                        plugin_type = %plugin.plugin_type_id(),
                        "plugin on_post_report_hosts failed"
                    );
                }
            }
        });
    }

    /// React to a host-config reload tick.
    ///
    /// Queries the current `ssh_hosts` snapshot, diffs it against the stored
    /// snapshot, evicts stale pool entries, and sends an updated `ReportHosts`
    /// message if anything changed.  Returns without sending if the snapshot
    /// is unchanged.
    async fn handle_host_config_changed(&mut self, conn: &mut ControllerConnection) {
        let db = match self.local_db.as_ref() {
            Some(db) => db,
            None => {
                // Defensive: reload_ticker is None until on_connected, so this
                // branch should never be reached in practice.
                tracing::warn!("host config reload tick fired before DB was initialized; skipping");
                return;
            }
        };

        let current_snapshot = match host_ops::list_host_snapshots(db).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to list host snapshots during reload check; skipping"
                );
                return;
            }
        };

        if current_snapshot == self.host_snapshot {
            // Nothing changed — no-op.
            return;
        }

        // Compute what changed.  `diff_host_snapshots` returns owned UUIDs,
        // so no lifetime issues with the snapshot borrows.
        let (deleted_ids, changed_ids) =
            diff_host_snapshots(&self.host_snapshot, &current_snapshot);

        // Evict pool entries for deleted and updated/new hosts so the next
        // acquire establishes a fresh connection rather than reusing a stale
        // or wrong-host session.
        for id in &deleted_ids {
            self.pool.evict(*id).await;
        }
        for id in &changed_ids {
            self.pool.evict(*id).await;
        }

        // Commit the new snapshot before the async send so that a send failure
        // does not cause us to re-send on the very next tick.
        self.host_snapshot = current_snapshot;

        // Load the full host list for building HostInfo.
        let hosts = match host_ops::list_hosts(db).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to list hosts for dynamic reload; skipping ReportHosts send"
                );
                return;
            }
        };

        tracing::info!(
            total_hosts = hosts.len(),
            changed = changed_ids.len(),
            deleted = deleted_ids.len(),
            "host configuration changed — sending updated ReportHosts"
        );

        client::report_hosts_after_config_change(db, conn, &hosts, &changed_ids, &self.pool).await;

        // After ReportHosts is sent the controller has registered all SSH hosts
        // using the agent_host_id hint.  Spawn the pending-match drain as a
        // background task so the event loop stays free to forward the proxy
        // request (via bg_tx → BackgroundResult) and receive the controller
        // response — calling invoke_proxy_action inline here would deadlock
        // because on_service_event cannot poll bg_rx while it is blocked.
        if let Some(db) = self.local_db.as_ref() {
            self.spawn_post_report_hooks(db.clone());
        }
    }
}

/// Rotate DEKs from the current KEK to a new KEK (same pattern as controller).
async fn rotate_ssh_master_key(db: &sea_orm::DatabaseConnection, new_key_path: &std::path::Path) {
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, TransactionTrait};

    let new_key_hex = match std::fs::read_to_string(new_key_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(path = %new_key_path.display(), error = %e, "failed to read new master key file");
            return;
        }
    };

    let new_key_bytes = match uptrakit_shared_types::hex::decode(new_key_hex.trim()) {
        Ok(bytes) => {
            let arr: [u8; 32] = match bytes.try_into() {
                Ok(a) => a,
                Err(_) => {
                    tracing::error!("new master key must be exactly 32 bytes (64 hex chars)");
                    return;
                }
            };
            arr
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to decode new master key hex");
            return;
        }
    };
    let new_kek = zeroize::Zeroizing::new(new_key_bytes);

    let new_kek_fp = {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(new_kek.as_slice());
        uptrakit_shared_types::hex::encode(&hash[..8])
    };

    let current_kek_fp = match uptrakit_crypto::master_key_fingerprint() {
        Ok(fp) => fp,
        Err(e) => {
            tracing::error!(error = %e, "failed to compute KEK fingerprint");
            return;
        }
    };

    if new_kek_fp == current_kek_fp {
        tracing::warn!("new master key has same fingerprint as current — no rotation needed");
        return;
    }

    tracing::info!(
        current_kek_fp,
        new_kek_fp,
        "starting SSH agent master key rotation"
    );

    let txn = match db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to begin transaction for key rotation");
            return;
        }
    };

    let rows = match db::entity::data_encryption_key::Entity::find()
        .all(&txn)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to query DEKs for rotation");
            return;
        }
    };

    for row in &rows {
        let dek = match uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(key_id = %row.key_id, error = %e, "failed to unwrap DEK");
                return;
            }
        };
        let new_wrapped = match uptrakit_crypto::wrap_data_key_with(&new_kek, &dek) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!(key_id = %row.key_id, error = %e, "failed to re-wrap DEK");
                return;
            }
        };
        let mut am: db::entity::data_encryption_key::ActiveModel = row.clone().into_active_model();
        am.wrapped_key = sea_orm::Set(new_wrapped);
        am.kek_fingerprint = sea_orm::Set(new_kek_fp.clone());
        if let Err(e) = am.update(&txn).await {
            tracing::error!(key_id = %row.key_id, error = %e, "failed to update DEK row");
            return;
        }
    }

    if let Err(e) = txn.commit().await {
        tracing::error!(error = %e, "failed to commit key rotation transaction");
        return;
    }

    tracing::info!(
        dek_count = rows.len(),
        new_kek_fp,
        "SSH agent master key rotation complete — restart with the new key file"
    );
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            "uptrakit-agent-ssh",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    // Host subcommands run with minimal tracing and no rustls provider.
    if let Some(Commands::Host { command }) = args.command {
        // Verbosity-aware tracing for CLI subcommands.
        init_tracing("uptrakit_agent_ssh", args.common.verbose);

        if let Err(e) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        register_ssh_column_aad();

        let state_dir = match resolve_state_dir_from_common(&args.common).await {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };

        // Initialise DEK ring for host subcommands that read/write encrypted keys.
        match db::init_db(&state_dir).await {
            Ok(host_db) => {
                init_ssh_data_key_ring(&host_db).await;
                host_db.close().await.ok();
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not init DEK ring for host subcommand");
            }
        }

        if let Err(e) = host_cli::run(&state_dir, command).await {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // ── Daemon mode ─────────────────────────────────────────────────
    // Validate that --url is provided for daemon mode.
    if args.common.url.is_none() {
        eprintln!("error: --url is required for daemon mode");
        std::process::exit(1);
    }

    init_tracing("uptrakit_agent_ssh", args.common.verbose);
    uptrakit_service_sdk::init_crypto();

    // Initialize master encryption key for local SSH credential storage.
    if let Err(e) = init_master_key(&args.master_key_file, args.allow_plaintext_secrets) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
    register_ssh_column_aad();

    // Resolve state directory early so we can pass it to the handler.
    let state_dir = match resolve_state_dir_from_common(&args.common).await {
        Ok(dir) => dir,
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    };

    // Initialize the local DB and run pending migrations at startup so that the
    // schema is always up-to-date before any operations — both daemon and CLI.
    // Waiting until `on_connected` would mean a freshly started service could
    // not open the DB if it has not yet reached the controller.
    let local_db = match crate::db::init_db(&state_dir).await {
        Ok(db) => {
            tracing::debug!("local SSH host database initialized");
            db
        }
        Err(e) => {
            tracing::error!("failed to initialize local database: {e}");
            std::process::exit(1);
        }
    };

    // Initialise the data-encryption-key ring (generates first DEK if needed).
    init_ssh_data_key_ring(&local_db).await;

    // Auto-upgrade any non-v3 encrypted values to ENC:v3:.
    reencrypt_ssh_to_v3(&local_db).await;

    // Master key rotation (re-wraps DEKs with the new KEK).
    if let Some(ref new_key_path) = args.rotate_master_key_file {
        rotate_ssh_master_key(&local_db, new_key_path).await;
    }

    let (aggregate_tx, aggregate_rx) =
        tokio::sync::mpsc::channel::<(String, client::UpdateEvent)>(256);
    let (bg_tx, bg_rx) = tokio::sync::mpsc::channel::<uptrakit_internal_wire::ServiceMessage>(64);

    let freeze_file_path = state_dir.join("update-freeze");

    let infra_plugins =
        std::sync::Arc::new(uptrakit_plugin_infrastructure_registry::create_agent_infra_plugins());

    let mut handler = SshAgentHandler {
        local_db: Some(local_db),
        state_dir: state_dir.to_path_buf(),
        service_id: None,
        tenant_id: None,
        private_key_der: None,
        encryption_public_key: None,
        freeze_file_path,
        in_flight_updates: HashMap::new(),
        aggregate_rx,
        aggregate_tx,
        pool: ssh_pool::SshConnectionPool::new(),
        reload_ticker: None,
        host_snapshot: Vec::new(),
        last_update_per_host: HashMap::new(),
        bg_rx,
        bg_tx,
        extension_proxy: std::sync::Arc::new(uptrakit_service_sdk::ServiceExtensionProxy::new()),
        infra_plugins,
        pending_initial_host_report: false,
    };
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-agent-ssh",
        &args.common,
        &mut handler,
    )
    .await;
}

/// Resolve the state directory for this service.
async fn resolve_state_dir_from_common(
    common: &uptrakit_service_sdk::cli::CommonServiceArgs,
) -> InitResult<std::path::PathBuf> {
    let dirs = common.resolve_dirs("agent-ssh").map_err(|e| {
        report!(InitError::Directory(format!(
            "failed to resolve directories: {e}"
        )))
    })?;
    dirs.ensure_state_dir().await.map_err(|e| {
        report!(InitError::Directory(format!(
            "failed to ensure state directory: {e}"
        )))
    })?;
    Ok(dirs.state_dir().to_path_buf())
}

/// Initialize the master encryption key from CLI args or environment.
fn init_master_key(
    master_key_file: &Option<std::path::PathBuf>,
    allow_plaintext_secrets: bool,
) -> InitResult<()> {
    let env_val = std::env::var("UPTRAKIT_MASTER_KEY").ok();
    // Clear the environment variable immediately to remove it from
    // /proc/pid/environ, container inspection output, and child processes.
    //
    // SAFETY: this is called during single-threaded startup before any
    // async runtime or threads are spawned.
    unsafe { std::env::remove_var("UPTRAKIT_MASTER_KEY") };
    let key_hex = read_master_key_hex(master_key_file.as_deref(), env_val.as_deref())?;

    match key_hex {
        Some(key_hex) => {
            if allow_plaintext_secrets {
                tracing::warn!(
                    "--allow-plaintext-secrets is enabled. This flag is for development only; \
                    encryption remains enabled because a master key was provided."
                );
            }
            let key_bytes = parse_master_key_hex(&key_hex)?;
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).map_err(|e| {
                report!(InitError::MasterKey(format!(
                    "failed to initialize master key: {e}"
                )))
            })?;
            tracing::info!("master encryption key initialized");
        }
        None => {
            if allow_plaintext_secrets {
                tracing::warn!(
                    "master encryption key not set; encryption at rest is disabled. \
                    This is for development only and is NOT safe for production."
                );
                uptrakit_crypto::enable_plaintext_mode();
            } else {
                bail!(InitError::MasterKey(
                    "master encryption key is required: set UPTRAKIT_MASTER_KEY env var \
                     (64-char hex string) or pass --master-key-file <path>. \
                     For development only, pass --allow-plaintext-secrets to run without \
                     encryption at rest."
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

fn read_master_key_hex(
    master_key_file: Option<&std::path::Path>,
    env_val: Option<&str>,
) -> InitResult<Option<String>> {
    if let Some(key_file) = master_key_file {
        let contents = std::fs::read_to_string(key_file).map_err(|e| report!(InitError::Io(e)))?;
        return Ok(Some(contents.trim().to_string()));
    }

    if let Some(env_val) = env_val {
        return Ok(Some(env_val.trim().to_string()));
    }

    Ok(None)
}

fn parse_master_key_hex(key_hex: &str) -> InitResult<[u8; 32]> {
    let bytes = uptrakit_shared_types::hex::decode(key_hex).map_err(|e| {
        report!(InitError::Hex(format!(
            "master key must be a 64-character hex string: {e}"
        )))
    })?;
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
        report!(InitError::Hex(format!(
            "master key must be exactly 32 bytes (64 hex chars), got {} bytes",
            v.len()
        )))
    })?;
    Ok(key_bytes)
}

/// Initialize `tracing_subscriber` with a verbosity-aware filter.
fn init_tracing(own_module: &str, verbosity: u8) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    if verbosity > 3 {
        eprintln!(
            "warning: -vvvv or more has no additional effect; maximum verbosity is -vvv (trace)"
        );
    }

    let directive = match verbosity {
        0 => "uptrakit=info".to_string(),
        1 => format!("{own_module}=debug"),
        2 => "uptrakit=debug".to_string(),
        _ => "uptrakit=trace".to_string(),
    };
    let mut filter = EnvFilter::from_default_env();
    if let Ok(d) = directive.parse() {
        filter = filter.add_directive(d);
    }
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .init();
}

#[cfg(test)]
mod tests {
    use super::{parse_master_key_hex, read_master_key_hex};
    use std::io::Write;

    #[test]
    fn missing_key_returns_none() {
        let result = read_master_key_hex(None, None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn env_key_is_trimmed() {
        let result = read_master_key_hex(None, Some("  deadbeef  "));
        assert!(matches!(result, Ok(Some(ref value)) if value == "deadbeef"));
    }

    #[test]
    fn file_key_is_trimmed() {
        let mut file = match tempfile::NamedTempFile::new() {
            Ok(f) => f,
            Err(_) => return,
        };
        assert!(file.write_all(b"  0123  ").is_ok());
        let result = read_master_key_hex(Some(file.path()), None);
        assert!(matches!(result, Ok(Some(ref value)) if value == "0123"));
    }

    #[test]
    fn parse_master_key_rejects_invalid_hex() {
        let result = parse_master_key_hex("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_rejects_invalid_length() {
        let result = parse_master_key_hex("aa");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_accepts_valid_length() {
        let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_master_key_hex(key_hex);
        assert!(matches!(result, Ok(bytes) if bytes.len() == 32));
    }

    // ── poll_updates tests ───────────────────────────────────────────────────

    use super::{HashMap, SshAgentHandler, client};

    /// `poll_updates` must park indefinitely (never resolve) when the
    /// `in_flight_updates` map is empty, regardless of whether the channel
    /// has buffered events.
    #[tokio::test]
    async fn poll_updates_parks_when_map_is_empty() {
        let (_, mut rx) = tokio::sync::mpsc::channel::<(String, client::UpdateEvent)>(4);
        let empty_map: HashMap<String, client::SshInFlightUpdate> = HashMap::new();

        let timed_out = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            SshAgentHandler::poll_updates(&mut rx, &empty_map),
        )
        .await
        .is_err();

        assert!(
            timed_out,
            "poll_updates must not resolve when in_flight_updates is empty"
        );
    }

    /// `poll_updates` must return the next event from the aggregate channel
    /// when the `in_flight_updates` map is non-empty.
    #[tokio::test]
    async fn poll_updates_returns_event_when_map_nonempty() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, client::UpdateEvent)>(4);

        let mut map: HashMap<String, client::SshInFlightUpdate> = HashMap::new();
        map.insert(
            "host-1".to_string(),
            client::SshInFlightUpdate {
                update_history_id: uuid::Uuid::nil(),
                forwarder: tokio::spawn(std::future::pending()),
                #[cfg(feature = "interactive")]
                stdin_tx: None,
                #[cfg(feature = "interactive")]
                signal_tx: None,
            },
        );

        // Construct a completed-update event by spawning a trivial task
        // (the only safe way to get a Result<UpdateExecutionResult, JoinError>).
        let exec_result = tokio::spawn(async {
            uptrakit_agent_core::update::UpdateExecutionResult {
                result: uptrakit_internal_wire::UpdateResultPayload {
                    update_history_id: uuid::Uuid::nil(),
                    status: uptrakit_internal_wire::UpdateFinalStatus::Completed,
                    from_version: None,
                    to_version: None,
                    output: String::new(),
                    error: None,
                },
            }
        })
        .await;

        tx.send((
            "host-1".to_string(),
            client::UpdateEvent::Completed(exec_result),
        ))
        .await
        .unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            SshAgentHandler::poll_updates(&mut rx, &map),
        )
        .await;

        assert!(
            result.is_ok(),
            "poll_updates must return an event when the map is non-empty"
        );
        let (host_id, _) = result.unwrap();
        assert_eq!(host_id, "host-1");

        for (_, update) in map.drain() {
            update.forwarder.abort();
        }
    }
}
