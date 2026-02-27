use std::path::PathBuf;
use std::sync::Arc;

use std::collections::BTreeSet;
use uptrakit_agent_core::ConnectionContext;
use uptrakit_command::{CommandExecutor, CommandSpec, SudoAwareCommandExecutor};

use uptrakit_internal_wire::{
    Capability, CheckVersionsPayload, DiscoverSoftwarePayload, DiscoveryPluginResult,
    DiscoveryResultsPayload, ExecuteUpdatePayload, HostInfo, ReportHostsPayload, ServiceMessage,
    UpdateFinalStatus, UpdateResultPayload, VersionCheckResult, VersionCheckResultsPayload,
};
use uptrakit_service_sdk::{ControllerConnection, LoopOutcome};

use crate::db::entity::ssh_host::Model;
use crate::host_info::collect_remote_host_info;
use crate::host_ops::{find_host_by_machine_id, list_hosts, update_host_machine_id};
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_pool::SshConnectionPool;

use std::collections::HashSet;

// Re-export shared update types for use in main.rs.
pub(crate) use uptrakit_agent_core::{InFlightUpdate, UpdateEvent};

// ── Temporary key file ────────────────────────────────────────────────────────

/// RAII wrapper that deletes the SSH private key file on drop.
///
/// Written to a temporary path with 0o600 permissions by
/// [`build_connection_context`] so bollard's SSH transport can authenticate
/// with the stored per-host key.  Stored as `Arc<SecureKeyFile>` inside
/// [`ConnectionContext::keep_alive`] so the file persists for the full
/// duration of the operation (including spawned update tasks).
struct SecureKeyFile {
    path: PathBuf,
}

impl Drop for SecureKeyFile {
    fn drop(&mut self) {
        // Best-effort cleanup — log on failure but do not panic.
        if let Err(e) = std::fs::remove_file(&self.path) {
            tracing::warn!(
                path = %self.path.display(),
                error = %e,
                "failed to remove temporary SSH key file"
            );
        }
    }
}

// ── Connection context ────────────────────────────────────────────────────────

/// Build a [`ConnectionContext`] for the given SSH host.
///
/// Decrypts the host's private key and writes it to a temporary file at
/// `$TMPDIR/uptrakit-ssh-key-<host_id>` with 0o600 permissions.  The file
/// is deleted when the last clone of the returned `ConnectionContext` is
/// dropped (via [`ConnectionContext::keep_alive`]).
///
/// The `docker_host_override` is set to `ssh://user@host:port` so bollard
/// connects to the remote Docker daemon via the system `ssh` binary.  The
/// `ssh_key_path` is populated with the temporary file path so bollard can
/// authenticate using the stored key rather than falling back to default
/// SSH key locations.
async fn build_connection_context(host: &Model) -> ConnectionContext {
    let key_path =
        std::env::temp_dir().join(format!("uptrakit-ssh-key-{}", host.id.replace('-', "")));

    let pem_bytes = host.private_key.expose_secret().as_bytes().to_vec();

    match uptrakit_directories::write_secure_file(&key_path, &pem_bytes).await {
        Ok(()) => {
            let keep: Arc<dyn std::any::Any + Send + Sync> = Arc::new(SecureKeyFile {
                path: key_path.clone(),
            });
            ConnectionContext {
                docker_host_override: Some(format!(
                    "ssh://{}@{}:{}",
                    host.username, host.hostname, host.port
                )),
                ssh_key_path: Some(key_path),
                keep_alive: vec![keep],
            }
        }
        Err(e) => {
            tracing::warn!(
                host_name = %host.name,
                error = %e,
                "failed to write temporary SSH key file; Docker operations \
                 on this host may not work (bollard will fall back to default key locations)"
            );
            ConnectionContext {
                docker_host_override: Some(format!(
                    "ssh://{}@{}:{}",
                    host.username, host.hostname, host.port
                )),
                ssh_key_path: None,
                keep_alive: vec![],
            }
        }
    }
}

// ── ReportHosts ───────────────────────────────────────────────────────────────

/// Connect to each enrolled SSH host, collect system info, and send a
/// `ReportHosts` message to the controller.
///
/// Errors for individual hosts are logged as warnings and skipped.
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

    let mut host_infos: Vec<HostInfo> = Vec::with_capacity(hosts.len());

    for host in &hosts {
        tracing::debug!(host_name = %host.name, hostname = %host.hostname, "collecting host info");

        let session = match pool.acquire(host).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    host_name = %host.name,
                    hostname = %host.hostname,
                    error = %e,
                    "failed to acquire SSH session for reporting, skipping"
                );
                continue;
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
            continue;
        }

        let mut info = collect_remote_host_info(&session).await;
        // Set the SSH target address as the host's ip_address.
        info.ip_address = Some(host.hostname.clone());

        // Persist the machine_id so incoming CheckVersions / ExecuteUpdate
        // messages can be routed to this host via find_host_by_machine_id().
        if let Err(e) = update_host_machine_id(local_db, &host.id, &info.machine_id).await {
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

        host_infos.push(info);
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
        tracing::info!(
            host_count = hosts.len(),
            "reported enrolled hosts to controller"
        );
    }
}

// ── Dynamic host reload ───────────────────────────────────────────────────────

/// Build `HostInfo` entries for the current host list, using SSH only where
/// necessary.
///
/// For hosts with a known, non-empty `machine_id` that are **not** in
/// `changed_ids` (neither new nor updated since the last reload), host info is
/// built directly from the database values — no SSH connection is made.
///
/// For hosts with an empty `machine_id` or whose `id` is in `changed_ids`
/// (new or recently updated), the pool is used to acquire an SSH session,
/// remote system info is collected, and the `machine_id` is persisted to the
/// database.  Hosts that fail to connect are skipped with a warning.
pub(crate) async fn build_reload_host_infos(
    db: &sea_orm::DatabaseConnection,
    current_hosts: &[Model],
    changed_ids: &HashSet<&str>,
    pool: &SshConnectionPool,
) -> Vec<HostInfo> {
    let mut host_infos: Vec<HostInfo> = Vec::with_capacity(current_hosts.len());

    for host in current_hosts {
        let needs_ssh = host.machine_id.is_empty() || changed_ids.contains(host.id.as_str());

        if needs_ssh {
            // SSH-connect to discover or refresh machine_id and OS info.
            let session = match pool.acquire(host).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        host_name = %host.name,
                        hostname = %host.hostname,
                        error = %e,
                        "failed to acquire SSH session during dynamic reload, skipping host"
                    );
                    continue;
                }
            };

            let mut info = collect_remote_host_info(&session).await;
            info.ip_address = Some(host.hostname.clone());

            if let Err(e) = update_host_machine_id(db, &host.id, &info.machine_id).await {
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

            host_infos.push(info);
        } else {
            // Fast path: build HostInfo from DB values; no SSH round-trip.
            host_infos.push(HostInfo {
                machine_id: host.machine_id.clone(),
                os_type: None,
                os_version: None,
                architecture: None,
                hostname: None,
                ip_address: Some(host.hostname.clone()),
            });
        }
    }

    host_infos
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

    let ctx = build_connection_context(&host).await;
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
/// delegates to the shared `uptrakit_agent_core::handle_execute_update()`.
pub(crate) async fn handle_execute_update_ssh(
    payload: ExecuteUpdatePayload,
    db: &sea_orm::DatabaseConnection,
    in_flight_update: &mut Option<InFlightUpdate>,
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

    let ctx = build_connection_context(&host).await;

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
    uptrakit_agent_core::handle_execute_update(payload, executor, in_flight_update, conn, &ctx)
        .await;
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

    let ctx = build_connection_context(&host).await;
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

pub(crate) use uptrakit_agent_core::{
    handle_graceful_shutdown, send_update_output, send_update_result,
};

// ── Private helpers ───────────────────────────────────────────────────────────

/// Build per-assignment error results for a failed `CheckVersions` message.
fn error_results_for_check(payload: &CheckVersionsPayload, error: &str) -> Vec<VersionCheckResult> {
    payload
        .assignments
        .iter()
        .map(|a| VersionCheckResult {
            software_item_id: a.software_item_id,
            installed_version: None,
            latest_version: None,
            error: Some(error.to_string()),
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
