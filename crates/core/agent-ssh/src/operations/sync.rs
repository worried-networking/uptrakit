//! Sync operation: reconcile host state with the remote system.
//!
//! Regenerates the sudoers drop-in file with current plugin commands,
//! detects and stores the PVE node name (for Proxmox guest matching),
//! and verifies PVE API user privileges (when a tenant ID is available).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_core::agent_infra::{
    GuestBootstrapExecutor, GuestBootstrapParams, GuestBootstrapResult, InfraActionInvoker,
    InfraPluginContext,
};
use uptrakit_plugin_infrastructure_registry::{PluginRegistry, create_agent_infra_plugins};

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
struct NoopInfraActionInvoker;

#[async_trait]
impl InfraActionInvoker for NoopInfraActionInvoker {
    async fn invoke(
        &self,
        _extension_id: &str,
        _action_id: &str,
        _params: serde_json::Value,
    ) -> std::result::Result<uptrakit_internal_wire::extension::ExtensionResponsePayload, String>
    {
        Err("InfraActionInvoker not available during sync".to_string())
    }
}

/// No-op [`GuestBootstrapExecutor`] for sync context.
struct NoopGuestBootstrap;

#[async_trait]
impl GuestBootstrapExecutor for NoopGuestBootstrap {
    async fn bootstrap_guest(
        &self,
        _params: GuestBootstrapParams,
    ) -> std::result::Result<GuestBootstrapResult, String> {
        Err("GuestBootstrapExecutor not available during sync".to_string())
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

/// Run the sync command from an extension action.
///
/// When `auth_override` is `None`, uses the stored SSH key and username.
/// When `Some`, connects as the specified user with the provided credentials
/// (sudo state is not persisted for the override user, matching CLI behavior).
pub(crate) async fn run_for_extension(
    host_id: &str,
    db: &DatabaseConnection,
    tenant_id: Option<uuid::Uuid>,
    auth_override: Option<&SyncAuthOverride>,
    allow_all: bool,
) -> std::result::Result<String, String> {
    let host = host_ops::find_host(db, host_id)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| format!("host '{host_id}' not found"))?;

    let stored_fingerprint = host
        .host_key_fingerprint
        .as_deref()
        .ok_or_else(|| format!("host '{}' has no stored key fingerprint", host.name))?;

    let config = SshConnectionConfig {
        hostname: host.hostname.clone(),
        port: host.port as u16,
        connect_timeout: Duration::from_secs(30),
    };

    // Determine connection username and auth method.
    let key_pem: String;
    let connect_username: &str;
    let auth: AuthMethod<'_>;
    let has_auth_override = auth_override.is_some();

    if let Some(ov) = auth_override {
        connect_username = &ov.username;
        if let Some(ref pw) = ov.auth_password {
            auth = AuthMethod::Password(pw.as_str());
        } else if let Some(ref pem) = ov.auth_private_key_pem {
            auth = AuthMethod::PrivateKey(pem.as_str());
        } else {
            return Err(
                "auth override provided but neither password nor private key set".to_string(),
            );
        }
    } else {
        key_pem = host.private_key.expose_secret().to_string();
        connect_username = &host.username;
        auth = AuthMethod::PrivateKey(&key_pem);
    }

    let (session, _fingerprint) = crate::ssh_transport::connect_and_authenticate(
        &config,
        connect_username,
        &auth,
        Some(stored_fingerprint),
    )
    .await
    .map_err(|e| format!("SSH connection failed: {e}"))?;

    let session = Arc::new(session);
    let executor = SshRemoteExecutor::new(Arc::clone(&session));

    // Detect sudo state.
    let is_root = detect_is_root(&executor)
        .await
        .map_err(|e| format!("failed to detect root status: {e}"))?;
    let sudo_available = if is_root {
        false
    } else {
        detect_sudo_available(&executor)
            .await
            .map_err(|e| format!("failed to detect sudo status: {e}"))?
    };

    // Only persist sudo state when using stored credentials (not override).
    let agent_is_root = if has_auth_override { false } else { is_root };
    let persisted_sudo_available = if has_auth_override {
        None
    } else {
        Some(sudo_available)
    };
    update_host_sudo_state(
        db,
        host.id,
        persisted_sudo_available,
        Some(agent_is_root),
        None,
    )
    .await
    .map_err(|e| format!("failed to update sudo state: {e}"))?;

    if !is_root && !sudo_available {
        drop(executor);
        SshSession::disconnect_shared(session).await;
        return Err(format!(
            "sudo is not available for user '{}' on '{}'; cannot sync",
            connect_username, host.hostname
        ));
    }

    let privileged = !is_root;

    // Collect + write sudoers.  `ssh_executor` is consumed by
    // `compatible_sudo_commands_for_host` so only `executor` remains.
    let ssh_executor = Arc::new(SshCommandExecutor::new(Arc::clone(&session)))
        as Arc<dyn uptrakit_command::CommandExecutor>;
    let plugin_sudo_cmds = PluginRegistry::compatible_sudo_commands_for_host(ssh_executor).await;
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

    // Configure docker group membership when Docker is installed on the host.
    ensure_docker_group_membership(&executor, &host.username, privileged)
        .await
        .map_err(|e| format!("failed to configure docker group: {e}"))?;

    let mut summary = Vec::new();

    // Ask infra plugins for additional sudo commands required by this host.
    let infra_plugins = create_agent_infra_plugins();
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
    for plugin in &infra_plugins {
        let Some(lifecycle) = plugin.as_host_lifecycle() else {
            continue;
        };
        if lifecycle.has_infra_state(db, host.id).await {
            match lifecycle
                .on_host_synced(&infra_ctx, &executor, host.id)
                .await
            {
                Ok(sync_result) => {
                    for cmd in sync_result.sudo_commands {
                        resolved.push(ResolvedSudoCommand {
                            command_path: cmd.command_path,
                            explanation: cmd.explanation,
                            needs_setenv: cmd.needs_setenv,
                        });
                    }
                    for line in sync_result.summary_lines {
                        summary.push(format!("{}: {line}", plugin.plugin_type_id()));
                    }
                }
                Err(e) => {
                    summary.push(format!("{}: sync failed ({e})", plugin.plugin_type_id()));
                }
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
        // Persist sudo state only for stored-credentials runs.
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

    drop(executor);
    SshSession::disconnect_shared(session).await;

    Ok(summary.join("; "))
}
