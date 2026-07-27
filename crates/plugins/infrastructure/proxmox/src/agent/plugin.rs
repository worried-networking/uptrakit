#![expect(
    clippy::expect_used,
    reason = "infallible literal surface ID and value constructions; panic would indicate a programming error in the surface manifest"
)]
//! Proxmox VE agent infrastructure plugin — subtrait implementations.
//!
//! Implements infrastructure subtraits (`HostLifecycle`,
//! `HostReport`, `GuestExec`) on [`ProxmoxPlugin`](crate::ProxmoxPlugin)
//! for PVE-specific agent logic: bootstrap detection, credential creation,
//! host sync, surface actions, and deferred post-ReportHosts matching.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use serde_json::json;
use uptrakit_command::RemoteExecutor;
use uptrakit_plugin_infrastructure_core::agent_infra::{
    BootstrapInfraResult, GuestExecProvider, InfraActionInvoker, InfraPluginContext,
    InfraResolvedSudo, PluginConfigReport, SyncInfraResult,
};
use uptrakit_plugin_infrastructure_core::error::{PluginError, Result};
use uptrakit_plugin_infrastructure_core::{
    AgentInteraction, AgentInteractionPlacement, FormFieldDescriptor, FormFieldType,
    FormSelectSourceDescriptor, SurfaceActionUi, SurfaceFormDescriptor,
    surfaces::{SurfaceActionRequest, SurfaceActionResponse},
};

use crate::pve_setup;

use super::db_ops;

// ── Agent surface actions ───────────────────────────────────────────────────

/// Returns the agent interaction declarations contributed by the agent side
/// of the Proxmox plugin (table-dispatched via `AgentInteractionHandler`).
pub(crate) fn agent_interactions() -> Vec<AgentInteraction> {
    vec![
        AgentInteraction::new("discovered-guests", "List Discovered Guests")
            .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
            .with_timeout(15)
            .with_agent_handler(super::surface_actions::discovered_guests_dispatch),
        bootstrap_proxmox_guest_interaction(),
    ]
}

// ── Subtrait implementations on ProxmoxPlugin ────────────────────────────────

use uptrakit_plugin_infrastructure_core::{GuestExec, HostLifecycle, HostReport};

#[async_trait]
impl HostLifecycle for crate::ProxmoxPlugin {
    fn sync_step_previews(&self) -> Vec<String> {
        vec![
            "Detect PVE node name".to_string(),
            "Reconcile plugin config ID across cluster peers".to_string(),
            format!(
                "Ensure {} on /, {} on /vms and /storage",
                pve_setup::UPTRAKIT_AUDIT_ROLE,
                pve_setup::UPTRAKIT_PROTECTION_ROLE,
            ),
        ]
    }

    fn sync_security_impact(&self) -> uptrakit_shared_types::Severity {
        // Creates PVE users and grants ACLs on the cluster.
        uptrakit_shared_types::Severity::High
    }

    async fn on_host_bootstrapped(
        &self,
        ctx: &InfraPluginContext<'_>,
        executor: &dyn RemoteExecutor,
        host_id: uuid::Uuid,
        _host_name: &str,
    ) -> Result<BootstrapInfraResult> {
        // Detect PVE node.
        let is_pve_node = match pve_setup::detect_pve_node(executor).await {
            Ok(detected) => {
                if detected {
                    tracing::info!("detected Proxmox VE node");
                }
                detected
            }
            Err(e) => {
                tracing::debug!(error = %e, "PVE detection failed, treating as non-PVE host");
                false
            }
        };

        if !is_pve_node {
            return Ok(BootstrapInfraResult::default());
        }

        // Detect PVE node name.
        let pve_node_name = match pve_setup::detect_pve_node_name(executor).await {
            Ok(name) => {
                tracing::info!(node_name = %name, "detected PVE node name");
                Some(name)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to detect PVE node name during bootstrap");
                None
            }
        };

        // Create or reuse PVE credentials.
        let (pve_credentials, existing_config_id) =
            create_or_reuse_pve_credentials(executor, ctx.db, ctx.tenant_id).await;

        // Persist PVE state.
        let host_id_str = host_id.to_string();
        let config_id = existing_config_id.clone();
        if let Err(e) =
            db_ops::upsert_host_state(ctx.db, &host_id_str, true, config_id, pve_node_name).await
        {
            tracing::warn!(error = %e, "failed to persist PVE state for host");
        }

        // Build the report config if new credentials were created.
        let report = pve_credentials.map(|creds| PluginConfigReport {
            plugin_type: "infrastructure.proxmox".to_string(),
            name: format!("pve-{host_id}"),
            config: json!({
                "api_url": creds.api_url,
                "api_token": creds.api_token,
                "verify_ssl": true,
            }),
        });

        let sudo_commands = collect_pve_sudo_commands(executor).await;

        Ok(BootstrapInfraResult {
            report_plugin_config: report,
            existing_plugin_config_id: existing_config_id,
            detected: true,
            sudo_commands,
        })
    }

    async fn on_host_synced(
        &self,
        ctx: &InfraPluginContext<'_>,
        executor: &dyn RemoteExecutor,
        host_id: uuid::Uuid,
    ) -> Result<SyncInfraResult> {
        let host_id_str = host_id.to_string();
        let state = db_ops::find_host_state(ctx.db, &host_id_str)
            .await
            .context_to::<PluginError>()?;
        let state = match state {
            Some(s) if s.is_pve_node => s,
            _ => return Ok(SyncInfraResult::default()),
        };

        let mut lines = Vec::new();

        // Step 1: node name.
        let node_name = match pve_setup::detect_pve_node_name(executor).await {
            Ok(name) => {
                lines.push(format!("node name: {name}"));
                Some(name)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to detect PVE node name");
                lines.push(format!("node name: detection failed ({e})"));
                None
            }
        };

        // Step 2: plugin config ID reconciliation.
        let mut report_plugin_config: Option<PluginConfigReport> = None;
        // Track whether the PVE token on this cluster belongs to our tenant.
        // Step 3 uses this to gate privilege management — we must not touch
        // ACLs/roles on clusters where the token was not created by Uptrakit.
        let mut token_owned_by_tenant: Option<uuid::Uuid> = None;
        let canonical_config_id: Option<String> = if let Some(tid) = ctx.tenant_id {
            let tid_uuid = uuid::Uuid::parse_str(tid).ok();
            if let Some(tid_uuid) = tid_uuid {
                match pve_setup::check_pve_token_exists(executor, &tid_uuid).await {
                    Ok(pve_setup::PveTokenStatus::OwnedByTenant(_)) => {
                        token_owned_by_tenant = Some(tid_uuid);
                        let cluster_nodes = pve_setup::detect_pve_cluster_nodes(executor).await;
                        let peer_config_id = if cluster_nodes.is_empty() {
                            None
                        } else {
                            reconcile_pve_config(ctx.db, &host_id_str, &cluster_nodes).await
                        };
                        if peer_config_id.is_some() {
                            // Use the config from a cluster peer.
                            peer_config_id
                        } else {
                            // No cluster peer has a config (or this is a standalone
                            // node). Regenerate the token so the controller always
                            // has a fresh, valid plugin config entry.  The local
                            // `pve_plugin_config_id` may be stale (controller config
                            // deleted) so we do NOT gate on it.
                            tracing::info!(
                                "PVE token owned by tenant but no cluster peer config found; \
                                 regenerating token to create/refresh config"
                            );
                            match pve_setup::regenerate_pve_api_token(executor, &tid_uuid).await {
                                Ok(creds) => {
                                    lines.push(
                                        "token: no config found on cluster, regenerating"
                                            .to_string(),
                                    );
                                    report_plugin_config = Some(PluginConfigReport {
                                        plugin_type: "infrastructure.proxmox".to_string(),
                                        name: format!("pve-{host_id}"),
                                        config: serde_json::json!({
                                            "api_url": creds.api_url,
                                            "api_token": creds.api_token,
                                            "verify_ssl": true,
                                        }),
                                    });
                                    None
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to regenerate PVE API token during sync"
                                    );
                                    lines.push(format!("token: regeneration failed ({e})"));
                                    None
                                }
                            }
                        }
                    }
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };

        let config_id_to_store = canonical_config_id
            .as_ref()
            .or(state.pve_plugin_config_id.as_ref())
            .cloned();

        if let Some(ref new_id) = canonical_config_id {
            if state.pve_plugin_config_id.as_deref() != Some(new_id.as_str()) {
                lines.push(format!(
                    "plugin config: corrected from {} to {new_id}",
                    state.pve_plugin_config_id.as_deref().unwrap_or("(none)")
                ));
            } else {
                lines.push(format!("plugin config: OK ({new_id})"));
            }
        }

        // Persist.
        if node_name.is_some() {
            db_ops::upsert_host_state(ctx.db, &host_id_str, true, config_id_to_store, node_name)
                .await
                .context_to::<PluginError>()?;
        }

        // Step 3: ensure privilege roles and ACLs — only for Uptrakit-owned tokens.
        if let Some(tid_uuid) = token_owned_by_tenant {
            match pve_setup::ensure_pve_privileges(executor, &tid_uuid).await {
                Ok(()) => lines.push(format!(
                    "privileges: OK ({} on /, {} and {} on /vms, {} on /storage)",
                    pve_setup::UPTRAKIT_AUDIT_ROLE,
                    pve_setup::UPTRAKIT_PROTECTION_ROLE,
                    pve_setup::UPTRAKIT_SCALING_ROLE,
                    pve_setup::UPTRAKIT_PROTECTION_ROLE,
                )),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to ensure PVE privileges during sync");
                    lines.push(format!("privileges: FAILED — {e}"));
                }
            }
        } else if ctx.tenant_id.is_none() {
            lines.push(
                "privilege check: skipped (tenant ID not yet available — \
                 ensure the agent has connected to the controller at least once)"
                    .to_string(),
            );
        }

        // Collect sudo commands for PVE tools.
        let sudo_commands = collect_pve_sudo_commands(executor).await;

        Ok(SyncInfraResult {
            summary_lines: lines,
            sudo_commands,
            report_plugin_config,
        })
    }

    async fn on_post_report_hosts(&self, ctx: &InfraPluginContext<'_>) -> Result<()> {
        drain_pending_matches_cycle(ctx.db, ctx.action_invoker).await
    }

    async fn on_plugin_config_reported(
        &self,
        db: &DatabaseConnection,
        plugin_config_id: uuid::Uuid,
        _request_id: &str,
    ) -> Result<()> {
        // Find the first PVE host without a config ID and update it.
        let pve_hosts = db_ops::find_pve_hosts(db)
            .await
            .context_to::<PluginError>()?;
        for host in pve_hosts {
            if host.pve_plugin_config_id.is_none() {
                db_ops::upsert_host_state(
                    db,
                    &host.host_id,
                    true,
                    Some(plugin_config_id.to_string()),
                    host.pve_node_name.clone(),
                )
                .await
                .context_to::<PluginError>()?;
                break;
            }
        }
        Ok(())
    }
}

// ── Pending-match drain (poison-row protection) ─────────────────────────────

/// Drains at most one cycle's worth of pending Proxmox host matches.
///
/// Caps the number of rows processed per call at `MAX_DRAIN_PER_CYCLE` so a
/// burst of pending rows cannot block a single `ReportHosts` cycle
/// indefinitely, and gives up on (deletes) any row that has failed
/// `MAX_MATCH_ATTEMPTS` times so a permanently-broken mapping cannot retry
/// forever.
async fn drain_pending_matches_cycle(
    db: &DatabaseConnection,
    action_invoker: &dyn InfraActionInvoker,
) -> Result<()> {
    const MAX_DRAIN_PER_CYCLE: usize = 50;
    const MAX_MATCH_ATTEMPTS: i32 = 10;

    let pending = db_ops::drain_pending_matches(db)
        .await
        .context_to::<PluginError>()?;

    if pending.is_empty() {
        return Ok(());
    }

    tracing::info!(
        count = pending.len(),
        "draining pending Proxmox host matches"
    );

    for entry in pending.into_iter().take(MAX_DRAIN_PER_CYCLE) {
        let match_params = json!({
            "mapping_id": entry.mapping_id,
            "host_id": entry.host_id,
        });

        match action_invoker
            .invoke("proxmox.hosts", "match", match_params)
            .await
        {
            Ok(resp) if resp.success => {
                tracing::info!(
                    host_id = %entry.host_id,
                    mapping_id = %entry.mapping_id,
                    "auto-matched bootstrapped guest to Proxmox host mapping"
                );
                if let Err(e) = db_ops::delete_pending_match(db, entry.id).await {
                    tracing::warn!(
                        id = entry.id,
                        error = %e,
                        "matched but failed to delete pending Proxmox match row"
                    );
                }
            }
            outcome => {
                match &outcome {
                    Ok(resp) => tracing::warn!(
                        host_id = %entry.host_id,
                        mapping_id = %entry.mapping_id,
                        error = ?resp.error,
                        "pending Proxmox match failed; will retry on next ReportHosts"
                    ),
                    Err(e) => tracing::warn!(
                        host_id = %entry.host_id,
                        mapping_id = %entry.mapping_id,
                        error = %e,
                        "pending Proxmox match proxy call failed; will retry on next ReportHosts"
                    ),
                }
                if entry.attempts + 1 >= MAX_MATCH_ATTEMPTS {
                    tracing::warn!(
                        host_id = %entry.host_id,
                        mapping_id = %entry.mapping_id,
                        attempts = entry.attempts + 1,
                        "giving up on pending Proxmox match; re-run matching manually from the Proxmox VE Hosts page"
                    );
                    if let Err(e) = db_ops::delete_pending_match(db, entry.id).await {
                        tracing::warn!(id = entry.id, error = %e, "failed to delete dead pending match row");
                    }
                } else if let Err(e) = db_ops::increment_match_attempts(db, entry.id).await {
                    tracing::warn!(id = entry.id, error = %e, "failed to record pending match attempt");
                }
            }
        }
    }

    Ok(())
}

#[async_trait]
impl HostReport for crate::ProxmoxPlugin {
    async fn has_infra_state(&self, db: &DatabaseConnection, host_id: uuid::Uuid) -> bool {
        db_ops::find_host_state(db, &host_id.to_string())
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.is_pve_node)
    }
}

#[async_trait]
impl GuestExec for crate::ProxmoxPlugin {
    fn guest_exec_provider(&self) -> Option<Arc<dyn GuestExecProvider>> {
        Some(Arc::new(
            super::guest_exec_adapter::ProxmoxGuestExecProvider,
        ))
    }

    async fn handle_service_extension_action(
        &self,
        ctx: &uptrakit_plugin_infrastructure_core::agent_infra::InfraPluginContext<'_>,
        request: &SurfaceActionRequest,
    ) -> Option<SurfaceActionResponse> {
        super::surface_actions::handle_surface_action(ctx, request).await
    }
}

// ── PVE credential dedup ─────────────────────────────────────────────────────

/// Check for existing Uptrakit PVE tokens on the cluster and either reuse
/// the existing config or create new credentials.
async fn create_or_reuse_pve_credentials(
    executor: &dyn RemoteExecutor,
    db: &DatabaseConnection,
    tenant_id: Option<&str>,
) -> (Option<pve_setup::PveCredentials>, Option<String>) {
    let Some(tid_str) = tenant_id else {
        tracing::warn!(
            "skipping PVE API credential creation: tenant ID not yet available; \
             ensure the agent has connected to the controller at least once, then re-bootstrap"
        );
        return (None, None);
    };

    let Ok(tid) = uuid::Uuid::parse_str(tid_str) else {
        tracing::warn!("invalid tenant_id format: {tid_str}");
        return (None, None);
    };

    match pve_setup::check_pve_token_exists(executor, &tid).await {
        Ok(pve_setup::PveTokenStatus::OwnedByTenant(user)) => {
            tracing::info!(pve_user = %user, "PVE API user already exists on this cluster, skipping credential creation");

            match db_ops::find_pve_host_with_config(db).await {
                Ok(Some(host)) => {
                    let config_id = host
                        .pve_plugin_config_id
                        .expect("find_pve_host_with_config only returns hosts with config");
                    tracing::info!(host_id = %host.host_id, "reusing plugin config from existing PVE host");
                    return (None, Some(config_id));
                }
                Ok(None) => {
                    tracing::info!(
                        "PVE user exists for this tenant but no local plugin config found; \
                         regenerating API token"
                    );
                    match pve_setup::regenerate_pve_api_token(executor, &tid).await {
                        Ok(creds) => {
                            tracing::info!(
                                api_url = %creds.api_url,
                                pve_user = %pve_setup::pve_user_realm(&tid),
                                "PVE API token regenerated"
                            );
                            return (Some(creds), None);
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "failed to regenerate PVE API token; configure the Proxmox plugin manually"
                            );
                            return (None, None);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to look up existing PVE hosts");
                    return (None, None);
                }
            }
        }
        Ok(pve_setup::PveTokenStatus::OwnedByOtherTenant(user)) => {
            tracing::warn!(pve_user = %user, "PVE API user belongs to a different tenant; skipping credential creation");
            return (None, None);
        }
        Ok(pve_setup::PveTokenStatus::NotFound) => {
            // No existing token — proceed with creation below.
        }
        Err(e) => {
            tracing::debug!(error = %e, "PVE token check failed, proceeding with creation");
        }
    }

    tracing::info!("creating PVE API credentials");
    match pve_setup::create_pve_api_credentials(executor, &tid).await {
        Ok(creds) => {
            tracing::info!(
                api_url = %creds.api_url,
                pve_user = %pve_setup::pve_user_realm(&tid),
                "PVE API token created"
            );
            (Some(creds), None)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to create PVE API credentials; configure them manually in the Proxmox plugin settings"
            );
            (None, None)
        }
    }
}

// ── PVE sudo commands ────────────────────────────────────────────────────────

/// Collect sudoers entries for PVE-specific management tools.
async fn collect_pve_sudo_commands(executor: &dyn RemoteExecutor) -> Vec<InfraResolvedSudo> {
    let mut cmds = Vec::new();

    let pct_exists = executor
        .exec_command("test -f /usr/sbin/pct")
        .await
        .map(|r| r.exit_code == 0)
        .unwrap_or(false);
    if pct_exists {
        cmds.push(InfraResolvedSudo {
            command_path: "/usr/sbin/pct exec *".to_string(),
            explanation: "Execute commands inside LXC containers for guest bootstrap".to_string(),
            needs_setenv: false,
        });
    }

    let qm_exists = executor
        .exec_command("test -f /usr/sbin/qm")
        .await
        .map(|r| r.exit_code == 0)
        .unwrap_or(false);
    if qm_exists {
        cmds.push(InfraResolvedSudo {
            command_path: "/usr/sbin/qm guest exec *".to_string(),
            explanation: "Execute commands inside QEMU VMs for guest bootstrap".to_string(),
            needs_setenv: false,
        });
        cmds.push(InfraResolvedSudo {
            command_path: "/usr/sbin/qm guest cmd * network-get-interfaces".to_string(),
            explanation: "Query QEMU VM network interfaces for guest IP detection".to_string(),
            needs_setenv: false,
        });
    }

    cmds
}

// ── Config reconciliation ────────────────────────────────────────────────────

/// Determine the canonical `pve_plugin_config_id` for the cluster.
async fn reconcile_pve_config(
    db: &DatabaseConnection,
    current_host_id: &str,
    cluster_nodes: &[String],
) -> Option<String> {
    let all_pve_hosts = match db_ops::find_pve_hosts(db).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list local PVE hosts for config reconciliation");
            return None;
        }
    };

    let mut peer_configs: Vec<String> = all_pve_hosts
        .iter()
        .filter(|h| h.host_id != current_host_id)
        .filter(|h| {
            h.pve_node_name
                .as_deref()
                .is_some_and(|n| cluster_nodes.contains(&n.to_string()))
        })
        .filter_map(|h| h.pve_plugin_config_id.clone())
        .collect();

    peer_configs.sort_unstable();
    peer_configs.dedup();

    match peer_configs.len() {
        0 => None,
        1 => Some(peer_configs.remove(0)),
        _ => {
            let newest = peer_configs
                .into_iter()
                .max()
                .expect("non-empty after multi-branch");
            tracing::warn!(
                canonical_config_id = %newest,
                "cluster peers disagree on pve_plugin_config_id \
                 (likely duplicate configs from a failed bootstrap dedup); \
                 using newest config"
            );
            Some(newest)
        }
    }
}

// ── Action definitions ───────────────────────────────────────────────────────

fn bootstrap_proxmox_guest_interaction() -> AgentInteraction {
    AgentInteraction::new("bootstrap-proxmox-guest", "Bootstrap Discovered Guest")
        .with_icon("boxes")
        .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
        .with_timeout(300)
        .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
            FormFieldDescriptor::new("discovered_guests", "Discovered Guests")
                .with_type(FormFieldType::MultiSelect)
                .required()
                .with_help_text(
                    "Select one or more Proxmox guests to bootstrap. \
                     Names are auto-derived from the guest's hostname.",
                )
                .with_select_source(FormSelectSourceDescriptor::Action {
                    action_id: "discovered-guests".to_string(),
                }),
            FormFieldDescriptor::new("target_username", "Target Username")
                .with_help_text("User to create/use in each guest.")
                .with_default_value("uptrakit"),
            FormFieldDescriptor::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            FormFieldDescriptor::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(FormFieldType::Toggle)
                .with_help_text(
                    "Remove existing Uptrakit-managed keys from authorized_keys before \
                     writing the new entry. Same-service keys are always removed regardless.",
                ),
        ])))
        .placement(AgentInteractionPlacement::Primary)
        .with_agent_handler(super::surface_actions::bootstrap_proxmox_guest_dispatch)
}

#[cfg(test)]
mod tests {
    use super::bootstrap_proxmox_guest_interaction;
    use super::drain_pending_matches_cycle;
    use crate::agent::db_ops;
    use uptrakit_plugin_infrastructure_core::surfaces::SurfaceActionResponse;
    use uptrakit_plugin_infrastructure_core::testing::RecordingActionInvoker;

    #[test]
    fn bootstrap_proxmox_guest_action_has_boxes_icon() {
        let action = bootstrap_proxmox_guest_interaction();
        assert_eq!(action.action_id, "bootstrap-proxmox-guest");
        assert_eq!(action.icon.as_deref(), Some("boxes"));
    }

    #[test]
    fn agent_interactions_registers_both_entries_with_handlers() {
        let interactions = super::agent_interactions();
        assert_eq!(interactions.len(), 2);

        let discovered_guests = interactions
            .iter()
            .find(|i| i.action_id == "discovered-guests")
            .expect("discovered-guests interaction registered");
        assert!(discovered_guests.agent_handler.is_some());

        let bootstrap = interactions
            .iter()
            .find(|i| i.action_id == "bootstrap-proxmox-guest")
            .expect("bootstrap-proxmox-guest interaction registered");
        assert!(bootstrap.agent_handler.is_some());
        assert!(matches!(
            bootstrap.placement,
            super::AgentInteractionPlacement::Primary
        ));
    }

    async fn setup_agent_db() -> sea_orm::DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        let manager = sea_orm_migration::SchemaManager::new(&db);
        for migration in crate::agent::migration::agent_migrations() {
            migration.up(&manager).await.expect("agent migration");
        }
        db
    }

    fn success_response() -> SurfaceActionResponse {
        SurfaceActionResponse {
            request_id: uuid::Uuid::nil(),
            success: true,
            result: None,
            error: None,
        }
    }

    fn failure_response() -> SurfaceActionResponse {
        SurfaceActionResponse {
            request_id: uuid::Uuid::nil(),
            success: false,
            result: None,
            error: None,
        }
    }

    #[tokio::test]
    async fn drain_succeeds_deletes_row() {
        let db = setup_agent_db().await;
        db_ops::insert_pending_match(&db, "host-1", "mapping-1")
            .await
            .expect("insert pending match");

        let invoker = RecordingActionInvoker::new();
        invoker.push_response(Ok(success_response()));

        drain_pending_matches_cycle(&db, &invoker)
            .await
            .expect("drain cycle");

        let remaining = db_ops::drain_pending_matches(&db).await.expect("drain");
        assert!(remaining.is_empty());

        let calls = invoker.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "proxmox.hosts");
        assert_eq!(calls[0].1, "match");
    }

    #[tokio::test]
    async fn drain_failure_increments_attempts() {
        let db = setup_agent_db().await;
        db_ops::insert_pending_match(&db, "host-1", "mapping-1")
            .await
            .expect("insert pending match");

        let invoker = RecordingActionInvoker::new();
        invoker.push_response(Ok(failure_response()));

        drain_pending_matches_cycle(&db, &invoker)
            .await
            .expect("drain cycle");

        let remaining = db_ops::drain_pending_matches(&db).await.expect("drain");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].attempts, 1);
    }

    #[tokio::test]
    async fn drain_dead_letters_at_max_attempts() {
        let db = setup_agent_db().await;
        db_ops::insert_pending_match(&db, "host-1", "mapping-1")
            .await
            .expect("insert pending match");
        let pending = db_ops::drain_pending_matches(&db).await.expect("drain");
        let id = pending[0].id;

        for _ in 0..9 {
            db_ops::increment_match_attempts(&db, id)
                .await
                .expect("increment attempts");
        }

        let invoker = RecordingActionInvoker::new();
        invoker.push_response(Ok(failure_response()));

        drain_pending_matches_cycle(&db, &invoker)
            .await
            .expect("drain cycle");

        let remaining = db_ops::drain_pending_matches(&db).await.expect("drain");
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn drain_respects_per_cycle_cap() {
        let db = setup_agent_db().await;
        for i in 0..51 {
            db_ops::insert_pending_match(&db, "host-1", &format!("mapping-{i}"))
                .await
                .expect("insert pending match");
        }

        let invoker = RecordingActionInvoker::new();
        // Queue nothing — RecordingActionInvoker yields a default success
        // response for every call when the response queue is empty.

        drain_pending_matches_cycle(&db, &invoker)
            .await
            .expect("drain cycle");

        assert_eq!(invoker.calls().len(), 50);
        let remaining = db_ops::drain_pending_matches(&db).await.expect("drain");
        assert_eq!(remaining.len(), 1);
    }
}
