use std::sync::Arc;
use std::time::Duration;

use uptrakit_command::{CommandExecutor, CommandSpec};
use std::collections::BTreeSet;

use uptrakit_internal_wire::{
    Capability, CheckVersionsPayload, DiscoverSoftwarePayload, DiscoveryProviderResult,
    DiscoveryResultsPayload, ExecuteUpdatePayload, HostInfo, ReportHostsPayload, ServiceMessage,
    UpdateFinalStatus, UpdateResultPayload, VersionCheckResult, VersionCheckResultsPayload,
};
use uptrakit_service_sdk::{ControllerConnection, LoopOutcome};

use crate::host_info::collect_remote_host_info;
use crate::host_ops::{find_host_by_machine_id, list_hosts, update_host_machine_id};
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig};

// Re-export shared update types for use in main.rs.
pub(crate) use uptrakit_agent_core::{InFlightUpdate, UpdateEvent};

/// Connect to each enrolled SSH host, collect system info, and send a
/// `ReportHosts` message to the controller.
///
/// Errors for individual hosts are logged as warnings and skipped.
pub(crate) async fn report_enrolled_hosts(
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

        // Wrap the session in Arc so it can be shared with the SshCommandExecutor.
        // The executor is dropped at end of the verification block so that
        // Arc::try_unwrap succeeds later for the disconnect call.
        let session = Arc::new(session);

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
                "SSH command executor check failed, skipping host"
            );
            continue;
        }

        let mut info = collect_remote_host_info(&session).await;
        // Set the SSH target address as the host's ip_address.
        info.ip_address = Some(host.hostname.clone());

        // Persist the machine_id so incoming CheckVersions / ExecuteUpdate
        // messages can be routed to this host via find_host_by_machine_id().
        if let Err(e) =
            update_host_machine_id(local_db, &host.id, &info.machine_id).await
        {
            tracing::warn!(
                host_name = %host.name,
                machine_id = %info.machine_id,
                error = %e,
                "failed to persist machine_id for SSH host"
            );
        }

        // executor was dropped above so the Arc has exactly one strong
        // reference; try_unwrap gives us ownership for the disconnect call.
        if let Ok(owned) = Arc::try_unwrap(session) {
            owned.disconnect().await;
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

/// Establish an SSH session for the given host model.
async fn establish_ssh_session(
    host: &crate::db::entity::ssh_host::Model,
) -> Result<Arc<crate::ssh_transport::SshSession>, String> {
    let config = SshConnectionConfig {
        hostname: host.hostname.clone(),
        port: host.port as u16,
        connect_timeout: Duration::from_secs(30),
    };
    let private_key_pem = host.private_key.expose_secret();
    let auth = AuthMethod::PrivateKey(private_key_pem);

    let (session, _fingerprint) = crate::ssh_transport::connect_and_authenticate(
        &config,
        &host.username,
        &auth,
        host.host_key_fingerprint.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(Arc::new(session))
}

/// Handle a `CheckVersions` message for the SSH agent.
///
/// Looks up the SSH host by `host_machine_id`, opens a session, and delegates
/// to the shared `uptrakit_agent_core::handle_check_versions()`.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_check_versions_ssh(
    payload: CheckVersionsPayload,
    db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
) -> Option<LoopOutcome> {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                "no SSH host found for CheckVersions host_machine_id; returning errors"
            );
            // Send error results for all assignments.
            let results = payload
                .assignments
                .iter()
                .map(|a| VersionCheckResult {
                    software_item_id: a.software_item_id,
                    installed_version: None,
                    latest_version: None,
                    error: Some(format!(
                        "SSH host with machine_id '{}' not found",
                        payload.host_machine_id
                    )),
                })
                .collect();
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
            let results = payload
                .assignments
                .iter()
                .map(|a| VersionCheckResult {
                    software_item_id: a.software_item_id,
                    installed_version: None,
                    latest_version: None,
                    error: Some(format!("DB error: {e}")),
                })
                .collect();
            conn.send_best_effort(ServiceMessage::VersionCheckResults(
                VersionCheckResultsPayload { results },
            ))
            .await;
            return None;
        }
    };

    let session = match establish_ssh_session(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                error = %e,
                "failed to establish SSH session for CheckVersions"
            );
            let results = payload
                .assignments
                .iter()
                .map(|a| VersionCheckResult {
                    software_item_id: a.software_item_id,
                    installed_version: None,
                    latest_version: None,
                    error: Some(format!("SSH connection failed: {e}")),
                })
                .collect();
            conn.send_best_effort(ServiceMessage::VersionCheckResults(
                VersionCheckResultsPayload { results },
            ))
            .await;
            return None;
        }
    };

    let executor: Arc<dyn CommandExecutor> =
        Arc::new(SshCommandExecutor::new(Arc::clone(&session)));

    let outcome =
        uptrakit_agent_core::handle_check_versions(payload, executor, conn).await;

    // Disconnect session after version check completes.
    if let Ok(owned) = Arc::try_unwrap(session) {
        owned.disconnect().await;
    }

    outcome
}

/// Handle an `ExecuteUpdate` message for the SSH agent.
///
/// Looks up the SSH host by `host_machine_id`, opens a session, and delegates
/// to the shared `uptrakit_agent_core::handle_execute_update()`.
pub(crate) async fn handle_execute_update_ssh(
    payload: ExecuteUpdatePayload,
    db: &sea_orm::DatabaseConnection,
    in_flight_update: &mut Option<InFlightUpdate>,
    conn: &mut ControllerConnection,
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

    let session = match establish_ssh_session(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                update_id = %payload.update_history_id,
                error = %e,
                "failed to establish SSH session for ExecuteUpdate"
            );
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

    // The session is kept alive for the duration of the update via the
    // SshCommandExecutor Arc. The session will be disconnected when the
    // executor is dropped after update completion.
    let executor: Arc<dyn CommandExecutor> =
        Arc::new(SshCommandExecutor::new(Arc::clone(&session)));

    uptrakit_agent_core::handle_execute_update(payload, executor, in_flight_update, conn).await;

    // Note: the SSH session remains open while the update is in-flight.
    // It is disconnected when the executor is dropped after the spawned task
    // completes. This is safe because SshCommandExecutor holds an Arc<SshSession>.
    // We intentionally do NOT try_unwrap here since the executor Arc was moved
    // into the spawned task.
}

/// Handle a `DiscoverSoftware` message for the SSH agent.
///
/// Looks up the SSH host by `host_machine_id`, opens a session, and delegates
/// to the shared `uptrakit_agent_core::handle_discover_software()`. SSH
/// connection failures are reported as per-provider errors rather than
/// aborting the entire run.
///
/// Returns `Some(LoopOutcome::Disconnected)` if the response send fails.
pub(crate) async fn handle_discover_software_ssh(
    payload: DiscoverSoftwarePayload,
    db: &sea_orm::DatabaseConnection,
    conn: &mut ControllerConnection,
) -> Option<LoopOutcome> {
    let host = match find_host_by_machine_id(db, &payload.host_machine_id).await {
        Ok(Some(h)) => h,
        Ok(None) => {
            tracing::warn!(
                host_machine_id = %payload.host_machine_id,
                "no SSH host found for DiscoverSoftware host_machine_id; returning errors"
            );
            let results = payload
                .providers
                .iter()
                .map(|a| DiscoveryProviderResult {
                    provider_config_id: a.provider_config_id,
                    provider_type: a.provider_type.clone(),
                    discoveries: vec![],
                    error: Some(format!(
                        "SSH host with machine_id '{}' not found",
                        payload.host_machine_id
                    )),
                })
                .collect();
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
            let results = payload
                .providers
                .iter()
                .map(|a| DiscoveryProviderResult {
                    provider_config_id: a.provider_config_id,
                    provider_type: a.provider_type.clone(),
                    discoveries: vec![],
                    error: Some(format!("DB error: {e}")),
                })
                .collect();
            conn.send_best_effort(ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            }))
            .await;
            return None;
        }
    };

    let session = match establish_ssh_session(&host).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                host_name = %host.name,
                error = %e,
                "failed to establish SSH session for DiscoverSoftware"
            );
            let results = payload
                .providers
                .iter()
                .map(|a| DiscoveryProviderResult {
                    provider_config_id: a.provider_config_id,
                    provider_type: a.provider_type.clone(),
                    discoveries: vec![],
                    error: Some(format!("SSH connection failed: {e}")),
                })
                .collect();
            conn.send_best_effort(ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: payload.host_machine_id,
                results,
            }))
            .await;
            return None;
        }
    };

    let executor: Arc<dyn CommandExecutor> =
        Arc::new(SshCommandExecutor::new(Arc::clone(&session)));

    let outcome =
        uptrakit_agent_core::handle_discover_software(payload, executor, conn).await;

    if let Ok(owned) = Arc::try_unwrap(session) {
        owned.disconnect().await;
    }

    outcome
}

pub(crate) use uptrakit_agent_core::{
    handle_graceful_shutdown, send_update_output, send_update_result,
};
