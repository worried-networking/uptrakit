#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel sends; errors mean the receiver is gone and we must not block"
)]

use std::collections::HashMap;
use std::sync::Arc;

use crate::SshInFlightUpdate;
use std::collections::{BTreeSet, HashSet};
use tokio::task::JoinSet;
use uptrakit_agent_core::ConnectionContext;
use uptrakit_command::{CommandExecutor, CommandSpec, SudoAwareCommandExecutor};
use uptrakit_plugin_infrastructure_registry::{HostCapabilities, HostRuntime};

use uptrakit_wire::{
    BatchUpdateItemResult, BatchUpdateResultPayload, Capability, CheckVersionsPayload,
    DiscoverSoftwarePayload, DiscoveryPluginResult, DiscoveryResultsPayload,
    ExecuteBatchUpdatePayload, ExecuteUpdatePayload, HostInfo, ReportHostsPayload, ServiceMessage,
    ServiceTransport, UpdateCategory, UpdateFinalStatus, UpdateResultPayload, VersionCheckResult,
    VersionCheckResultsPayload,
};

use crate::db::entity::ssh_host::Model;
use crate::host_info::{collect_remote_host_info, collect_remote_host_info_routeros};
use crate::host_ops::{find_host_by_machine_id, list_hosts, update_host_machine_id};
use crate::routeros_executor::RouterOsSshExecutor;
use crate::ssh_executor::PosixSshCommandExecutor;
use crate::ssh_pool::SshConnectionPool;
use crate::ssh_transport::SshSession;

// Re-export shared update types for use in main.rs.
pub use uptrakit_agent_core::UpdateEvent;

/// Receive from an optional attention channel. Pends forever when `None`.
async fn recv_attention_opt(rx: &mut Option<tokio::sync::mpsc::Receiver<()>>) -> Option<()> {
    if let Some(rx) = rx {
        return rx.recv().await;
    }
    std::future::pending().await
}

/// Receive from an optional interactive-channels resolution oneshot. Pends
/// forever when `None` (either never interactive, or already resolved and
/// reset by the caller).
#[cfg(feature = "interactive")]
async fn recv_channels_opt(
    rx: &mut Option<
        tokio::sync::oneshot::Receiver<uptrakit_agent_core::update::InteractiveChannels>,
    >,
) -> std::result::Result<
    uptrakit_agent_core::update::InteractiveChannels,
    tokio::sync::oneshot::error::RecvError,
> {
    match rx {
        Some(rx) => rx.await,
        None => std::future::pending().await,
    }
}

/// Drain anything already buffered in the proxy stdin/signal receivers into
/// the real PTY channels, once `channels_rx` resolves `Ok`.
///
/// Pinned semantic: pre-resolution `try_send`s from `handle_update_stdin_data_ssh`
/// land in the bounded proxy (capacity `SSH_PROXY_CHANNEL_CAPACITY`); this
/// drains everything buffered there into the real channel so it is delivered,
/// not lost. Anything beyond the proxy's capacity was already dropped at
/// `try_send` time (existing warn, no new drop path here).
#[cfg(feature = "interactive")]
fn drain_proxies_into_real_channels(
    stdin_proxy_rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>,
    signal_proxy_rx: &mut tokio::sync::mpsc::Receiver<i32>,
    real_stdin_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
    real_signal_tx: &tokio::sync::mpsc::Sender<i32>,
) {
    while let Ok(buffered) = stdin_proxy_rx.try_recv() {
        let _ = real_stdin_tx.try_send(buffered);
    }
    while let Ok(buffered) = signal_proxy_rx.try_recv() {
        let _ = real_signal_tx.try_send(buffered);
    }
}

// ── Per-host runtime dispatch ────────────────────────────────────────────────

/// Select the appropriate [`HostRuntime`] implementation for the given host.
///
/// If the host has a `routeros_host_config` row in the local database, a
/// [`RouterOsHostRuntime`] is returned, wired to the RouterOS-specific SSH
/// executor.  Otherwise — or when the DB query fails — falls back to a
/// POSIX [`HostRuntime`] via [`construct_host_runtime`] with a `"linux"`
/// capability set.
async fn build_host_runtime(
    host_id: uuid::Uuid,
    session: Arc<SshSession>,
    executor: Arc<dyn CommandExecutor>,
    db: &sea_orm::DatabaseConnection,
) -> Arc<dyn HostRuntime> {
    use sea_orm::EntityTrait as _;
    use uptrakit_plugin_infrastructure_registry::{
        construct_host_runtime, construct_routeros_host_runtime,
    };
    use uptrakit_shared_types::host_features;

    use crate::db::entity::routeros_host_config;

    match routeros_host_config::Entity::find_by_id(host_id)
        .one(db)
        .await
    {
        Ok(Some(ros_config)) => {
            let ros_exec = Arc::new(RouterOsSshExecutor::new(Arc::clone(&session)));
            let caps = HostCapabilities::new(
                Some("routeros"),
                None,
                None,
                &[host_features::ROUTER_OS_CLI.as_str().to_string()],
            );
            construct_routeros_host_runtime(ros_exec, caps, ros_config.allow_reboot)
        }
        Ok(None) => {
            let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
            construct_host_runtime(executor, caps)
        }
        Err(e) => {
            tracing::warn!(
                host_id = %host_id,
                error = %e,
                "failed to query routeros_host_config; defaulting to StandardHostRuntime"
            );
            let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
            construct_host_runtime(executor, caps)
        }
    }
}

// ── Connection context ────────────────────────────────────────────────────────

/// Build a [`ConnectionContext`] for the given SSH host.
///
/// Docker daemon connectivity is now handled by the executor's
/// [`StdioTunnel`](uptrakit_command::StdioTunnel) support — the context no
/// longer writes temporary key files or sets docker_host overrides. The
/// returned context only carries RAII handles when needed.
fn build_connection_context() -> ConnectionContext {
    ConnectionContext::default()
}

// ── ReportHosts ───────────────────────────────────────────────────────────────

/// Connect to each enrolled SSH host, collect system info, and send a
/// `ReportHosts` message to the controller.
///
/// All hosts are contacted **in parallel**: one `tokio` task is spawned per
/// host so that SSH handshakes and remote commands overlap instead of
/// serialising.  Errors for individual hosts are logged as warnings and
/// skipped; the remaining hosts are still reported.
#[tracing::instrument(skip_all)]
pub async fn report_enrolled_hosts(
    local_db: &sea_orm::DatabaseConnection,
    conn: &mut dyn ServiceTransport,
    pool: &SshConnectionPool,
    agent_version: &str,
) {
    let hosts = match list_hosts(local_db).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list SSH hosts for reporting");
            return;
        }
    };

    let total = hosts.len();
    let mut join_set: JoinSet<Option<HostInfo>> = JoinSet::new();

    for host in hosts {
        let db = local_db.clone();
        let pool = pool.clone();
        join_set.spawn(async move { collect_one_host_for_report(db, pool, host).await });
    }

    let mut host_infos: Vec<HostInfo> = Vec::with_capacity(total);
    while let Some(task_result) = join_set.join_next().await {
        match task_result {
            Ok(Some(info)) => host_infos.push(info),
            Ok(None) => {} // host was skipped; warning already logged inside helper
            Err(join_err) => {
                tracing::error!(
                    error = %join_err,
                    "host info collection task panicked"
                );
            }
        }
    }

    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: host_infos,
        agent_version: agent_version.to_string(),
        capabilities: ssh_agent_capabilities(),
    });

    if let Err(e) = conn.transport_send_auto_paginate(msg).await {
        tracing::warn!(error = %e, "failed to send ReportHosts message");
    } else {
        tracing::info!(host_count = total, "reported enrolled hosts to controller");
    }
}

/// Collect [`HostInfo`] for a single host during the initial startup report.
///
/// Returns `None` if the SSH session cannot be acquired or the executor check
/// fails; the host is skipped in that case.  Persists the discovered
/// `machine_id` to the local database before returning.
async fn collect_one_host_for_report(
    db: sea_orm::DatabaseConnection,
    pool: SshConnectionPool,
    host: Model,
) -> Option<HostInfo> {
    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        "collecting host info"
    );

    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                host_name = %host.name,
                hostname = %host.hostname,
                error = %e,
                "failed to acquire SSH session for reporting, skipping"
            );
            return None;
        }
    };

    use crate::db::entity::routeros_host_config;
    use sea_orm::EntityTrait as _;

    let is_routeros = routeros_host_config::Entity::find_by_id(host.id)
        .one(&db)
        .await
        .unwrap_or(None)
        .is_some();

    let mut info = if is_routeros {
        let ros_exec = RouterOsSshExecutor::new(Arc::clone(&session));
        collect_remote_host_info_routeros(&ros_exec).await
    } else {
        // Verify that command execution is available via the CommandExecutor
        // interface before proceeding with host information collection.
        let executor = PosixSshCommandExecutor::new(Arc::clone(&session));
        if executor
            .execute_quiet(&CommandSpec::exec("true", Vec::<String>::new()))
            .await
            .is_err()
        {
            tracing::warn!(
                host_name = %host.name,
                hostname = %host.hostname,
                "SSH command executor check failed, evicting session and skipping host"
            );
            pool.evict(host.id).await;
            return None;
        }
        collect_remote_host_info(&session, &executor).await
    };
    // Set the SSH target address as the host's ip_address.
    info.ip_address = Some(host.hostname.clone());
    // Provide the agent-local UUID so the controller can use it as hosts.id.
    info.agent_host_id = Some(host.id);

    // Persist the machine_id so incoming CheckVersions / ExecuteUpdate
    // messages can be routed to this host via find_host_by_machine_id().
    if let Err(e) = update_host_machine_id(&db, host.id, &info.machine_id).await {
        tracing::warn!(
            host_name = %host.name,
            machine_id = %info.machine_id,
            error = %e,
            "failed to persist machine_id for SSH host"
        );
    }

    tracing::debug!(
        host_name = %host.name,
        machine_id = %info.machine_id,
        hostname = ?info.hostname,
        "collected remote host info"
    );

    Some(info)
}

// ── Dynamic host reload ───────────────────────────────────────────────────────

/// Returns `true` when `host` requires an SSH round-trip to refresh its info.
///
/// A host needs SSH when:
/// - its `machine_id` is not yet known (`None`), or
/// - it appears in `changed_ids` (added or updated since the last snapshot).
///
/// Extracted as a free function so it can be unit-tested independently.
fn host_needs_ssh(host: &Model, changed_ids: &HashSet<uuid::Uuid>) -> bool {
    host.machine_id.is_none() || changed_ids.contains(&host.id)
}

/// Build the fast-path [`HostInfo`] for a host that does not require SSH.
///
/// Uses the `machine_id` already persisted in the database and leaves all
/// OS-level fields as `None` (they were not refreshed).
///
/// # Panics (impossible)
/// This function is only called when `host_needs_ssh` returns `false`, which
/// guarantees `machine_id.is_some()`.  The `unwrap_or_default` is a
/// belt-and-suspenders fallback.
fn build_fast_path_host_info(host: &Model) -> HostInfo {
    HostInfo {
        machine_id: host.machine_id.clone().unwrap_or_default(),
        os_type: None,
        os_version: None,
        architecture: None,
        hostname: None,
        ip_address: Some(host.hostname.clone()),
        agent_host_id: Some(host.id),
        features: None,
    }
}

/// Build `HostInfo` entries for the current host list, using SSH only where
/// necessary.
///
/// For hosts with a known `machine_id` that are **not** in `changed_ids`
/// (neither new nor updated since the last reload), host info is built
/// directly from the database values — no SSH connection is made.
///
/// For hosts with `machine_id` equal to `None` or whose `id` is in
/// `changed_ids` (new or recently updated), SSH tasks are spawned and run
/// **in parallel**: all such hosts are contacted concurrently so that network
/// latency for one host does not delay the others.
///
/// Hosts that fail to connect are skipped with a warning.
pub async fn build_reload_host_infos(
    db: &sea_orm::DatabaseConnection,
    current_hosts: &[Model],
    changed_ids: &HashSet<uuid::Uuid>,
    pool: &SshConnectionPool,
) -> Vec<HostInfo> {
    // Fast-path: build HostInfo from DB values for hosts that need no SSH.
    let mut host_infos: Vec<HostInfo> = current_hosts
        .iter()
        .filter(|h| !host_needs_ssh(h, changed_ids))
        .map(build_fast_path_host_info)
        .collect();

    // Spawn parallel tasks for the SSH-needing hosts.
    let mut join_set: JoinSet<Option<HostInfo>> = JoinSet::new();
    for host in current_hosts
        .iter()
        .filter(|h| host_needs_ssh(h, changed_ids))
    {
        let db = db.clone();
        let pool = pool.clone();
        let host = host.clone();
        join_set.spawn(async move { collect_one_host_for_reload(db, pool, host).await });
    }

    while let Some(task_result) = join_set.join_next().await {
        match task_result {
            Ok(Some(info)) => host_infos.push(info),
            Ok(None) => {} // host skipped; warning already logged inside helper
            Err(join_err) => {
                tracing::error!(
                    error = %join_err,
                    "host info reload task panicked"
                );
            }
        }
    }

    host_infos
}

/// Collect [`HostInfo`] for a single host during a dynamic reload.
///
/// Establishes (or reuses from the pool) an SSH session, collects remote
/// system info, and persists the `machine_id` to the local database.
///
/// Returns `None` if the SSH session cannot be acquired; the host is skipped
/// in that case.
async fn collect_one_host_for_reload(
    db: sea_orm::DatabaseConnection,
    pool: SshConnectionPool,
    host: Model,
) -> Option<HostInfo> {
    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                host_name = %host.name,
                hostname = %host.hostname,
                error = %e,
                "failed to acquire SSH session during dynamic reload, skipping host"
            );
            return None;
        }
    };

    use crate::db::entity::routeros_host_config;
    use sea_orm::EntityTrait as _;

    let is_routeros = routeros_host_config::Entity::find_by_id(host.id)
        .one(&db)
        .await
        .unwrap_or(None)
        .is_some();

    let mut info = if is_routeros {
        let ros_exec = RouterOsSshExecutor::new(Arc::clone(&session));
        collect_remote_host_info_routeros(&ros_exec).await
    } else {
        let executor = PosixSshCommandExecutor::new(Arc::clone(&session));
        collect_remote_host_info(&session, &executor).await
    };
    info.ip_address = Some(host.hostname.clone());
    info.agent_host_id = Some(host.id);

    if let Err(e) = update_host_machine_id(&db, host.id, &info.machine_id).await {
        tracing::warn!(
            host_name = %host.name,
            machine_id = %info.machine_id,
            error = %e,
            "failed to persist machine_id during dynamic host reload"
        );
    }

    tracing::debug!(
        host_name = %host.name,
        machine_id = %info.machine_id,
        "collected remote host info during dynamic reload"
    );

    Some(info)
}

/// Build updated host infos and send a `ReportHosts` message to the controller.
///
/// Called from `SshAgentHandler::on_service_event` when a
/// `SshAgentEvent::HostConfigChanged` event fires and the host snapshot has
/// actually changed.
#[tracing::instrument(skip_all, fields(host_count = current_hosts.len()))]
pub async fn report_hosts_after_config_change(
    db: &sea_orm::DatabaseConnection,
    conn: &mut dyn ServiceTransport,
    current_hosts: &[Model],
    changed_ids: &HashSet<uuid::Uuid>,
    pool: &SshConnectionPool,
    agent_version: &str,
) {
    let host_infos = build_reload_host_infos(db, current_hosts, changed_ids, pool).await;
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: host_infos,
        agent_version: agent_version.to_string(),
        capabilities: ssh_agent_capabilities(),
    });

    if let Err(e) = conn.transport_send_auto_paginate(msg).await {
        tracing::warn!(
            error = %e,
            "failed to send ReportHosts after dynamic host config change"
        );
    } else {
        tracing::info!(
            host_count = current_hosts.len(),
            "sent dynamic ReportHosts to controller after host config change"
        );
    }
}

/// Capabilities advertised by the SSH agent service.
pub fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    let mut caps: BTreeSet<Capability> = [
        Capability::SoftwareDiscovery,
        Capability::UpdateHooks,
        Capability::GracefulShutdown,
        Capability::SshRemote,
        Capability::UiSurfaces,
    ]
    .into_iter()
    .collect();
    if cfg!(feature = "interactive") {
        caps.insert(Capability::InteractiveUpdates);
    }
    if cfg!(feature = "reset-data") {
        caps.insert(Capability::ResetData);
    }
    caps
}

// ── ExecuteUpdate ─────────────────────────────────────────────────────────────

/// Handle an `ExecuteUpdate` message for the SSH agent.
///
/// Looks up the SSH host by `host_machine_id`, acquires a pooled session, and
/// spawns the update task with a per-host concurrency guard.
///
/// Unlike the regular agent (which uses a global `Option<InFlightUpdate>`),
/// the SSH agent maintains a `HashMap<String, SshInFlightUpdate>` keyed by
/// `host_machine_id`. This allows different hosts to update simultaneously
/// while still preventing two concurrent updates on the **same** host.
///
/// Each spawned update gets a lightweight **forwarder task** that owns the
/// `InFlightUpdate` and forwards all output/completion events to the shared
/// `aggregate_tx` channel. The `SshAgentHandler` drains that channel in
/// `poll_service_event`.
#[tracing::instrument(skip_all, fields(host_machine_id = %payload.host_machine_id, update_id = %payload.update_history_id))]
pub async fn handle_execute_update_ssh(
    payload: ExecuteUpdatePayload,
    db: &sea_orm::DatabaseConnection,
    in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
    aggregate_tx: &tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
    conn: &mut dyn ServiceTransport,
    pool: &SshConnectionPool,
) {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                update_id = %payload.update_history_id,
                "no SSH host found for ExecuteUpdate host_machine_id"
            );
            conn.transport_send_best_effort(make_ssh_update_error_response(
                payload.update_history_id,
                format!(
                    "SSH host with machine_id '{}' not found",
                    payload.host_machine_id
                ),
            ))
            .await;
            return;
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                update_id = %payload.update_history_id,
                error = %e,
                "DB error looking up SSH host for ExecuteUpdate"
            );
            conn.transport_send_best_effort(make_ssh_update_error_response(
                payload.update_history_id,
                format!("DB error: {e}"),
            ))
            .await;
            return;
        }
    };

    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                update_id = %payload.update_history_id,
                error = %e,
                "failed to acquire SSH session for ExecuteUpdate"
            );
            pool.evict(host.id).await;
            conn.transport_send_best_effort(make_ssh_update_error_response(
                payload.update_history_id,
                format!("SSH connection failed: {e}"),
            ))
            .await;
            return;
        }
    };

    // Per-host concurrency guard — reject if this host already has an update in flight.
    if in_flight_updates.contains_key(&payload.host_machine_id) {
        tracing::warn!(
            host_machine_id = %payload.host_machine_id,
            update_id = %payload.update_history_id,
            "rejecting update: another update is already in progress for this host"
        );
        conn.transport_send_best_effort(make_ssh_update_error_response(
            payload.update_history_id,
            format!(
                "Another update is already in progress for host '{}'",
                payload.host_machine_id
            ),
        ))
        .await;
        return;
    }

    let ctx = build_connection_context();

    // The session Arc is shared with the executor that travels into the spawned
    // update task, keeping the SSH connection alive for the duration of the
    // update.  The pool's own Arc remains so the session is returned to the
    // pool after the task completes.
    let raw: Arc<dyn CommandExecutor> =
        Arc::new(PosixSshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        "running update on SSH host"
    );

    let host_machine_id = payload.host_machine_id.clone();
    let update_history_id = payload.update_history_id;

    let runtime =
        build_host_runtime(host.id, Arc::clone(&session), Arc::clone(&executor), db).await;

    #[cfg_attr(
        not(feature = "interactive"),
        expect(
            unused_mut,
            reason = "mut only needed when interactive feature enables .take() calls on in_flight fields"
        )
    )]
    let mut in_flight = uptrakit_agent_core::start_update(payload, runtime, conn, &ctx).await;

    // Extract the resolution oneshot before moving InFlightUpdate into the
    // forwarder, and synchronously create the proxy stdin/signal channels.
    //
    // The map entry (SshInFlightUpdate) stores the proxy SEND halves directly
    // — consumers (handle_update_stdin_data_ssh) never see an Option/lock,
    // they try_send into the proxy exactly as before. The forwarder task
    // below holds the proxy RECEIVE halves plus channels_rx, and bridges
    // proxy -> real PTY channels once channels_rx resolves, draining
    // anything already buffered so pre-resolution stdin isn't lost.
    #[cfg(feature = "interactive")]
    let channels_rx = in_flight.channels_rx.take();
    #[cfg(feature = "interactive")]
    let (stdin_proxy_tx, mut stdin_proxy_rx) =
        tokio::sync::mpsc::channel::<Vec<u8>>(crate::SSH_PROXY_CHANNEL_CAPACITY);
    #[cfg(feature = "interactive")]
    let (signal_proxy_tx, mut signal_proxy_rx) =
        tokio::sync::mpsc::channel::<i32>(crate::SSH_PROXY_CHANNEL_CAPACITY);
    #[cfg(feature = "interactive")]
    let resolution = Arc::new(parking_lot::Mutex::new(
        crate::SshInteractiveResolution::pending(),
    ));
    #[cfg(feature = "interactive")]
    let forwarder_resolution = Arc::clone(&resolution);

    tracing::debug!(
        host_machine_id = %host_machine_id,
        update_history_id = %update_history_id,
        "spawning update forwarder task for SSH host"
    );

    // Spawn a forwarder task that owns the InFlightUpdate and forwards all
    // output/completion/attention events to the shared aggregate channel.
    let host_id = host_machine_id.clone();
    let tx = aggregate_tx.clone();
    let forwarder = tokio::spawn(async move {
        let mut handle = in_flight.handle;
        let mut output_rx = in_flight.output_rx;
        let mut early_result_rx = in_flight.early_result_rx;
        let update_history_id = in_flight.update_history_id;
        // Extract the attention channel (feature-gated in InFlightUpdate).
        #[cfg(feature = "interactive")]
        let mut attention_rx: Option<tokio::sync::mpsc::Receiver<()>> = in_flight.attention_rx;
        #[cfg(not(feature = "interactive"))]
        let mut attention_rx: Option<tokio::sync::mpsc::Receiver<()>> = None;
        // Real PTY stdin/signal senders, filled in once channels_rx resolves
        // Ok. Until then, proxy-buffered data has nowhere to bridge to.
        #[cfg(feature = "interactive")]
        let mut stdin_real_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>> = None;
        #[cfg(feature = "interactive")]
        let mut signal_real_tx: Option<tokio::sync::mpsc::Sender<i32>> = None;
        #[cfg(feature = "interactive")]
        let mut channels_rx = channels_rx;
        loop {
            #[cfg(feature = "interactive")]
            tokio::select! {
                biased;
                Some(early) = early_result_rx.recv() => {
                    if tx.send((host_id.clone(), UpdateEvent::EarlyResult(early))).await.is_err() {
                        break;
                    }
                }
                Some(msg) = output_rx.recv() => {
                    if tx.send((host_id.clone(), UpdateEvent::Output(msg))).await.is_err() {
                        // The aggregate channel receiver has gone away.  We must
                        // still await the update handle (so the task is not
                        // orphaned) and attempt to send Completed, even though it
                        // will likely also fail — this avoids leaving the DB in
                        // `in_progress` if the event loop is merely slow to start.
                        let result = handle.await;
                        let _ = tx.send((host_id, UpdateEvent::Completed(result))).await;
                        break;
                    }
                }
                result = &mut handle => {
                    let _ = tx.send((host_id, UpdateEvent::Completed(result))).await;
                    break;
                }
                Some(()) = recv_attention_opt(&mut attention_rx) => {
                    let _ = tx.send((host_id.clone(), UpdateEvent::Attention(update_history_id))).await;
                }
                resolved = recv_channels_opt(&mut channels_rx) => {
                    channels_rx = None;
                    match resolved {
                        Ok((real_stdin_tx, real_signal_tx, real_attention_rx)) => {
                            // Drain anything buffered pre-resolution, then bridge live.
                            drain_proxies_into_real_channels(
                                &mut stdin_proxy_rx,
                                &mut signal_proxy_rx,
                                &real_stdin_tx,
                                &real_signal_tx,
                            );
                            stdin_real_tx = Some(real_stdin_tx);
                            signal_real_tx = Some(real_signal_tx);
                            attention_rx = Some(real_attention_rx);
                            let mut state = forwarder_resolution.lock();
                            state.resolved = true;
                            state.channels_rx_pending = false;
                        }
                        Err(_) => {
                            // No real PTY to bridge to — drop the proxies implicitly
                            // by leaving stdin_real_tx/signal_real_tx as None.
                            tracing::warn!(
                                %update_history_id,
                                "interactive update ended without PTY promotion; stdin/signal channels unavailable"
                            );
                            let mut state = forwarder_resolution.lock();
                            state.resolved = false;
                            state.channels_rx_pending = false;
                        }
                    }
                }
                Some(data) = stdin_proxy_rx.recv(), if stdin_real_tx.is_some() => {
                    if let Some(real) = &stdin_real_tx {
                        let _ = real.try_send(data);
                    }
                }
                Some(sig) = signal_proxy_rx.recv(), if signal_real_tx.is_some() => {
                    if let Some(real) = &signal_real_tx {
                        let _ = real.try_send(sig);
                    }
                }
            }

            // Non-interactive fallback: a select! with only the non-PTY arms.
            // The additive-only feature rule (coding-standards.md) cannot apply
            // here: `tokio::select!` rejects `#[cfg]` attributes on individual
            // arms, so a single unified select! gated per-arm does not compile.
            // The only additive alternative would compile the entire PTY
            // machinery (stdin/signal bridging, channel resolution) into the
            // non-interactive agent binary. A paired `#[cfg(feature)]` /
            // `#[cfg(not(feature))]` block is the sanctioned pattern for this —
            // it mirrors the existing sites in agent-runtime/src/lib.rs.
            #[cfg(not(feature = "interactive"))]
            tokio::select! {
                biased;
                Some(early) = early_result_rx.recv() => {
                    if tx.send((host_id.clone(), UpdateEvent::EarlyResult(early))).await.is_err() {
                        break;
                    }
                }
                Some(msg) = output_rx.recv() => {
                    if tx.send((host_id.clone(), UpdateEvent::Output(msg))).await.is_err() {
                        // The aggregate channel receiver has gone away.  We must
                        // still await the update handle (so the task is not
                        // orphaned) and attempt to send Completed, even though it
                        // will likely also fail — this avoids leaving the DB in
                        // `in_progress` if the event loop is merely slow to start.
                        let result = handle.await;
                        let _ = tx.send((host_id, UpdateEvent::Completed(result))).await;
                        break;
                    }
                }
                result = &mut handle => {
                    let _ = tx.send((host_id, UpdateEvent::Completed(result))).await;
                    break;
                }
                Some(()) = recv_attention_opt(&mut attention_rx) => {
                    let _ = tx.send((host_id.clone(), UpdateEvent::Attention(update_history_id))).await;
                }
            }
        }
    });

    in_flight_updates.insert(
        host_machine_id,
        SshInFlightUpdate {
            update_history_id,
            forwarder,
            early_sent: false,
            #[cfg(feature = "interactive")]
            stdin_tx: stdin_proxy_tx,
            #[cfg(feature = "interactive")]
            signal_tx: signal_proxy_tx,
            #[cfg(feature = "interactive")]
            resolution,
        },
    );
}

// ── Background-spawned operations ─────────────────────────────────────────────
//
// These functions spawn long-running operations (discovery, version checks,
// batch updates) as background tokio tasks so the event loop remains
// responsive for pings, signals, and other controller messages.
//
// Each function resolves the SSH host and session on the calling task (quick),
// then spawns the actual work. The completed `ServiceMessage` is sent through
// `bg_tx` for the event loop to forward to the controller.

/// Spawn a `CheckVersions` operation as a background task.
pub fn spawn_check_versions_ssh(
    ops: &uptrakit_agent_core::BackgroundOps,
    payload: CheckVersionsPayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        assignment_count = payload.assignments.len(),
        "spawning background CheckVersions task for SSH host"
    );
    let items = payload
        .assignments
        .iter()
        .map(|a| a.software_item_id)
        .collect();
    let db = db.clone();
    let pool = pool.clone();
    // Second clone needed: `host_machine_id` also moves into the async block
    // below (used by the completion trace), so it cannot be borrowed here in
    // the same call — E0505.
    let guard_host = host_machine_id.clone();
    uptrakit_agent_core::spawn_background_guarded(
        ops,
        &guard_host,
        uptrakit_agent_core::BgOpKind::CheckVersions,
        uptrakit_shared_types::op_timeouts::VERSION_CHECK_OP_TIMEOUT,
        Some(items),
        bg_tx,
        async move {
            let msg = run_check_versions_ssh(payload, &db, &pool).await;
            tracing::debug!(host_machine_id = %host_machine_id, "background CheckVersions task completed");
            msg
        },
    );
}

/// Run `CheckVersions` for an SSH host, returning the result as a
/// [`ServiceMessage`].
async fn run_check_versions_ssh(
    payload: CheckVersionsPayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
) -> ServiceMessage {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                "no SSH host found for CheckVersions host_machine_id; returning errors"
            );
            let results = error_results_for_check(
                &payload,
                &format!(
                    "SSH host with machine_id '{}' not found",
                    payload.host_machine_id
                ),
            );
            return ServiceMessage::VersionCheckResults(VersionCheckResultsPayload { results });
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                error = %e,
                "DB error looking up SSH host for CheckVersions"
            );
            let results = error_results_for_check(&payload, &format!("DB error: {e}"));
            return ServiceMessage::VersionCheckResults(VersionCheckResultsPayload { results });
        }
    };

    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                error = %e,
                "failed to acquire SSH session for CheckVersions"
            );
            pool.evict(host.id).await;
            let results = error_results_for_check(&payload, &format!("SSH connection failed: {e}"));
            return ServiceMessage::VersionCheckResults(VersionCheckResultsPayload { results });
        }
    };

    let ctx = build_connection_context();
    let raw: Arc<dyn CommandExecutor> =
        Arc::new(PosixSshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        "running version check on SSH host"
    );
    let runtime =
        build_host_runtime(host.id, Arc::clone(&session), Arc::clone(&executor), db).await;
    uptrakit_agent_core::run_check_versions(payload, runtime, &ctx).await
}

/// Spawn a `DiscoverSoftware` operation as a background task.
pub fn spawn_discover_software_ssh(
    ops: &uptrakit_agent_core::BackgroundOps,
    payload: DiscoverSoftwarePayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        plugin_count = payload.plugins.len(),
        "spawning background DiscoverSoftware task for SSH host"
    );
    let db = db.clone();
    let pool = pool.clone();
    // Second clone needed: `host_machine_id` also moves into the async block
    // below (used by the completion trace), so it cannot be borrowed here in
    // the same call — E0505.
    let guard_host = host_machine_id.clone();
    uptrakit_agent_core::spawn_background_guarded(
        ops,
        &guard_host,
        uptrakit_agent_core::BgOpKind::Discovery,
        uptrakit_shared_types::op_timeouts::DISCOVERY_OP_TIMEOUT,
        None,
        bg_tx,
        async move {
            let msg = run_discover_software_ssh(payload, &db, &pool).await;
            tracing::debug!(host_machine_id = %host_machine_id, "background DiscoverSoftware task completed");
            msg
        },
    );
}

/// Run `DiscoverSoftware` for an SSH host, returning the result as a
/// [`ServiceMessage`].
async fn run_discover_software_ssh(
    payload: DiscoverSoftwarePayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
) -> ServiceMessage {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                "no SSH host found for DiscoverSoftware host_machine_id; returning errors"
            );
            let results = error_results_for_discovery(
                &payload,
                &format!(
                    "SSH host with machine_id '{}' not found",
                    payload.host_machine_id
                ),
            );
            return ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            });
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                error = %e,
                "DB error looking up SSH host for DiscoverSoftware"
            );
            let results = error_results_for_discovery(&payload, &format!("DB error: {e}"));
            return ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            });
        }
    };

    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                error = %e,
                "failed to acquire SSH session for DiscoverSoftware"
            );
            pool.evict(host.id).await;
            let results =
                error_results_for_discovery(&payload, &format!("SSH connection failed: {e}"));
            return ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            });
        }
    };

    let ctx = build_connection_context();
    let raw: Arc<dyn CommandExecutor> =
        Arc::new(PosixSshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        "running discovery on SSH host"
    );
    let runtime =
        build_host_runtime(host.id, Arc::clone(&session), Arc::clone(&executor), db).await;
    uptrakit_agent_core::run_discover_software(payload, runtime, &ctx).await
}

/// Spawn an `ExecuteBatchUpdate` operation as a background task.
pub fn spawn_execute_batch_update_ssh(
    payload: ExecuteBatchUpdatePayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        batch_id = %payload.batch_id,
        update_count = payload.updates.len(),
        "spawning background ExecuteBatchUpdate task for SSH host"
    );
    let db = db.clone();
    let pool = pool.clone();
    // Deliberately unguarded: updates have their own overlap protection via
    // `update_history` (see AGENTS.md invariant 9).
    uptrakit_agent_core::spawn_background(bg_tx, async move {
        let msg = run_execute_batch_update_ssh(payload, &db, &pool).await;
        tracing::debug!(host_machine_id = %host_machine_id, "background ExecuteBatchUpdate task completed");
        msg
    });
}

/// Run `ExecuteBatchUpdate` for an SSH host, returning the result
/// as a [`ServiceMessage`].
async fn run_execute_batch_update_ssh(
    payload: ExecuteBatchUpdatePayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
) -> ServiceMessage {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                batch_id = %payload.batch_id,
                "no SSH host found for ExecuteBatchUpdate host_machine_id"
            );
            let results: Vec<BatchUpdateItemResult> = payload
                .updates
                .iter()
                .map(|u| BatchUpdateItemResult {
                    host_software_item_id: u.host_software_item_id,
                    update_history_id: u.update_history_id,
                    status: UpdateFinalStatus::Failed,
                    output: String::new(),
                    installed_version: None,
                    error: Some(format!(
                        "SSH host with machine_id '{}' not found",
                        payload.host_machine_id
                    )),
                })
                .collect();
            return ServiceMessage::BatchUpdateResult(BatchUpdateResultPayload {
                batch_id: payload.batch_id,
                results,
            });
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                batch_id = %payload.batch_id,
                error = %e,
                "DB error looking up SSH host for ExecuteBatchUpdate"
            );
            let results: Vec<BatchUpdateItemResult> = payload
                .updates
                .iter()
                .map(|u| BatchUpdateItemResult {
                    host_software_item_id: u.host_software_item_id,
                    update_history_id: u.update_history_id,
                    status: UpdateFinalStatus::Failed,
                    output: String::new(),
                    installed_version: None,
                    error: Some(format!("DB error: {e}")),
                })
                .collect();
            return ServiceMessage::BatchUpdateResult(BatchUpdateResultPayload {
                batch_id: payload.batch_id,
                results,
            });
        }
    };

    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                batch_id = %payload.batch_id,
                error = %e,
                "failed to acquire SSH session for ExecuteBatchUpdate"
            );
            pool.evict(host.id).await;
            let results: Vec<BatchUpdateItemResult> = payload
                .updates
                .iter()
                .map(|u| BatchUpdateItemResult {
                    host_software_item_id: u.host_software_item_id,
                    update_history_id: u.update_history_id,
                    status: UpdateFinalStatus::Failed,
                    output: String::new(),
                    installed_version: None,
                    error: Some(format!("SSH connection failed: {e}")),
                })
                .collect();
            return ServiceMessage::BatchUpdateResult(BatchUpdateResultPayload {
                batch_id: payload.batch_id,
                results,
            });
        }
    };

    let ctx = build_connection_context();
    let raw: Arc<dyn CommandExecutor> =
        Arc::new(PosixSshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        batch_id = %payload.batch_id,
        "running batch update on SSH host"
    );
    let runtime =
        build_host_runtime(host.id, Arc::clone(&session), Arc::clone(&executor), db).await;
    uptrakit_agent_core::run_execute_batch_update(payload, runtime, &ctx).await
}

// ── Shared re-exports ─────────────────────────────────────────────────────────

pub use uptrakit_agent_core::{send_update_output, send_update_result};

/// Forward stdin data or a signal from the controller to the correct SSH host's
/// in-flight update.
#[cfg(feature = "interactive")]
pub fn handle_update_stdin_data_ssh(
    payload: uptrakit_wire::UpdateStdinDataPayload,
    in_flight_updates: &HashMap<String, SshInFlightUpdate>,
) {
    // Find the update by update_history_id across all hosts.
    let Some((_, update)) = in_flight_updates
        .iter()
        .find(|(_, u)| u.update_history_id == payload.update_history_id)
    else {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "received UpdateStdinData but no matching in-flight update found; ignoring"
        );
        return;
    };

    if let Some(signal) = payload.signal {
        // update.signal_tx is a proxy sender, always live from update start.
        // Pre-resolution sends buffer in the proxy (up to
        // SSH_PROXY_CHANNEL_CAPACITY) and are drained into the real PTY
        // channel by the forwarder once channels_rx resolves.
        if update.signal_tx.try_send(signal).is_err() {
            tracing::warn!("signal channel full or closed; dropping signal {signal}");
        }
    } else {
        use base64::Engine as _;
        match base64::engine::general_purpose::STANDARD.decode(&payload.data) {
            Ok(bytes) => {
                if update.stdin_tx.try_send(bytes).is_err() {
                    tracing::warn!("stdin channel full or closed; dropping stdin data");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to decode base64 stdin data");
            }
        }
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Build a failed `UpdateResult` [`ServiceMessage`] for SSH error paths.
///
/// Used by `handle_execute_update_ssh` for the host-not-found, DB-error,
/// SSH-connection-failed, and concurrent-update-rejected cases to ensure
/// consistent error format across all failure modes.
fn make_ssh_update_error_response(
    update_history_id: uuid::Uuid,
    error_message: String,
) -> ServiceMessage {
    ServiceMessage::UpdateResult(UpdateResultPayload {
        update_history_id,
        status: UpdateFinalStatus::Failed,
        from_version: None,
        to_version: None,
        output: String::new(),
        error: Some(error_message),
        resumable: None,
    })
}

/// Build per-assignment error results for a failed `CheckVersions` message.
fn error_results_for_check(payload: &CheckVersionsPayload, error: &str) -> Vec<VersionCheckResult> {
    payload
        .assignments
        .iter()
        .map(|a| VersionCheckResult {
            software_item_id: a.software_item_id,
            host_software_item_id: a.host_software_item_id,
            installed_version: None,
            installed_display_version: None,
            latest_version: None,
            error: Some(error.to_string()),
            update_category: UpdateCategory::Unknown,
            not_ready: None,
        })
        .collect()
}

/// Build per-plugin error results for a failed `DiscoverSoftware` message.
fn error_results_for_discovery(
    payload: &DiscoverSoftwarePayload,
    error: &str,
) -> Vec<DiscoveryPluginResult> {
    payload
        .plugins
        .iter()
        .map(|a| DiscoveryPluginResult {
            plugin_config_id: a.plugin_config_id,
            plugin_type: a.plugin_type.clone(),
            discoveries: vec![],
            error: Some(error.to_string()),
        })
        .collect()
}

/// Spawn a `TestPluginConfig` operation as a background task for an SSH host.
pub fn spawn_config_test_ssh(
    payload: uptrakit_wire::TestPluginConfigPayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let host_machine_id = payload.host_machine_id.clone();
    tracing::debug!(
        host_machine_id = %host_machine_id,
        request_id = %payload.request_id,
        "spawning background TestPluginConfig task for SSH host"
    );
    let db = db.clone();
    let pool = pool.clone();
    // Request/response with its own correlator and in-op deadline — outside
    // the guard (2026-08-22 spec amendment).
    uptrakit_agent_core::spawn_background(bg_tx, async move {
        run_config_test_ssh(payload, &db, &pool).await
    });
}

/// Run a config test for an SSH host: resolve host, acquire SSH session,
/// delegate to `uptrakit_agent_core::config_test::run_config_test`.
async fn run_config_test_ssh(
    payload: uptrakit_wire::TestPluginConfigPayload,
    db: &sea_orm::DatabaseConnection,
    pool: &SshConnectionPool,
) -> ServiceMessage {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                "no SSH host found for TestPluginConfig; returning error"
            );
            let mut result = uptrakit_wire::TestPluginConfigResultPayload::new(
                payload.request_id.clone(),
                false,
                0,
            );
            result.error = Some(format!(
                "SSH host with machine_id '{}' not found",
                payload.host_machine_id
            ));
            return ServiceMessage::TestPluginConfigResult(result);
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                error = %e,
                "DB error looking up SSH host for TestPluginConfig"
            );
            let mut result = uptrakit_wire::TestPluginConfigResultPayload::new(
                payload.request_id.clone(),
                false,
                0,
            );
            result.error = Some(format!("DB error: {e}"));
            return ServiceMessage::TestPluginConfigResult(result);
        }
    };

    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                error = %e,
                "failed to acquire SSH session for TestPluginConfig"
            );
            pool.evict(host.id).await;
            let mut result = uptrakit_wire::TestPluginConfigResultPayload::new(
                payload.request_id.clone(),
                false,
                0,
            );
            result.error = Some(format!("SSH connection failed: {e}"));
            return ServiceMessage::TestPluginConfigResult(result);
        }
    };

    let raw: Arc<dyn CommandExecutor> =
        Arc::new(PosixSshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    uptrakit_agent_core::config_test::run_config_test(payload, executor).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn make_ssh_in_flight() -> SshInFlightUpdate {
        #[cfg(feature = "interactive")]
        let (stdin_tx, _stdin_rx) =
            tokio::sync::mpsc::channel::<Vec<u8>>(crate::SSH_PROXY_CHANNEL_CAPACITY);
        #[cfg(feature = "interactive")]
        let (signal_tx, _signal_rx) =
            tokio::sync::mpsc::channel::<i32>(crate::SSH_PROXY_CHANNEL_CAPACITY);
        SshInFlightUpdate {
            update_history_id: uuid::Uuid::nil(),
            forwarder: tokio::spawn(std::future::pending()),
            early_sent: false,
            #[cfg(feature = "interactive")]
            stdin_tx,
            #[cfg(feature = "interactive")]
            signal_tx,
            #[cfg(feature = "interactive")]
            resolution: Arc::new(parking_lot::Mutex::new(
                crate::SshInteractiveResolution::pending(),
            )),
        }
    }

    // ── Proxy channel bridging (pinned semantic) ────────────────────────────

    /// Stdin bytes sent into the proxy sender *before* `channels_rx` resolves
    /// must be delivered to the real channel once the forwarder bridges
    /// proxy -> real on resolution — not lost.
    #[cfg(feature = "interactive")]
    #[tokio::test]
    async fn proxy_buffered_stdin_is_delivered_on_bridge() {
        let (stdin_proxy_tx, mut stdin_proxy_rx) =
            tokio::sync::mpsc::channel::<Vec<u8>>(crate::SSH_PROXY_CHANNEL_CAPACITY);
        let (signal_proxy_tx, mut signal_proxy_rx) =
            tokio::sync::mpsc::channel::<i32>(crate::SSH_PROXY_CHANNEL_CAPACITY);

        // Pre-resolution: buffer stdin bytes and a signal into the proxies,
        // exactly as handle_update_stdin_data_ssh would via try_send.
        stdin_proxy_tx
            .try_send(b"hello".to_vec())
            .expect("buffer stdin");
        stdin_proxy_tx
            .try_send(b"world".to_vec())
            .expect("buffer stdin");
        signal_proxy_tx.try_send(2).expect("buffer signal");
        drop(stdin_proxy_tx);
        drop(signal_proxy_tx);

        // Resolution: bridge proxy -> real, draining what's buffered.
        let (real_stdin_tx, mut real_stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
        let (real_signal_tx, mut real_signal_rx) = tokio::sync::mpsc::channel::<i32>(8);
        drain_proxies_into_real_channels(
            &mut stdin_proxy_rx,
            &mut signal_proxy_rx,
            &real_stdin_tx,
            &real_signal_tx,
        );

        assert_eq!(real_stdin_rx.try_recv().expect("first chunk"), b"hello");
        assert_eq!(real_stdin_rx.try_recv().expect("second chunk"), b"world");
        assert!(
            real_stdin_rx.try_recv().is_err(),
            "no further buffered stdin beyond what was sent"
        );
        assert_eq!(real_signal_rx.try_recv().expect("signal"), 2);
    }

    /// Once the proxy fills to `SSH_PROXY_CHANNEL_CAPACITY`, a further
    /// pre-resolution `try_send` errors and drops — no panic, no new drop
    /// path (this is `try_send`'s existing `Err` handling).
    #[cfg(feature = "interactive")]
    #[tokio::test]
    async fn proxy_overflow_beyond_capacity_drops_without_panic() {
        let (stdin_proxy_tx, _stdin_proxy_rx) =
            tokio::sync::mpsc::channel::<Vec<u8>>(crate::SSH_PROXY_CHANNEL_CAPACITY);

        for i in 0..crate::SSH_PROXY_CHANNEL_CAPACITY {
            stdin_proxy_tx
                .try_send(vec![i as u8])
                .expect("buffer up to capacity");
        }

        // One more send beyond capacity must error (dropped), not panic.
        let overflow_result = stdin_proxy_tx.try_send(vec![0xFF]);
        assert!(
            overflow_result.is_err(),
            "try_send beyond SSH_PROXY_CHANNEL_CAPACITY must error, not buffer"
        );
    }

    /// Build a minimal [`Model`] for testing classification and fast-path logic.
    ///
    /// Initializes the crypto master key (no-op if already set) so that
    /// [`EncryptedString::new`] succeeds.
    fn make_test_host(id: uuid::Uuid, hostname: &str, machine_id: Option<&str>) -> Model {
        use crate::db::entity::ssh_host::SshKeyType;
        use uptrakit_crypto::{EncryptedString, init_master_key};
        let _ = init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        Model {
            id,
            name: id.to_string(),
            hostname: hostname.to_string(),
            port: 22,
            username: "uptrakit".to_string(),
            private_key: EncryptedString::new("key".to_string(), "uptrakit:ssh_hosts:private_key")
                .expect("master key initialized above"),
            key_type: SshKeyType::Ed25519,
            host_key_fingerprint: None,
            machine_id: machine_id.map(str::to_string),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
            sudo_available: None,
            is_root: None,
            sudo_policy: "auto".to_string(),
            pve_plugin_config_id: None,
            pve_node_name: None,
        }
    }

    /// A second `ExecuteUpdate` for the **same** host must be rejected while an
    /// update is already in-flight for that host.
    #[tokio::test]
    async fn test_per_host_guard_rejects_same_host() {
        let mut map: HashMap<String, SshInFlightUpdate> = HashMap::new();
        map.insert("host-machine-1".to_string(), make_ssh_in_flight());

        // The guard in handle_execute_update_ssh checks contains_key.
        assert!(
            map.contains_key("host-machine-1"),
            "in-flight update must block a second request for the same host"
        );

        // Clean up background task.
        for (_, update) in map.drain() {
            update.forwarder.abort();
        }
    }

    /// An `ExecuteUpdate` for a **different** host must not be blocked by an
    /// in-flight update on another host.
    #[tokio::test]
    async fn test_per_host_guard_allows_different_host() {
        let mut map: HashMap<String, SshInFlightUpdate> = HashMap::new();
        map.insert("host-machine-1".to_string(), make_ssh_in_flight());

        assert!(
            !map.contains_key("host-machine-2"),
            "a different host must not be blocked by another host's in-flight update"
        );

        // Clean up background task.
        for (_, update) in map.drain() {
            update.forwarder.abort();
        }
    }

    // ── host_needs_ssh ───────────────────────────────────────────────────────

    /// A host with no `machine_id` always needs SSH (new host).
    #[test]
    fn host_needs_ssh_when_machine_id_is_none() {
        let id1 = uuid::Uuid::now_v7();
        let host = make_test_host(id1, "10.0.0.1", None);
        let changed: HashSet<uuid::Uuid> = HashSet::new();
        assert!(
            host_needs_ssh(&host, &changed),
            "host with no machine_id must need SSH"
        );
    }

    /// A host with a known `machine_id` that is also in `changed_ids` needs SSH.
    #[test]
    fn host_needs_ssh_when_in_changed_ids() {
        let id2 = uuid::Uuid::now_v7();
        let host = make_test_host(id2, "10.0.0.2", Some("mid-abc"));
        let mut changed: HashSet<uuid::Uuid> = HashSet::new();
        changed.insert(id2);
        assert!(
            host_needs_ssh(&host, &changed),
            "host in changed_ids must need SSH even if machine_id is known"
        );
    }

    /// A host with a known `machine_id` that is NOT in `changed_ids` takes the
    /// fast path — no SSH required.
    #[test]
    fn host_does_not_need_ssh_when_machine_id_known_and_unchanged() {
        let host = make_test_host(uuid::Uuid::now_v7(), "10.0.0.3", Some("mid-xyz"));
        let changed: HashSet<uuid::Uuid> = HashSet::new();
        assert!(
            !host_needs_ssh(&host, &changed),
            "host with known machine_id not in changed_ids must skip SSH"
        );
    }

    /// A different host being in `changed_ids` must not affect this host.
    #[test]
    fn host_does_not_need_ssh_when_different_host_changed() {
        let host = make_test_host(uuid::Uuid::now_v7(), "10.0.0.4", Some("mid-def"));
        let mut changed: HashSet<uuid::Uuid> = HashSet::new();
        changed.insert(uuid::Uuid::now_v7()); // different host
        assert!(
            !host_needs_ssh(&host, &changed),
            "an unrelated entry in changed_ids must not affect this host"
        );
    }

    // ── build_fast_path_host_info ────────────────────────────────────────────

    /// Fast-path `HostInfo` must carry the persisted `machine_id` and SSH address.
    #[test]
    fn fast_path_host_info_fields() {
        let host = make_test_host(uuid::Uuid::now_v7(), "192.168.1.10", Some("machine-id-99"));
        let info = build_fast_path_host_info(&host);

        assert_eq!(info.machine_id, "machine-id-99");
        assert_eq!(info.ip_address.as_deref(), Some("192.168.1.10"));
        assert!(info.os_type.is_none(), "os_type must be None on fast path");
        assert!(
            info.os_version.is_none(),
            "os_version must be None on fast path"
        );
        assert!(
            info.architecture.is_none(),
            "architecture must be None on fast path"
        );
        assert!(
            info.hostname.is_none(),
            "hostname must be None on fast path"
        );
    }

    /// Fast-path `HostInfo` falls back to empty string when `machine_id` is
    /// `None` (defensive belt-and-suspenders — `host_needs_ssh` should prevent
    /// this branch from being reached in practice).
    #[test]
    fn fast_path_host_info_machine_id_none_fallback() {
        let host = make_test_host(uuid::Uuid::now_v7(), "192.168.1.11", None);
        let info = build_fast_path_host_info(&host);
        assert_eq!(
            info.machine_id, "",
            "machine_id must fall back to empty string when None"
        );
    }
}
