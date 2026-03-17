//! Embedded SSH agent service for single-tenant controller deployments.
//!
//! When the `embedded-ssh-agent` feature is enabled, the controller can run the
//! SSH agent inside its own process. This eliminates the need for a separate
//! `uptrakit-agent-ssh` binary when managing remote hosts over SSH.
//!
//! The embedded SSH agent:
//! - Manages remote hosts defined in its own local SQLite database
//! - Yields to an external `uptrakit-agent-ssh` with the same app name
//! - Uses in-process mpsc channels instead of WebSocket for transport
//! - Reuses all business logic from `uptrakit-agent-ssh` library

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use uptrakit_agent_ssh::client::{self, SshInFlightUpdate, UpdateEvent};
use uptrakit_agent_ssh::extension::{self, ExtensionContext, InfraActionInvokerImpl};
use uptrakit_agent_ssh::{
    ServiceExtensionProxy, diff_host_snapshots, handle_set_update_freeze, host_ops,
    init_ssh_data_key_ring, reencrypt_ssh_to_v3, register_ssh_column_aad, ssh_pool,
};
use uptrakit_internal_wire::extension::ExtensionActionsPayload;
use uptrakit_internal_wire::{
    ControllerMessage, DisconnectReason, DisconnectingPayload, RegisterPayload, ServiceMessage,
    ServiceTransport, UpdateFinalStatus, UpdateResultPayload,
};
use uptrakit_plugin_infrastructure_core::PluginBase;

use crate::embedded::EmbeddedShutdownTokens;
use crate::embedded::types::EmbeddedTransport;

/// Timeout for graceful shutdown: how long to wait for in-flight updates
/// before abandoning them.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Build the SSH agent capabilities set (delegates to the library).
pub(crate) fn ssh_agent_capabilities()
-> std::collections::BTreeSet<uptrakit_internal_wire::Capability> {
    client::ssh_agent_capabilities()
}

/// Check whether the freeze file exists.
async fn is_frozen(freeze_file_path: &std::path::Path) -> bool {
    tokio::fs::try_exists(freeze_file_path)
        .await
        .unwrap_or(false)
}

/// Run the embedded SSH agent event loop.
///
/// This is the main entry point called from `EmbeddedServiceHost::add()`. It
/// mirrors the standalone SSH agent's event loop but uses `EmbeddedTransport`
/// instead of a WebSocket connection.
pub(crate) async fn run_embedded_ssh_agent(
    mut transport: EmbeddedTransport,
    tokens: EmbeddedShutdownTokens,
    state_dir: PathBuf,
) {
    // 1. Create state subdir.
    let ssh_state_dir = state_dir.join("embedded-ssh-agent");
    if let Err(e) = tokio::fs::create_dir_all(&ssh_state_dir).await {
        tracing::error!(error = %e, "failed to create embedded SSH agent state directory");
        return;
    }

    // 2. Register column AAD mapping.
    register_ssh_column_aad();

    // 3. Open and migrate local SQLite database.
    let db = match uptrakit_agent_ssh::db::init_db(&ssh_state_dir).await {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(error = %e, "failed to initialize embedded SSH agent database");
            return;
        }
    };

    // 4. Initialize the data key ring from the local DB.
    init_ssh_data_key_ring(&db).await;

    // 5. Re-encrypt any non-v3 encrypted values.
    reencrypt_ssh_to_v3(&db).await;

    // 6. Create SSH connection pool.
    let pool = ssh_pool::SshConnectionPool::new();

    // 7. Generate ephemeral ECIES P-256 key pair for extension param decryption.
    let (private_key_der, encryption_public_key) = match generate_ecies_keypair() {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(error = %e, "failed to generate ECIES key pair");
            return;
        }
    };

    // 8. Create infrastructure plugins and extension proxy.
    let extension_proxy = Arc::new(ServiceExtensionProxy::new());
    let infra_plugins: Arc<Vec<Arc<dyn PluginBase>>> =
        Arc::new(uptrakit_plugin_infrastructure_registry::create_agent_infra_plugins());

    // 9. Create aggregate and background channels.
    let (aggregate_tx, mut aggregate_rx) = tokio::sync::mpsc::channel::<(String, UpdateEvent)>(64);
    let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel::<ServiceMessage>(64);

    // Resolve freeze file path.
    let freeze_file_path = ssh_state_dir.join("update-freeze");

    // --- Registration ---

    // Send Register with SSH agent capabilities.
    let caps = ssh_agent_capabilities();
    if let Err(e) = transport
        .transport_send(ServiceMessage::Register(RegisterPayload::new(caps.clone())))
        .await
    {
        tracing::error!(error = %e, "embedded SSH agent: failed to send Register");
        return;
    }

    // Send initial ReportHosts.
    client::report_enrolled_hosts(&db, &mut transport, &pool).await;

    // Register UI extensions.
    if caps.contains(&uptrakit_internal_wire::Capability::UiExtensions) {
        let register_payload =
            extension::build_register_payload(Some(encryption_public_key.clone()), &infra_plugins);
        if let Err(e) = transport
            .transport_send(ServiceMessage::ExtensionRegister(register_payload))
            .await
        {
            tracing::warn!(error = %e, "failed to register UI extensions");
        }

        let actions_payload = ExtensionActionsPayload::new(extension::build_actions());
        if let Err(e) = transport
            .transport_send(ServiceMessage::ExtensionActionsRegister(actions_payload))
            .await
        {
            tracing::warn!(error = %e, "failed to register extension actions");
        }
    }

    // Capture initial host snapshot and start reload ticker.
    let mut host_snapshot = match host_ops::list_host_snapshots(&db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to initialize host snapshot");
            Vec::new()
        }
    };

    let start = tokio::time::Instant::now() + uptrakit_agent_ssh::HOST_RELOAD_INTERVAL;
    let mut reload_ticker =
        tokio::time::interval_at(start, uptrakit_agent_ssh::HOST_RELOAD_INTERVAL);
    reload_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Spawn post-report hooks (Proxmox pending match drain, etc.).
    spawn_post_report_hooks(
        &db,
        &extension_proxy,
        &bg_tx,
        &infra_plugins,
        &ssh_state_dir,
        None,
        private_key_der.as_deref(),
    );

    tracing::info!("embedded SSH agent started");

    // --- Event loop state ---
    let mut in_flight_updates: HashMap<String, SshInFlightUpdate> = HashMap::new();
    let mut last_update_per_host: HashMap<String, std::time::Instant> = HashMap::new();

    loop {
        tokio::select! {
            biased;

            // Drain (graceful shutdown phase 1).
            () = tokens.drain.cancelled() => {
                tracing::info!("embedded SSH agent: draining");
                drain_and_shutdown(
                    &mut transport,
                    &mut in_flight_updates,
                    &mut aggregate_rx,
                    &mut bg_rx,
                    &pool,
                )
                .await;
                break;
            }

            // Abort (hard stop).
            () = tokens.abort.cancelled() => {
                tracing::info!("embedded SSH agent: aborting");
                break;
            }

            // In-flight update events (per-host).
            Some((host_machine_id, event)) = poll_updates(&mut aggregate_rx, &in_flight_updates) => {
                handle_update_event(
                    &mut transport,
                    &mut in_flight_updates,
                    &host_machine_id,
                    event,
                )
                .await;
            }

            // Background results (discovery, version checks, batch updates).
            Some(msg) = bg_rx.recv() => {
                if let Err(e) = transport.transport_send_auto_paginate(msg).await {
                    tracing::error!(error = %e, "embedded SSH agent: failed to send background result");
                }
            }

            // Host config reload ticker.
            _ = reload_ticker.tick() => {
                if !transport.is_yielded() {
                    handle_host_config_changed(
                        &db,
                        &mut transport,
                        &mut host_snapshot,
                        &pool,
                        &extension_proxy,
                        &bg_tx,
                        &infra_plugins,
                        &ssh_state_dir,
                        private_key_der.as_deref(),
                    )
                    .await;
                }
            }

            // Controller messages.
            msg = transport.transport_recv() => {
                let Some(msg) = msg else {
                    tracing::info!("embedded SSH agent: transport closed");
                    break;
                };

                // Skip processing when yielded to an external SSH agent.
                if transport.is_yielded() {
                    tracing::debug!("embedded SSH agent: yielded, ignoring controller message");
                    continue;
                }

                handle_controller_message(
                    msg,
                    &db,
                    &mut transport,
                    &mut in_flight_updates,
                    &mut last_update_per_host,
                    &aggregate_tx,
                    &pool,
                    &freeze_file_path,
                    &extension_proxy,
                    &infra_plugins,
                    &ssh_state_dir,
                    private_key_der.as_deref(),
                    &bg_tx,
                    &mut host_snapshot,
                )
                .await;
            }
        }
    }

    // Drain any remaining background results (best-effort).
    while let Ok(msg) = bg_rx.try_recv() {
        transport.transport_send_best_effort(msg).await;
    }

    tracing::info!("embedded SSH agent stopped");
}

/// Generate an ephemeral ECIES P-256 key pair for extension parameter decryption.
///
/// Returns `(private_key_der, base64_public_key)`.
fn generate_ecies_keypair() -> Result<(Option<Vec<u8>>, String), String> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| format!("P-256 key generation failed: {e}"))?;
    let private_der = key_pair.serialize_der();
    let public_raw = key_pair.public_key_raw().to_vec();
    let public_b64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&public_raw)
    };
    Ok((Some(private_der), public_b64))
}

/// Poll the in-flight update map for events. Returns `pending()` when no
/// updates are active.
async fn poll_updates(
    aggregate_rx: &mut tokio::sync::mpsc::Receiver<(String, UpdateEvent)>,
    in_flight_updates: &HashMap<String, SshInFlightUpdate>,
) -> Option<(String, UpdateEvent)> {
    if in_flight_updates.is_empty() {
        std::future::pending().await
    } else {
        aggregate_rx.recv().await
    }
}

/// Handle an update event from an in-flight update task.
async fn handle_update_event(
    transport: &mut EmbeddedTransport,
    in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
    host_machine_id: &str,
    event: UpdateEvent,
) {
    let Some(update) = in_flight_updates.get(host_machine_id) else {
        tracing::error!(
            %host_machine_id,
            "received update event but no in-flight update found"
        );
        return;
    };
    let update_history_id = update.update_history_id;

    match event {
        UpdateEvent::Output(output_msg) => {
            client::send_update_output(transport, update_history_id, output_msg).await;
        }
        UpdateEvent::Completed(result) => {
            if let Err(e) = client::send_update_result(transport, update_history_id, result).await {
                tracing::error!(error = %e, "failed to send UpdateResult");
            }
            in_flight_updates.remove(host_machine_id);
        }
        UpdateEvent::Attention(uid) => {
            transport
                .transport_send_best_effort(ServiceMessage::StdinAttention(
                    uptrakit_internal_wire::StdinAttentionPayload::new(uid),
                ))
                .await;
        }
    }
}

/// Dispatch a controller message to the appropriate handler.
#[allow(clippy::too_many_arguments)]
async fn handle_controller_message(
    msg: ControllerMessage,
    db: &sea_orm::DatabaseConnection,
    transport: &mut EmbeddedTransport,
    in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
    last_update_per_host: &mut HashMap<String, std::time::Instant>,
    aggregate_tx: &tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
    pool: &ssh_pool::SshConnectionPool,
    freeze_file_path: &std::path::Path,
    extension_proxy: &Arc<ServiceExtensionProxy>,
    infra_plugins: &Arc<Vec<Arc<dyn PluginBase>>>,
    state_dir: &std::path::Path,
    private_key_der: Option<&[u8]>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    host_snapshot: &mut Vec<host_ops::HostSnapshot>,
) {
    match msg {
        ControllerMessage::CheckVersions(payload) => {
            client::spawn_check_versions_ssh(payload, db, pool, bg_tx);
        }

        ControllerMessage::ExecuteUpdate(payload) => {
            if !is_update_allowed(
                &payload.host_machine_id,
                freeze_file_path,
                last_update_per_host,
            )
            .await
            {
                return;
            }
            last_update_per_host.insert(payload.host_machine_id.clone(), std::time::Instant::now());
            client::handle_execute_update_ssh(
                *payload,
                db,
                in_flight_updates,
                aggregate_tx,
                transport,
                pool,
            )
            .await;
        }

        ControllerMessage::ExecuteBatchUpdate(payload) => {
            if !is_update_allowed(
                &payload.host_machine_id,
                freeze_file_path,
                last_update_per_host,
            )
            .await
            {
                return;
            }
            last_update_per_host.insert(payload.host_machine_id.clone(), std::time::Instant::now());
            client::spawn_execute_batch_update_ssh(*payload, db, pool, bg_tx);
        }

        ControllerMessage::DiscoverSoftware(payload) => {
            client::spawn_discover_software_ssh(payload, db, pool, bg_tx);
        }

        ControllerMessage::SetUpdateFreeze(payload) => {
            handle_set_update_freeze(freeze_file_path, payload).await;
        }

        #[cfg(feature = "interactive")]
        ControllerMessage::UpdateStdinData(payload) => {
            client::handle_update_stdin_data_ssh(payload, in_flight_updates);
        }

        ControllerMessage::ReportPluginConfigResponse(payload) => {
            handle_report_plugin_config_response(payload, db, infra_plugins).await;
        }

        ControllerMessage::ResetData => {
            handle_reset_data(db, host_snapshot, last_update_per_host).await;
        }

        ControllerMessage::ExtensionRequest(request) => {
            let ctx = ExtensionContext {
                db,
                state_dir,
                private_key_der,
                service_id: None,
                tenant_id: None,
                bg_tx,
                extension_proxy,
                infra_plugins: Arc::clone(infra_plugins),
            };
            extension::handle_extension_request(request, &ctx, transport).await;
        }

        ControllerMessage::ExtensionResponse(response) => {
            let request_id = response.request_id.clone();
            extension_proxy.complete(&request_id, response);
        }

        _ => {
            tracing::trace!("embedded SSH agent: ignoring unhandled controller message");
        }
    }
}

/// Check whether an update is allowed (not frozen, not rate-limited).
async fn is_update_allowed(
    host_machine_id: &str,
    freeze_file_path: &std::path::Path,
    last_update_per_host: &HashMap<String, std::time::Instant>,
) -> bool {
    if is_frozen(freeze_file_path).await {
        tracing::warn!(
            %host_machine_id,
            "security_audit: update rejected — updates are frozen"
        );
        return false;
    }
    if let Some(last) = last_update_per_host.get(host_machine_id)
        && last.elapsed() < uptrakit_agent_ssh::UPDATE_COOLDOWN
    {
        tracing::warn!(
            %host_machine_id,
            "security_audit: update rejected — rate limit"
        );
        return false;
    }
    true
}

/// Handle a `ReportPluginConfigResponse` from the controller.
async fn handle_report_plugin_config_response(
    payload: uptrakit_internal_wire::ReportPluginConfigResponsePayload,
    db: &sea_orm::DatabaseConnection,
    infra_plugins: &Arc<Vec<Arc<dyn PluginBase>>>,
) {
    if payload.success {
        if let Some(config_id_str) = &payload.plugin_config_id {
            let config_id = *config_id_str;
            let request_id = payload.request_id.clone();
            for plugin in infra_plugins.iter() {
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
    } else {
        tracing::warn!(
            request_id = %payload.request_id,
            error = ?payload.error,
            "plugin config report failed"
        );
    }
}

/// Handle a `ResetData` message: truncate the local SSH database.
async fn handle_reset_data(
    db: &sea_orm::DatabaseConnection,
    host_snapshot: &mut Vec<host_ops::HostSnapshot>,
    last_update_per_host: &mut HashMap<String, std::time::Instant>,
) {
    if cfg!(feature = "reset-data") {
        tracing::info!("embedded SSH agent: received ResetData, truncating local data");
        use sea_orm::{ConnectionTrait, EntityTrait, TransactionTrait};
        match db.begin().await {
            Ok(txn) => {
                // Delete in FK-safe order:
                // 1. pending_proxmox_matches (references ssh_hosts)
                if let Err(e) =
                    uptrakit_agent_ssh::db::entity::pending_proxmox_match::Entity::delete_many()
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
                if let Err(e) = uptrakit_agent_ssh::db::entity::ssh_host::Entity::delete_many()
                    .exec(&txn)
                    .await
                {
                    tracing::error!(error = %e, "failed to truncate ssh_hosts");
                }
                match txn.commit().await {
                    Ok(()) => {
                        tracing::info!("local data stores truncated successfully");
                        host_snapshot.clear();
                        last_update_per_host.clear();
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

/// React to a host-config reload tick.
///
/// Queries the current `ssh_hosts` snapshot, diffs it against the stored
/// snapshot, evicts stale pool entries, and sends an updated `ReportHosts`
/// message if anything changed.
#[allow(clippy::too_many_arguments)]
async fn handle_host_config_changed(
    db: &sea_orm::DatabaseConnection,
    transport: &mut EmbeddedTransport,
    host_snapshot: &mut Vec<host_ops::HostSnapshot>,
    pool: &ssh_pool::SshConnectionPool,
    extension_proxy: &Arc<ServiceExtensionProxy>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    infra_plugins: &Arc<Vec<Arc<dyn PluginBase>>>,
    state_dir: &std::path::Path,
    private_key_der: Option<&[u8]>,
) {
    let current_snapshot = match host_ops::list_host_snapshots(db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list host snapshots during reload check");
            return;
        }
    };

    if current_snapshot == *host_snapshot {
        return;
    }

    let (deleted_ids, changed_ids) = diff_host_snapshots(host_snapshot, &current_snapshot);

    // Evict pool entries for deleted and changed hosts.
    for id in &deleted_ids {
        pool.evict(*id).await;
    }
    for id in &changed_ids {
        pool.evict(*id).await;
    }

    *host_snapshot = current_snapshot;

    let hosts = match host_ops::list_hosts(db).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list hosts for dynamic reload");
            return;
        }
    };

    tracing::info!(
        total_hosts = hosts.len(),
        changed = changed_ids.len(),
        deleted = deleted_ids.len(),
        "host configuration changed — sending updated ReportHosts"
    );

    client::report_hosts_after_config_change(db, transport, &hosts, &changed_ids, pool).await;

    spawn_post_report_hooks(
        db,
        extension_proxy,
        bg_tx,
        infra_plugins,
        state_dir,
        None,
        private_key_der,
    );
}

/// Spawn post-report-hooks background task (Proxmox pending match drain, etc.).
fn spawn_post_report_hooks(
    db: &sea_orm::DatabaseConnection,
    extension_proxy: &Arc<ServiceExtensionProxy>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    infra_plugins: &Arc<Vec<Arc<dyn PluginBase>>>,
    state_dir: &std::path::Path,
    private_key_der_override: Option<Option<&[u8]>>,
    default_private_key_der: Option<&[u8]>,
) {
    let proxy = Arc::clone(extension_proxy);
    let bg_tx = bg_tx.clone();
    let infra_plugins = Arc::clone(infra_plugins);
    let state_dir = state_dir.to_path_buf();
    let private_key_der = private_key_der_override
        .unwrap_or(Some(default_private_key_der.unwrap_or(&[])))
        .map(|s| s.to_vec());
    let db = db.clone();

    tokio::spawn(async move {
        let action_invoker = InfraActionInvokerImpl::new(&proxy, &bg_tx);
        let ctx = uptrakit_plugin_infrastructure_core::agent_infra::InfraPluginContext {
            db: &db,
            tenant_id: None,
            service_id: None,
            state_dir: &state_dir,
            private_key_der: private_key_der.as_deref(),
            action_invoker: &action_invoker,
            guest_bootstrap:
                &uptrakit_agent_ssh::operations::bootstrap_proxmox::NoopGuestBootstrapExecutor,
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

/// Graceful shutdown: drain in-flight updates, send Disconnecting, close pool.
async fn drain_and_shutdown(
    transport: &mut EmbeddedTransport,
    in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
    aggregate_rx: &mut tokio::sync::mpsc::Receiver<(String, UpdateEvent)>,
    bg_rx: &mut tokio::sync::mpsc::Receiver<ServiceMessage>,
    pool: &ssh_pool::SshConnectionPool,
) {
    if !in_flight_updates.is_empty() {
        let count = in_flight_updates.len();
        tracing::info!(
            count,
            timeout = ?SHUTDOWN_TIMEOUT,
            "waiting for in-flight SSH updates to complete before shutdown"
        );

        let deadline = tokio::time::Instant::now() + SHUTDOWN_TIMEOUT;

        while !in_flight_updates.is_empty() {
            tokio::select! {
                biased;
                Some((host_id, event)) = aggregate_rx.recv() => {
                    if let Some(update) = in_flight_updates.get(&host_id) {
                        let uid = update.update_history_id;
                        match event {
                            UpdateEvent::Output(msg) => {
                                client::send_update_output(transport, uid, msg).await;
                            }
                            UpdateEvent::Completed(result) => {
                                if let Err(e) = client::send_update_result(transport, uid, result).await {
                                    tracing::warn!(error = %e, "failed to send UpdateResult during shutdown");
                                }
                                in_flight_updates.remove(&host_id);
                            }
                            UpdateEvent::Attention(_) => {
                                // Ignore attention during shutdown.
                            }
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!(
                        remaining = in_flight_updates.len(),
                        "shutdown timeout reached, abandoning remaining in-flight updates"
                    );
                    for (_, update) in in_flight_updates.drain() {
                        transport
                            .transport_send_best_effort(ServiceMessage::UpdateResult(
                                UpdateResultPayload {
                                    update_history_id: update.update_history_id,
                                    status: UpdateFinalStatus::Failed,
                                    from_version: None,
                                    to_version: None,
                                    output: String::new(),
                                    error: Some(format!(
                                        "Agent shutdown timeout ({}s) reached",
                                        SHUTDOWN_TIMEOUT.as_secs()
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

        // Drain any remaining buffered events.
        while let Ok((host_id, event)) = aggregate_rx.try_recv() {
            if let Some(update) = in_flight_updates.get(&host_id)
                && let UpdateEvent::Output(msg) = event
            {
                client::send_update_output(transport, update.update_history_id, msg).await;
            }
        }
    }

    // Send Disconnecting.
    if let Err(e) = transport
        .transport_send(ServiceMessage::Disconnecting(DisconnectingPayload::new(
            DisconnectReason::Shutdown,
        )))
        .await
    {
        tracing::debug!(error = %e, "failed to send Disconnecting message");
    }

    // Drain background results.
    while let Ok(msg) = bg_rx.try_recv() {
        transport.transport_send_best_effort(msg).await;
    }

    // Close all pooled SSH connections.
    pool.disconnect_all().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_agent_capabilities_includes_expected_set() {
        let caps = ssh_agent_capabilities();
        assert!(caps.contains(&uptrakit_internal_wire::Capability::SoftwareDiscovery));
        assert!(caps.contains(&uptrakit_internal_wire::Capability::SshRemote));
        assert!(caps.contains(&uptrakit_internal_wire::Capability::UiExtensions));
        assert!(caps.contains(&uptrakit_internal_wire::Capability::GracefulShutdown));
    }

    #[cfg(feature = "interactive")]
    #[test]
    fn ssh_agent_capabilities_includes_interactive_when_feature_enabled() {
        let caps = ssh_agent_capabilities();
        assert!(caps.contains(&uptrakit_internal_wire::Capability::InteractiveUpdates));
    }

    #[tokio::test]
    async fn freeze_file_create_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let freeze_path = dir.path().join("embedded-ssh-agent").join("update-freeze");

        assert!(!is_frozen(&freeze_path).await);

        // Create parent directory first (mirrors real init).
        tokio::fs::create_dir_all(freeze_path.parent().unwrap())
            .await
            .unwrap();

        handle_set_update_freeze(
            &freeze_path,
            uptrakit_internal_wire::SetUpdateFreezePayload {
                enabled: true,
                reason: Some("test freeze".to_string()),
            },
        )
        .await;
        assert!(is_frozen(&freeze_path).await);

        handle_set_update_freeze(
            &freeze_path,
            uptrakit_internal_wire::SetUpdateFreezePayload {
                enabled: false,
                reason: None,
            },
        )
        .await;
        assert!(!is_frozen(&freeze_path).await);
    }

    #[test]
    fn generate_ecies_keypair_produces_valid_pair() {
        let (private_key, public_key) = generate_ecies_keypair().expect("keygen");
        assert!(private_key.is_some());
        let private_key = private_key.unwrap();
        assert!(!private_key.is_empty());
        assert!(!public_key.is_empty());

        // Public key should be valid base64.
        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&public_key)
            .expect("valid base64");
        // Uncompressed P-256 point: 0x04 || x (32 bytes) || y (32 bytes) = 65 bytes.
        assert_eq!(
            decoded.len(),
            65,
            "P-256 uncompressed public key should be 65 bytes"
        );
        assert_eq!(decoded[0], 0x04, "uncompressed point marker");
    }
}
