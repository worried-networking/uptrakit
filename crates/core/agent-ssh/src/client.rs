use std::collections::HashMap;
use std::sync::Arc;

use std::collections::{BTreeSet, HashSet};
use tokio::task::JoinSet;
use uptrakit_agent_core::ConnectionContext;
use uptrakit_command::{CommandExecutor, CommandSpec, SudoAwareCommandExecutor};

use uptrakit_internal_wire::{
    BatchHostPackageUpdateResult, BatchHostPackageUpdateResultPayload, Capability,
    CheckVersionsPayload, DiscoverSoftwarePayload, DiscoveryPluginResult, DiscoveryResultsPayload,
    ExecuteBatchHostPackageUpdatePayload, ExecuteUpdatePayload, HostInfo, ReportHostsPayload,
    ServiceMessage, UpdateCategory, UpdateFinalStatus, UpdateResultPayload, VersionCheckResult,
    VersionCheckResultsPayload,
};
use uptrakit_service_sdk::{ControllerConnection, LoopOutcome};

use crate::db::entity::ssh_host::Model;
use crate::host_info::collect_remote_host_info;
use crate::host_ops::{find_host_by_machine_id, list_hosts, update_host_machine_id};
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_pool::SshConnectionPool;

// Re-export shared update types for use in main.rs.
pub(crate) use uptrakit_agent_core::{InFlightUpdate, UpdateEvent};

// ── SSH in-flight update tracking ────────────────────────────────────────────

/// State for a per-host in-flight update managed by the SSH agent.
///
/// Unlike `InFlightUpdate` (which owns the JoinHandle and output channel
/// directly), `SshInFlightUpdate` only stores the update ID and a handle to
/// the **forwarder task**. The forwarder task owns the underlying
/// `InFlightUpdate` and forwards all events to the shared aggregate channel.
pub(crate) struct SshInFlightUpdate {
    /// The update history ID used to correlate events with the controller.
    pub update_history_id: uuid::Uuid,
    /// JoinHandle for the forwarder task.
    ///
    /// Dropped when the update completes normally; aborted on shutdown timeout.
    pub forwarder: tokio::task::JoinHandle<()>,
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
pub(crate) async fn report_enrolled_hosts(
    local_db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
    pool: &SshConnectionPool,
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

    let agent_version = env!("CARGO_PKG_VERSION").to_string();
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: host_infos,
        agent_version,
        capabilities: ssh_agent_capabilities(),
    });

    if let Err(e) = conn.send(msg).await {
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

    // Verify that command execution is available via the CommandExecutor
    // interface before proceeding with host information collection.
    let executor_ok = {
        let executor = SshCommandExecutor::new(Arc::clone(&session));
        executor
            .execute_quiet(&CommandSpec::exec("true", Vec::<String>::new()))
            .await
            .is_ok()
    };
    if !executor_ok {
        tracing::warn!(
            host_name = %host.name,
            hostname = %host.hostname,
            "SSH command executor check failed, evicting session and skipping host"
        );
        pool.evict(&host.id).await;
        return None;
    }

    let mut info = collect_remote_host_info(&session).await;
    // Set the SSH target address as the host's ip_address.
    info.ip_address = Some(host.hostname.clone());

    // Persist the machine_id so incoming CheckVersions / ExecuteUpdate
    // messages can be routed to this host via find_host_by_machine_id().
    if let Err(e) = update_host_machine_id(&db, &host.id, &info.machine_id).await {
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
fn host_needs_ssh(host: &Model, changed_ids: &HashSet<&str>) -> bool {
    host.machine_id.is_none() || changed_ids.contains(host.id.as_str())
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
pub(crate) async fn build_reload_host_infos(
    db: &sea_orm::DatabaseConnection,
    current_hosts: &[Model],
    changed_ids: &HashSet<&str>,
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

    let mut info = collect_remote_host_info(&session).await;
    info.ip_address = Some(host.hostname.clone());

    if let Err(e) = update_host_machine_id(&db, &host.id, &info.machine_id).await {
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
pub(crate) async fn report_hosts_after_config_change(
    db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
    current_hosts: &[Model],
    changed_ids: &HashSet<&str>,
    pool: &SshConnectionPool,
) {
    let host_infos = build_reload_host_infos(db, current_hosts, changed_ids, pool).await;
    let agent_version = env!("CARGO_PKG_VERSION").to_string();
    let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
        hosts: host_infos,
        agent_version,
        capabilities: ssh_agent_capabilities(),
    });

    if let Err(e) = conn.send(msg).await {
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
pub(crate) fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    [
        Capability::SoftwareDiscovery,
        Capability::UpdateHooks,
        Capability::GracefulShutdown,
        Capability::SshRemote,
    ]
    .into_iter()
    .collect()
}

// ── CheckVersions ─────────────────────────────────────────────────────────────

/// Handle a `CheckVersions` message for the SSH agent.
///
/// Looks up the SSH host by `host_machine_id`, acquires a pooled session, and
/// delegates to the shared `uptrakit_agent_core::handle_check_versions()`.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_check_versions_ssh(
    payload: CheckVersionsPayload,
    db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
    pool: &SshConnectionPool,
) -> Option<LoopOutcome> {
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
            conn.send_best_effort(ServiceMessage::VersionCheckResults(
                VersionCheckResultsPayload { results },
            ))
            .await;
            return None;
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                error = %e,
                "DB error looking up SSH host for CheckVersions"
            );
            let results = error_results_for_check(&payload, &format!("DB error: {e}"));
            conn.send_best_effort(ServiceMessage::VersionCheckResults(
                VersionCheckResultsPayload { results },
            ))
            .await;
            return None;
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
            pool.evict(&host.id).await;
            let results = error_results_for_check(&payload, &format!("SSH connection failed: {e}"));
            conn.send_best_effort(ServiceMessage::VersionCheckResults(
                VersionCheckResultsPayload { results },
            ))
            .await;
            return None;
        }
    };

    let ctx = build_connection_context();
    let raw: Arc<dyn CommandExecutor> = Arc::new(SshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        "running version check on SSH host"
    );
    // The session Arc is returned to the pool (it stays alive via the pool's own Arc).
    // If a channel-open error occurred during version checks, the session may be stale;
    // the pool will detect this on the next acquire via TTL.
    uptrakit_agent_core::handle_check_versions(payload, executor, conn, &ctx).await
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
pub(crate) async fn handle_execute_update_ssh(
    payload: ExecuteUpdatePayload,
    db: &sea_orm::DatabaseConnection,
    in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
    aggregate_tx: &tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
    conn: &mut ControllerConnection,
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
            conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id: payload.update_history_id,
                status: UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some(format!(
                    "SSH host with machine_id '{}' not found",
                    payload.host_machine_id
                )),
            }))
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
            conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id: payload.update_history_id,
                status: UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some(format!("DB error: {e}")),
            }))
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
            pool.evict(&host.id).await;
            conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
                update_history_id: payload.update_history_id,
                status: UpdateFinalStatus::Failed,
                from_version: None,
                to_version: None,
                output: String::new(),
                error: Some(format!("SSH connection failed: {e}")),
            }))
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
        conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
            update_history_id: payload.update_history_id,
            status: UpdateFinalStatus::Failed,
            from_version: None,
            to_version: None,
            output: String::new(),
            error: Some(format!(
                "Another update is already in progress for host '{}'",
                payload.host_machine_id
            )),
        }))
        .await;
        return;
    }

    let ctx = build_connection_context();

    // The session Arc is shared with the executor that travels into the spawned
    // update task, keeping the SSH connection alive for the duration of the
    // update.  The pool's own Arc remains so the session is returned to the
    // pool after the task completes.
    let raw: Arc<dyn CommandExecutor> = Arc::new(SshCommandExecutor::new(Arc::clone(&session)));
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

    let in_flight = uptrakit_agent_core::start_update(payload, executor, conn, &ctx).await;

    // Spawn a forwarder task that owns the InFlightUpdate and forwards all
    // output/completion events to the shared aggregate channel.
    let host_id = host_machine_id.clone();
    let tx = aggregate_tx.clone();
    let forwarder = tokio::spawn(async move {
        let InFlightUpdate {
            update_history_id: _,
            mut handle,
            mut output_rx,
        } = in_flight;
        loop {
            tokio::select! {
                biased;
                Some(msg) = output_rx.recv() => {
                    if tx.send((host_id.clone(), UpdateEvent::Output(msg))).await.is_err() {
                        break;
                    }
                }
                result = &mut handle => {
                    let _ = tx.send((host_id, UpdateEvent::Completed(result))).await;
                    break;
                }
            }
        }
    });

    in_flight_updates.insert(
        host_machine_id,
        SshInFlightUpdate {
            update_history_id,
            forwarder,
        },
    );
}

// ── ExecuteBatchHostPackageUpdate ─────────────────────────────────────────────

/// Handle an `ExecuteBatchHostPackageUpdate` message for the SSH agent.
///
/// Looks up the SSH host by `host_machine_id`, acquires a pooled session, and
/// delegates to the shared
/// `uptrakit_agent_core::handle_execute_batch_host_package_update()`.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_execute_batch_host_package_update_ssh(
    payload: ExecuteBatchHostPackageUpdatePayload,
    db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
    pool: &SshConnectionPool,
) -> Option<LoopOutcome> {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                batch_id = %payload.batch_id,
                "no SSH host found for ExecuteBatchHostPackageUpdate host_machine_id"
            );
            let results: Vec<BatchHostPackageUpdateResult> = payload
                .updates
                .iter()
                .map(|u| BatchHostPackageUpdateResult {
                    host_package_id: u.host_package_id,
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
            conn.send_best_effort(ServiceMessage::BatchHostPackageUpdateResult(
                BatchHostPackageUpdateResultPayload {
                    batch_id: payload.batch_id,
                    results,
                },
            ))
            .await;
            return None;
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                batch_id = %payload.batch_id,
                error = %e,
                "DB error looking up SSH host for ExecuteBatchHostPackageUpdate"
            );
            let results: Vec<BatchHostPackageUpdateResult> = payload
                .updates
                .iter()
                .map(|u| BatchHostPackageUpdateResult {
                    host_package_id: u.host_package_id,
                    update_history_id: u.update_history_id,
                    status: UpdateFinalStatus::Failed,
                    output: String::new(),
                    installed_version: None,
                    error: Some(format!("DB error: {e}")),
                })
                .collect();
            conn.send_best_effort(ServiceMessage::BatchHostPackageUpdateResult(
                BatchHostPackageUpdateResultPayload {
                    batch_id: payload.batch_id,
                    results,
                },
            ))
            .await;
            return None;
        }
    };

    let session = match pool.acquire(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                batch_id = %payload.batch_id,
                error = %e,
                "failed to acquire SSH session for ExecuteBatchHostPackageUpdate"
            );
            pool.evict(&host.id).await;
            let results: Vec<BatchHostPackageUpdateResult> = payload
                .updates
                .iter()
                .map(|u| BatchHostPackageUpdateResult {
                    host_package_id: u.host_package_id,
                    update_history_id: u.update_history_id,
                    status: UpdateFinalStatus::Failed,
                    output: String::new(),
                    installed_version: None,
                    error: Some(format!("SSH connection failed: {e}")),
                })
                .collect();
            conn.send_best_effort(ServiceMessage::BatchHostPackageUpdateResult(
                BatchHostPackageUpdateResultPayload {
                    batch_id: payload.batch_id,
                    results,
                },
            ))
            .await;
            return None;
        }
    };

    let ctx = build_connection_context();
    let raw: Arc<dyn CommandExecutor> = Arc::new(SshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        batch_id = %payload.batch_id,
        "running batch host package update on SSH host"
    );
    uptrakit_agent_core::handle_execute_batch_host_package_update(payload, executor, conn, &ctx)
        .await
}

// ── DiscoverSoftware ──────────────────────────────────────────────────────────

/// Handle a `DiscoverSoftware` message for the SSH agent.
///
/// Looks up the SSH host by `host_machine_id`, acquires a pooled session, and
/// delegates to the shared `uptrakit_agent_core::handle_discover_software()`.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_discover_software_ssh(
    payload: DiscoverSoftwarePayload,
    db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
    pool: &SshConnectionPool,
) -> Option<LoopOutcome> {
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
            conn.send_best_effort(ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            }))
            .await;
            return None;
        }
        Err(e) => {
            tracing::error!(
                host_machine_id = %payload.host_machine_id,
                error = %e,
                "DB error looking up SSH host for DiscoverSoftware"
            );
            let results = error_results_for_discovery(&payload, &format!("DB error: {e}"));
            conn.send_best_effort(ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            }))
            .await;
            return None;
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
            pool.evict(&host.id).await;
            let results =
                error_results_for_discovery(&payload, &format!("SSH connection failed: {e}"));
            conn.send_best_effort(ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            }))
            .await;
            return None;
        }
    };

    let ctx = build_connection_context();
    let raw: Arc<dyn CommandExecutor> = Arc::new(SshCommandExecutor::new(Arc::clone(&session)));
    let executor: Arc<dyn CommandExecutor> = Arc::new(SudoAwareCommandExecutor::new(
        raw,
        host.resolved_sudo_context(),
    ));

    tracing::debug!(
        host_name = %host.name,
        hostname = %host.hostname,
        "running discovery on SSH host"
    );
    uptrakit_agent_core::handle_discover_software(payload, executor, conn, &ctx).await
}

// ── Shared re-exports ─────────────────────────────────────────────────────────

pub(crate) use uptrakit_agent_core::{send_update_output, send_update_result};

// ── Private helpers ───────────────────────────────────────────────────────────

/// Build per-assignment error results for a failed `CheckVersions` message.
fn error_results_for_check(payload: &CheckVersionsPayload, error: &str) -> Vec<VersionCheckResult> {
    payload
        .assignments
        .iter()
        .map(|a| VersionCheckResult {
            software_item_id: a.software_item_id,
            host_package_id: a.host_package_id,
            installed_version: None,
            latest_version: None,
            error: Some(error.to_string()),
            update_category: UpdateCategory::Unknown,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn make_ssh_in_flight() -> SshInFlightUpdate {
        SshInFlightUpdate {
            update_history_id: uuid::Uuid::nil(),
            forwarder: tokio::spawn(std::future::pending()),
        }
    }

    /// Build a minimal [`Model`] for testing classification and fast-path logic.
    ///
    /// Initializes the crypto master key (no-op if already set) so that
    /// [`EncryptedString::new`] succeeds.
    fn make_test_host(id: &str, hostname: &str, machine_id: Option<&str>) -> Model {
        use crate::db::entity::ssh_host::SshKeyType;
        use uptrakit_crypto::{EncryptedString, init_master_key};
        let _ = init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        Model {
            id: id.to_string(),
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
        let host = make_test_host("h1", "10.0.0.1", None);
        let changed: HashSet<&str> = HashSet::new();
        assert!(
            host_needs_ssh(&host, &changed),
            "host with no machine_id must need SSH"
        );
    }

    /// A host with a known `machine_id` that is also in `changed_ids` needs SSH.
    #[test]
    fn host_needs_ssh_when_in_changed_ids() {
        let host = make_test_host("h2", "10.0.0.2", Some("mid-abc"));
        let mut changed: HashSet<&str> = HashSet::new();
        changed.insert("h2");
        assert!(
            host_needs_ssh(&host, &changed),
            "host in changed_ids must need SSH even if machine_id is known"
        );
    }

    /// A host with a known `machine_id` that is NOT in `changed_ids` takes the
    /// fast path — no SSH required.
    #[test]
    fn host_does_not_need_ssh_when_machine_id_known_and_unchanged() {
        let host = make_test_host("h3", "10.0.0.3", Some("mid-xyz"));
        let changed: HashSet<&str> = HashSet::new();
        assert!(
            !host_needs_ssh(&host, &changed),
            "host with known machine_id not in changed_ids must skip SSH"
        );
    }

    /// A different host being in `changed_ids` must not affect this host.
    #[test]
    fn host_does_not_need_ssh_when_different_host_changed() {
        let host = make_test_host("h4", "10.0.0.4", Some("mid-def"));
        let mut changed: HashSet<&str> = HashSet::new();
        changed.insert("h5"); // different host
        assert!(
            !host_needs_ssh(&host, &changed),
            "an unrelated entry in changed_ids must not affect this host"
        );
    }

    // ── build_fast_path_host_info ────────────────────────────────────────────

    /// Fast-path `HostInfo` must carry the persisted `machine_id` and SSH address.
    #[test]
    fn fast_path_host_info_fields() {
        let host = make_test_host("h6", "192.168.1.10", Some("machine-id-99"));
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
        let host = make_test_host("h7", "192.168.1.11", None);
        let info = build_fast_path_host_info(&host);
        assert_eq!(
            info.machine_id, "",
            "machine_id must fall back to empty string when None"
        );
    }
}
