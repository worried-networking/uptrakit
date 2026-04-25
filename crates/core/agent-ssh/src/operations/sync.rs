//! Sync operation: reconcile host state with the remote system.
//!
//! Regenerates the sudoers drop-in file with current plugin commands,
//! detects and stores the PVE node name (for Proxmox guest matching),
//! and verifies PVE API user privileges (when a tenant ID is available).
//!
//! The operation is split into two phases:
//!
//! 1. **Connect** (`sync_connect`) -- connects via SSH, detects host
//!    capabilities, collects plugin requirements, and returns a
//!    [`SyncPlan`] describing the actions that *would* be performed.
//!
//! 2. **Execute** (`sync_execute`) -- reconnects and carries out the
//!    planned actions, optionally skipping those the caller marks in
//!    `skip_actions`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_registry::agent_infra::{
    GuestBootstrapError, GuestBootstrapExecutor, GuestBootstrapParams, GuestBootstrapResult,
    InfraActionInvokeError, InfraActionInvoker, InfraPluginContext, PluginConfigReport,
};
use uptrakit_plugin_infrastructure_registry::{
    CatalogConfig, build_catalog, compatible_sudo_commands_for_host,
};

use crate::db::entity::ssh_host::Model as SshHostModel;
use crate::host_ops::{self, update_host_sudo_state};
use crate::operations::sudoers::{
    ResolvedSudoCommand, SudoersContent, detect_is_root, detect_sudo_available,
    ensure_docker_group_membership, install_helper_script, resolve_command_path,
    write_sudoers_file,
};
use crate::remote_exec::SshRemoteExecutor;
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig, SshSession};

// ── Noop infra impls for sync context ────────────────────────────────

/// No-op [`InfraActionInvoker`] for sync context.
pub(crate) struct NoopInfraActionInvoker;

type InfraActionInvokeResult = std::result::Result<
    uptrakit_internal_wire::surfaces::SurfaceActionResponse,
    InfraActionInvokeError,
>;

#[async_trait]
impl InfraActionInvoker for NoopInfraActionInvoker {
    async fn invoke(
        &self,
        _extension_id: &str,
        _action_id: &str,
        _params: serde_json::Value,
    ) -> InfraActionInvokeResult {
        Err(InfraActionInvokeError::from(
            "InfraActionInvoker not available during sync",
        ))
    }
}

/// No-op [`GuestBootstrapExecutor`] for sync context.
pub(crate) struct NoopGuestBootstrap;

#[async_trait]
impl GuestBootstrapExecutor for NoopGuestBootstrap {
    async fn bootstrap_guest(
        &self,
        _params: GuestBootstrapParams,
    ) -> std::result::Result<GuestBootstrapResult, GuestBootstrapError> {
        Err(GuestBootstrapError::from(
            "GuestBootstrapExecutor not available during sync",
        ))
    }
}

/// Authentication override for the extension sync action.
///
/// When the UI user selects a non-stored auth method, the extension handler
/// populates this struct from the form fields and ECIES-decrypted sensitive
/// params.
pub(crate) struct SyncAuthOverride {
    /// Username to connect as (e.g. `root`).
    pub username: String,
    /// Password authentication (mutually exclusive with `auth_private_key_pem`).
    pub auth_password: Option<String>,
    /// PEM-encoded private key (mutually exclusive with `auth_password`).
    pub auth_private_key_pem: Option<String>,
}

// ── Sync plan types ──────────────────────────────────────────────────

/// Well-known action IDs used in the sync plan.
const ACTION_UPDATE_SUDOERS: &str = "update_sudoers";
const ACTION_DOCKER_GROUP: &str = "docker_group";
const ACTION_INFRA_SYNC: &str = "infra_sync";

/// Information gathered about the target host during the sync connect phase.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SyncHostInfo {
    pub hostname: String,
    pub port: u16,
    pub connect_user: String,
    pub is_root: bool,
    pub sudo_available: bool,
}

/// The result of the sync connect phase: a plan for the user to review.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SyncPlan {
    pub host_info: SyncHostInfo,
    pub actions: Vec<SyncPlannedAction>,
}

/// A planned sync action.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SyncPlannedAction {
    pub id: String,
    pub label: String,
    pub description: String,
    pub security_impact: uptrakit_shared_types::Severity,
    pub default_enabled: bool,
    pub skippable: bool,
    /// Human-readable preview of the commands this action will run or configure.
    pub commands: Vec<String>,
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Resolve SSH connection parameters and authenticate against the host.
///
/// Returns the session, the connect-username reference, and whether an
/// auth override was used.
async fn establish_session(
    host: &SshHostModel,
    auth_override: Option<&SyncAuthOverride>,
) -> std::result::Result<(Arc<SshSession>, String, bool), String> {
    let stored_fingerprint = host
        .host_key_fingerprint
        .as_deref()
        .ok_or_else(|| format!("host '{}' has no stored key fingerprint", host.name))?;

    let config = SshConnectionConfig {
        hostname: host.hostname.clone(),
        port: host.port as u16,
        connect_timeout: Duration::from_secs(30),
    };

    let key_pem_owned: String;
    let connect_username: String;
    let has_override = auth_override.is_some();

    let auth: AuthMethod<'_>;

    // We need the owned values to outlive `auth`, so we declare them
    // outside the if/else and borrow from them.
    let password_owned: Option<String>;
    let pem_owned: Option<String>;

    if let Some(ov) = auth_override {
        connect_username = ov.username.clone();
        password_owned = ov.auth_password.clone();
        pem_owned = ov.auth_private_key_pem.clone();
        if let Some(ref pw) = password_owned {
            auth = AuthMethod::Password(pw.as_str());
        } else if let Some(ref pem) = pem_owned {
            auth = AuthMethod::PrivateKey(pem.as_str());
        } else {
            return Err(
                "auth override provided but neither password nor private key set".to_string(),
            );
        }
    } else {
        key_pem_owned = host.private_key.expose_secret().to_string();
        connect_username = host.username.clone();
        password_owned = None;
        pem_owned = None;
        auth = AuthMethod::PrivateKey(&key_pem_owned);
    }

    // Suppress unused-variable warnings for the owned buffers whose sole
    // purpose is keeping the borrowed `AuthMethod` alive.
    let _ = (&password_owned, &pem_owned);

    let (session, _fingerprint) = crate::ssh_transport::connect_and_authenticate(
        &config,
        &connect_username,
        &auth,
        Some(stored_fingerprint),
    )
    .await
    .map_err(|e| format!("SSH connection failed: {e}"))?;

    Ok((Arc::new(session), connect_username, has_override))
}

/// Detect root / sudo state and persist it (when not using an auth override).
async fn detect_and_persist_sudo_state(
    executor: &SshRemoteExecutor,
    db: &DatabaseConnection,
    host_id: uuid::Uuid,
    has_auth_override: bool,
    connect_username: &str,
    hostname: &str,
) -> std::result::Result<(bool, bool), String> {
    let is_root = detect_is_root(executor)
        .await
        .map_err(|e| format!("failed to detect root status: {e}"))?;
    let sudo_available = if is_root {
        false
    } else {
        detect_sudo_available(executor)
            .await
            .map_err(|e| format!("failed to detect sudo status: {e}"))?
    };

    let agent_is_root = if has_auth_override { false } else { is_root };
    let persisted_sudo_available = if has_auth_override {
        None
    } else {
        Some(sudo_available)
    };
    update_host_sudo_state(
        db,
        host_id,
        persisted_sudo_available,
        Some(agent_is_root),
        None,
    )
    .await
    .map_err(|e| format!("failed to update sudo state: {e}"))?;

    if !is_root && !sudo_available {
        return Err(format!(
            "sudo is not available for user '{connect_username}' on '{hostname}'; cannot sync",
        ));
    }

    Ok((is_root, sudo_available))
}

/// Return `(plugin_type_id, step_previews, security_impact)` for each infra
/// plugin that has state for the given host.
async fn active_infra_plugins(
    db: &DatabaseConnection,
    host_id: uuid::Uuid,
) -> Vec<(String, Vec<String>, uptrakit_shared_types::Severity)> {
    let catalog_config = CatalogConfig::default();
    let Ok(catalog) = build_catalog(&catalog_config) else {
        return vec![];
    };
    let infra_bundles = catalog.create_infra_bundles(&catalog_config);
    let mut result = Vec::new();
    for bundle in &infra_bundles {
        if let (Some(report), Some(lifecycle)) = (bundle.report.as_ref(), bundle.lifecycle.as_ref())
        {
            if report.has_infra_state(db, host_id).await {
                result.push((
                    lifecycle.plugin_type_id().to_string(),
                    lifecycle.sync_step_previews(),
                    lifecycle.sync_security_impact(),
                ));
            }
        }
    }
    result
}

// ── Phase 1: connect ─────────────────────────────────────────────────

/// Connect to the host, inspect its state, and return a plan describing
/// the actions that would be performed during `sync_execute`.
pub(crate) async fn sync_connect(
    host_id: &str,
    db: &DatabaseConnection,
    tenant_id: Option<uuid::Uuid>,
    auth_override: Option<&SyncAuthOverride>,
    allow_all: bool,
) -> std::result::Result<SyncPlan, String> {
    let host = host_ops::find_host(db, host_id)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| format!("host '{host_id}' not found"))?;

    let (session, connect_username, has_auth_override) =
        establish_session(&host, auth_override).await?;
    let executor = SshRemoteExecutor::new(Arc::clone(&session));

    let (is_root, sudo_available) = match detect_and_persist_sudo_state(
        &executor,
        db,
        host.id,
        has_auth_override,
        &connect_username,
        &host.hostname,
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            drop(executor);
            SshSession::disconnect_shared(session).await;
            return Err(e);
        }
    };

    // Collect plugin sudo commands to determine whether sudoers would change.
    let ssh_executor = Arc::new(SshCommandExecutor::new(Arc::clone(&session)))
        as Arc<dyn uptrakit_command::CommandExecutor>;
    let plugin_sudo_cmds = compatible_sudo_commands_for_host(ssh_executor).await;
    let has_sudo_commands = plugin_sudo_cmds.iter().any(|(_, v)| !v.is_empty());

    // Build a human-readable preview of every command that would appear in sudoers.
    let sudo_command_previews: Vec<String> = plugin_sudo_cmds
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .map(|entry| {
            if entry.helper_script.is_some() {
                format!("{} [helper script] — {}", entry.command, entry.explanation)
            } else if let Some(ref suffix) = entry.args_suffix {
                format!("{} {} — {}", entry.command, suffix, entry.explanation)
            } else {
                format!("{} — {}", entry.command, entry.explanation)
            }
        })
        .collect();

    // Determine which infra plugins are active for this host.
    let active_infra = active_infra_plugins(db, host.id).await;

    drop(executor);
    SshSession::disconnect_shared(session).await;

    // Build actions list.
    let mut actions = Vec::new();

    let sudoers_desc = if has_sudo_commands {
        format!(
            "Write /etc/sudoers.d/uptrakit-{} granting NOPASSWD access to {} plugin command(s).",
            host.username,
            sudo_command_previews.len()
        )
    } else if allow_all {
        format!(
            "Write /etc/sudoers.d/uptrakit-{} granting NOPASSWD: ALL (no plugin commands detected).",
            host.username
        )
    } else {
        "No sudoers changes needed (no plugin commands detected).".to_string()
    };

    actions.push(SyncPlannedAction {
        id: ACTION_UPDATE_SUDOERS.to_string(),
        label: "Update sudoers".to_string(),
        description: sudoers_desc,
        security_impact: uptrakit_shared_types::Severity::High,
        default_enabled: has_sudo_commands || allow_all,
        skippable: true,
        commands: sudo_command_previews,
    });

    actions.push(SyncPlannedAction {
        id: ACTION_DOCKER_GROUP.to_string(),
        label: "Docker group membership".to_string(),
        description: "Add the connect user to the docker group (if Docker is installed)."
            .to_string(),
        security_impact: uptrakit_shared_types::Severity::Low,
        default_enabled: true,
        skippable: true,
        commands: vec![],
    });

    if !active_infra.is_empty() {
        let plugin_list = active_infra
            .iter()
            .map(|(id, ..)| id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let security_impact = active_infra
            .iter()
            .map(|(.., impact)| *impact)
            .max()
            .unwrap_or_default();
        let infra_commands = active_infra
            .into_iter()
            .flat_map(|(_, steps, _)| steps)
            .collect();
        actions.push(SyncPlannedAction {
            id: ACTION_INFRA_SYNC.to_string(),
            label: "Infrastructure plugin sync".to_string(),
            description: format!("Run host-sync hooks for: {plugin_list}."),
            security_impact,
            default_enabled: true,
            skippable: true,
            commands: infra_commands,
        });
    }

    let _ = tenant_id; // used later when infra plugins need it in execute phase

    Ok(SyncPlan {
        host_info: SyncHostInfo {
            hostname: host.hostname.clone(),
            port: host.port as u16,
            connect_user: connect_username,
            is_root,
            sudo_available,
        },
        actions,
    })
}

// ── Phase 2: execute ─────────────────────────────────────────────────

/// Reconnect and execute the planned sync actions, skipping any whose ID
/// appears in `skip_actions`.
///
/// Returns `(summary, plugin_config_reports)` on success.
pub(crate) async fn sync_execute(
    host_id: &str,
    db: &DatabaseConnection,
    tenant_id: Option<uuid::Uuid>,
    auth_override: Option<&SyncAuthOverride>,
    allow_all: bool,
    skip_actions: &HashSet<String>,
) -> std::result::Result<(String, Vec<PluginConfigReport>), String> {
    let host = host_ops::find_host(db, host_id)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| format!("host '{host_id}' not found"))?;

    let (session, connect_username, has_auth_override) =
        establish_session(&host, auth_override).await?;
    let executor = SshRemoteExecutor::new(Arc::clone(&session));

    let (is_root, _sudo_available) = match detect_and_persist_sudo_state(
        &executor,
        db,
        host.id,
        has_auth_override,
        &connect_username,
        &host.hostname,
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            drop(executor);
            SshSession::disconnect_shared(session).await;
            return Err(e);
        }
    };

    let privileged = !is_root;
    let mut summary = Vec::new();
    let mut plugin_config_reports: Vec<PluginConfigReport> = Vec::new();

    // ── Sudoers ──────────────────────────────────────────────────────
    if !skip_actions.contains(ACTION_UPDATE_SUDOERS) {
        let ssh_executor = Arc::new(SshCommandExecutor::new(Arc::clone(&session)))
            as Arc<dyn uptrakit_command::CommandExecutor>;
        let plugin_sudo_cmds = compatible_sudo_commands_for_host(ssh_executor).await;
        let mut resolved: Vec<ResolvedSudoCommand> = Vec::new();

        for (_plugin_type, entries) in &plugin_sudo_cmds {
            for entry in entries {
                if let Some(helper) = &entry.helper_script {
                    install_helper_script(&executor, helper, privileged)
                        .await
                        .map_err(|e| format!("failed to install helper script: {e}"))?;
                    resolved.push(ResolvedSudoCommand {
                        command_path: helper.install_path.to_string(),
                        explanation: entry.explanation.clone(),
                        needs_setenv: entry.needs_setenv,
                    });
                } else if let Some(path) = resolve_command_path(&executor, &entry.command)
                    .await
                    .map_err(|e| format!("failed to resolve command path: {e}"))?
                {
                    let command_path = match &entry.args_suffix {
                        Some(suffix) => format!("{path} {suffix}"),
                        None => path,
                    };
                    resolved.push(ResolvedSudoCommand {
                        command_path,
                        explanation: entry.explanation.clone(),
                        needs_setenv: entry.needs_setenv,
                    });
                }
            }
        }

        let has_resolved_commands = !resolved.is_empty();
        let sudoers_content: Option<SudoersContent> = if has_resolved_commands {
            Some(SudoersContent::SpecificCommands(resolved))
        } else if allow_all {
            Some(SudoersContent::AllCommands)
        } else {
            None
        };

        if let Some(ref content) = sudoers_content {
            write_sudoers_file(&executor, &host.username, content, privileged)
                .await
                .map_err(|e| format!("failed to write sudoers file: {e}"))?;
            if !has_auth_override {
                update_host_sudo_state(db, host.id, Some(true), Some(is_root), None)
                    .await
                    .map_err(|e| format!("failed to update sudo state: {e}"))?;
            }
            if allow_all && !has_resolved_commands {
                summary.push("sudoers: updated (NOPASSWD: ALL)".to_string());
            } else {
                summary.push("sudoers: updated".to_string());
            }
        } else {
            summary.push("sudoers: no commands to write".to_string());
        }
    } else {
        summary.push("sudoers: skipped".to_string());
    }

    // ── Docker group ─────────────────────────────────────────────────
    if !skip_actions.contains(ACTION_DOCKER_GROUP) {
        ensure_docker_group_membership(&executor, &host.username, privileged)
            .await
            .map_err(|e| format!("failed to configure docker group: {e}"))?;
    } else {
        summary.push("docker group: skipped".to_string());
    }

    // ── Infra plugins ────────────────────────────────────────────────
    if !skip_actions.contains(ACTION_INFRA_SYNC) {
        let catalog_config = CatalogConfig::default();
        if let Ok(catalog) = build_catalog(&catalog_config) {
            let infra_bundles = catalog.create_infra_bundles(&catalog_config);
            let noop_invoker = NoopInfraActionInvoker;
            let noop_bootstrap = NoopGuestBootstrap;
            let tenant_id_str = tenant_id.map(|t| t.to_string());
            let infra_ctx = InfraPluginContext {
                db,
                tenant_id: tenant_id_str.as_deref(),
                service_id: None,
                state_dir: std::path::Path::new("."),
                private_key_der: None,
                action_invoker: &noop_invoker,
                guest_bootstrap: &noop_bootstrap,
            };

            // We need to collect infra sudo commands into the sudoers set that
            // was already written above. For now the infra sync step only adds
            // summary lines; infra sudo commands were already resolved in the
            // sudoers block via `compatible_sudo_commands_for_host`.
            for bundle in &infra_bundles {
                let (Some(report), Some(lifecycle)) =
                    (bundle.report.as_ref(), bundle.lifecycle.as_ref())
                else {
                    continue;
                };
                if report.has_infra_state(db, host.id).await {
                    match lifecycle
                        .on_host_synced(&infra_ctx, &executor, host.id)
                        .await
                    {
                        Ok(sync_result) => {
                            for line in sync_result.summary_lines {
                                summary.push(format!("{}: {line}", lifecycle.plugin_type_id()));
                            }
                            if let Some(config_report) = sync_result.report_plugin_config {
                                plugin_config_reports.push(config_report);
                            }
                        }
                        Err(e) => {
                            summary
                                .push(format!("{}: sync failed ({e})", lifecycle.plugin_type_id()));
                        }
                    }
                }
            }
        }
    } else {
        summary.push("infra sync: skipped".to_string());
    }

    drop(executor);
    SshSession::disconnect_shared(session).await;

    Ok((summary.join("; "), plugin_config_reports))
}
