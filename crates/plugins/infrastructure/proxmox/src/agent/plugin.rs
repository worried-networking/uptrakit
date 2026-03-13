//! [`AgentInfraPlugin`] implementation for Proxmox VE.
//!
//! Encapsulates all PVE-specific logic that was previously inlined in the SSH
//! agent crate: bootstrap detection, credential creation, host sync, extension
//! actions, and deferred post-ReportHosts matching.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use serde_json::json;
use uptrakit_command::RemoteExecutor;
use uptrakit_plugin_infrastructure_core::agent_infra::{
    AgentInfraPlugin, BootstrapInfraResult, GuestExecProvider, InfraPluginContext,
    InfraResolvedSudo, PluginConfigReport, SyncInfraResult,
};
use uptrakit_plugin_infrastructure_core::error::{PluginError, Result};

use uptrakit_extension_framework::{
    ActionDef, ActionUi, ExtensionManifest, ExtensionRequestPayload, ExtensionResponsePayload,
    FieldDef, FieldType, FormDef, SelectOption, SelectSource,
};

use crate::pve_setup;

use super::db_ops;
use super::extension_actions;

/// Proxmox VE agent infrastructure plugin.
pub struct ProxmoxAgentPlugin;

impl ProxmoxAgentPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ProxmoxAgentPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentInfraPlugin for ProxmoxAgentPlugin {
    fn plugin_type(&self) -> &str {
        "infrastructure_proxmox"
    }

    fn migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(super::migration::CreateProxmoxHostState),
            Box::new(super::migration::CreateProxmoxPendingMatches),
        ]
    }

    fn extension_manifests(&self) -> Vec<ExtensionManifest> {
        // The Proxmox plugin does not register its own top-level extension
        // manifest — it contributes actions to the SSH agent's existing
        // `ssh-agent.hosts` manifest via `primary_action_ids`.
        vec![]
    }

    fn extension_actions(&self) -> Vec<ActionDef> {
        vec![
            ActionDef::new("list-pve-hosts", "List PVE Hosts")
                .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
                .with_timeout(10),
            ActionDef::new("list-discovered-guests", "List Discovered Guests")
                .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
                .with_timeout(15),
            bootstrap_proxmox_action(),
            bootstrap_proxmox_guest_action(),
        ]
    }

    fn primary_action_ids(&self) -> Vec<String> {
        vec![
            "bootstrap-proxmox".to_string(),
            "bootstrap-proxmox-guest".to_string(),
        ]
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
            plugin_type: "infrastructure_proxmox".to_string(),
            name: format!("pve-{host_id}"),
            config: json!({
                "api_url": creds.api_url,
                "api_token": creds.api_token,
                "verify_ssl": true,
            }),
        });

        Ok(BootstrapInfraResult {
            report_plugin_config: report,
            existing_plugin_config_id: existing_config_id,
            detected: true,
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
        let canonical_config_id: Option<String> = if let Some(tid) = ctx.tenant_id {
            let tid_uuid = uuid::Uuid::parse_str(tid).ok();
            if let Some(tid_uuid) = tid_uuid {
                match pve_setup::check_pve_token_exists(executor, &tid_uuid).await {
                    Ok(pve_setup::PveTokenStatus::OwnedByTenant(_)) => {
                        let cluster_nodes = pve_setup::detect_pve_cluster_nodes(executor).await;
                        if cluster_nodes.is_empty() {
                            None
                        } else {
                            reconcile_pve_config(ctx.db, &host_id_str, &cluster_nodes).await
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

        // Step 3: privilege verification.
        if let Some(tid) = ctx.tenant_id {
            if let Ok(tid_uuid) = uuid::Uuid::parse_str(tid) {
                match pve_setup::verify_pve_privileges(executor, &tid_uuid).await {
                    Ok(()) => lines.push("privileges: OK (PVEAuditor on /)".to_string()),
                    Err(e) => {
                        lines.push(format!("privileges: FAILED — {e}"));
                        lines.push(
                            "run bootstrap again or manually grant PVEAuditor on / to the \
                             Uptrakit user"
                                .to_string(),
                        );
                    }
                }
            }
        } else {
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
        })
    }

    async fn has_infra_state(&self, db: &DatabaseConnection, host_id: uuid::Uuid) -> bool {
        db_ops::find_host_state(db, &host_id.to_string())
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.is_pve_node)
    }

    async fn handle_extension_action(
        &self,
        ctx: &InfraPluginContext<'_>,
        request: &ExtensionRequestPayload,
    ) -> Option<ExtensionResponsePayload> {
        extension_actions::handle_action(ctx, request).await
    }

    async fn on_post_report_hosts(&self, ctx: &InfraPluginContext<'_>) -> Result<()> {
        let pending = db_ops::drain_pending_matches(ctx.db)
            .await
            .context_to::<PluginError>()?;

        if pending.is_empty() {
            return Ok(());
        }

        tracing::info!(
            count = pending.len(),
            "draining pending Proxmox host matches"
        );

        for entry in pending {
            let match_params = json!({
                "mapping_id": entry.mapping_id,
                "host_id": entry.host_id,
            });

            match ctx
                .action_invoker
                .invoke("proxmox.hosts", "match", match_params)
                .await
            {
                Ok(resp) if resp.success => {
                    tracing::info!(
                        host_id = %entry.host_id,
                        mapping_id = %entry.mapping_id,
                        "auto-matched bootstrapped guest to Proxmox host mapping"
                    );
                    if let Err(e) = db_ops::delete_pending_match(ctx.db, entry.id).await {
                        tracing::warn!(
                            id = entry.id,
                            error = %e,
                            "matched but failed to delete pending Proxmox match row"
                        );
                    }
                }
                Ok(resp) => {
                    tracing::warn!(
                        host_id = %entry.host_id,
                        mapping_id = %entry.mapping_id,
                        error = ?resp.error,
                        "pending Proxmox match failed; will retry on next ReportHosts"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        host_id = %entry.host_id,
                        mapping_id = %entry.mapping_id,
                        error = %e,
                        "pending Proxmox match proxy call failed; will retry on next ReportHosts"
                    );
                }
            }
        }

        Ok(())
    }

    fn guest_exec_provider(&self) -> Option<Arc<dyn GuestExecProvider>> {
        Some(Arc::new(
            super::guest_exec_adapter::ProxmoxGuestExecProvider,
        ))
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
                    tracing::warn!(
                        "no local PVE host has a plugin config ID to reuse; configure the Proxmox plugin manually"
                    );
                    return (None, None);
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

fn bootstrap_proxmox_action() -> ActionDef {
    ActionDef::new("bootstrap-proxmox", "Bootstrap via Proxmox")
        .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
        .with_timeout(120)
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("pve_host_id", "PVE Host")
                .with_type(FieldType::Select)
                .required()
                .with_help_text("PVE node to use as gateway.")
                .with_select_source(SelectSource::Action {
                    action_id: "list-pve-hosts".to_string(),
                }),
            FieldDef::new("vmid", "Guest VMID")
                .required()
                .with_placeholder("100")
                .with_help_text("VMID of the target container or virtual machine."),
            FieldDef::new("guest_type", "Guest Type")
                .with_type(FieldType::Select)
                .required()
                .with_default_value("lxc")
                .with_options(vec![
                    SelectOption::new("lxc", "LXC Container"),
                    SelectOption::new("qemu", "QEMU VM"),
                ]),
            FieldDef::new("name", "Host Name")
                .required()
                .with_placeholder("my-container")
                .with_help_text("Friendly name for identification."),
            FieldDef::new("target_username", "Target Username")
                .with_help_text("User to create/use in the guest.")
                .with_default_value("uptrakit"),
            FieldDef::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            FieldDef::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(FieldType::Toggle)
                .with_help_text(
                    "Remove existing Uptrakit-managed keys from authorized_keys before \
                     writing the new entry. Same-service keys are always removed regardless.",
                ),
        ])))
}

fn bootstrap_proxmox_guest_action() -> ActionDef {
    ActionDef::new("bootstrap-proxmox-guest", "Bootstrap Discovered Guest")
        .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
        .with_timeout(300)
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("discovered_guests", "Discovered Guests")
                .with_type(FieldType::MultiSelect)
                .required()
                .with_help_text(
                    "Select one or more Proxmox guests to bootstrap. \
                     Names are auto-derived from the guest's hostname.",
                )
                .with_select_source(SelectSource::Action {
                    action_id: "list-discovered-guests".to_string(),
                }),
            FieldDef::new("target_username", "Target Username")
                .with_help_text("User to create/use in each guest.")
                .with_default_value("uptrakit"),
            FieldDef::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            FieldDef::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(FieldType::Toggle)
                .with_help_text(
                    "Remove existing Uptrakit-managed keys from authorized_keys before \
                     writing the new entry. Same-service keys are always removed regardless.",
                ),
        ])))
}
