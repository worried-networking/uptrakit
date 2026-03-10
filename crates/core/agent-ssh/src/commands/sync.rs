//! `sync` command: reconcile host state with the remote system.
//!
//! Replaces the former `update-sudoers` command. In addition to regenerating
//! the sudoers drop-in file, `sync` also:
//!
//! - Detects and stores the PVE node name (for Proxmox guest matching)
//! - Verifies PVE API user privileges (when a tenant ID is available)

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_core::agent_infra::{
    GuestBootstrapExecutor, GuestBootstrapParams, GuestBootstrapResult, InfraActionInvoker,
    InfraPluginContext,
};
use uptrakit_plugin_infrastructure_registry::{PluginRegistry, create_agent_infra_registry};

use crate::commands::sudoers::{
    self, ResolvedSudoCommand, SudoersContent, detect_is_root, detect_sudo_available,
    install_helper_script, resolve_command_path, write_sudoers_file,
};
use crate::db::entity::ssh_host::Model;
use crate::error::{Error, Result};
use crate::host_ops::{self, update_host_sudo_state};
use crate::remote_exec::SshRemoteExecutor;
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_target::SshTarget;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig, SshSession};

/// Arguments for the `sync` command.
pub struct SyncArgs {
    /// Host name, UUID, or SSH address (`[user@]host[:port]` /
    /// `ssh://[user@]host[:port]`).
    ///
    /// When the value contains `@` or starts with `ssh://` it is parsed as an
    /// SSH address and the host is looked up by hostname (and port, if
    /// present).  Otherwise the value is treated as a host name or UUID.
    ///
    /// When a username is present in the SSH address *and* it differs from the
    /// stored host username, that username is used for the connection together
    /// with the supplied authentication credentials (or SSH agent).
    pub name_or_id: String,
    /// Password for authenticating as the SSH address username (when it
    /// differs from the stored host username).
    pub auth_password: Option<String>,
    /// Private key PEM for authenticating as the SSH address username.
    pub auth_private_key_pem: Option<String>,
    /// Use the local SSH agent (`SSH_AUTH_SOCK`) for authentication when the
    /// SSH address username differs from the stored host username.
    pub use_ssh_agent: bool,
    /// Force `NOPASSWD: ALL` instead of specific command entries.
    pub allow_all: bool,
    /// Preview changes without writing them.
    pub dry_run: bool,
    /// Tenant UUID for PVE privilege verification.  `None` when the tenant
    /// ID has not yet been received from the controller (CLI mode).
    pub tenant_id: Option<uuid::Uuid>,
}

/// Resolve the target host from `name_or_id`.
///
/// Returns `(host, url_username)` where `url_username` is `Some` when the
/// SSH address included an explicit `user@` prefix that should be used as
/// the connection username instead of the stored host username.
async fn resolve_host(
    db: &DatabaseConnection,
    name_or_id: &str,
) -> Result<(Model, Option<String>)> {
    if name_or_id.contains('@') || name_or_id.starts_with("ssh://") {
        // Parse as SSH target and find by hostname (+ optional port).
        let target = name_or_id.parse::<SshTarget>().map_err(|e| {
            report!(Error::InvalidInput(format!(
                "invalid SSH address '{name_or_id}': {e}"
            )))
        })?;

        let matches = host_ops::find_hosts_by_hostname(db, &target.hostname, target.port).await?;

        match matches.len() {
            0 => bail!(Error::HostNotFound(name_or_id.to_string())),
            1 => Ok((
                matches
                    .into_iter()
                    .next()
                    .ok_or_else(|| report!(Error::HostNotFound(name_or_id.to_string())))?,
                target.username,
            )),
            _ => bail!(Error::InvalidInput(format!(
                "multiple hosts found for '{}'; use the host name or UUID to \
                 disambiguate (see `host list`)",
                name_or_id
            ))),
        }
    } else {
        // Name or UUID lookup.
        let host = host_ops::find_host(db, name_or_id)
            .await?
            .ok_or_else(|| report!(Error::HostNotFound(name_or_id.to_string())))?;
        Ok((host, None))
    }
}

/// Run the `sync` command.
pub async fn run(args: &SyncArgs, db: &DatabaseConnection) -> Result<()> {
    // 1. Load SSH host from DB (supports name, UUID, and SSH address).
    let (host, url_username) = resolve_host(db, &args.name_or_id).await?;

    // 2. Determine connection username and authentication method.
    let connect_username: &str;
    let auth: AuthMethod<'_>;

    // `key_pem` owns the PEM string when we borrow from the stored host key.
    // Declared here so its lifetime covers the `auth` borrow below.
    let key_pem: String;

    let auth_override = url_username
        .as_deref()
        .map(|u| u != host.username.as_str())
        .unwrap_or(false);

    if auth_override {
        let override_user = url_username.as_deref().ok_or_else(|| {
            report!(Error::InvalidInput(
                "expected username from SSH address but none was present".to_string()
            ))
        })?;
        connect_username = override_user;
        auth = match (
            &args.auth_password,
            &args.auth_private_key_pem,
            args.use_ssh_agent,
        ) {
            (Some(pw), _, _) => AuthMethod::Password(pw.as_str()),
            (_, Some(pem), _) => AuthMethod::PrivateKey(pem.as_str()),
            (_, _, true) => AuthMethod::Agent,
            _ => bail!(Error::InvalidInput(format!(
                "no authentication method available for '{override_user}': use \
                 --auth-password, --auth-private-key-file, or ensure SSH_AUTH_SOCK \
                 is set for SSH agent forwarding"
            ))),
        };
    } else {
        key_pem = host.private_key.expose_secret().to_string();
        connect_username = &host.username;
        auth = AuthMethod::PrivateKey(&key_pem);
    }

    // 3. Establish SSH session (strict host key checking — no TOFU allowed).
    let stored_fingerprint = host.host_key_fingerprint.as_deref().ok_or_else(|| {
        report!(Error::InvalidInput(format!(
            "host '{}' has no stored key fingerprint; cannot connect without \
             TOFU. Set it with: host update {} --host-key-fingerprint SHA256:...",
            host.name, host.name
        )))
    })?;

    let config = SshConnectionConfig {
        hostname: host.hostname.clone(),
        port: host.port as u16,
        connect_timeout: Duration::from_secs(30),
    };

    println!(
        "Connecting to {}:{} as '{}'...",
        host.hostname, host.port, connect_username
    );

    let (session, _fingerprint) = crate::ssh_transport::connect_and_authenticate(
        &config,
        connect_username,
        &auth,
        Some(stored_fingerprint),
    )
    .await?;

    let session = Arc::new(session);
    let executor = SshRemoteExecutor::new(Arc::clone(&session));

    // 4. Detect sudo state.
    println!("Detecting privilege context...");
    let is_root = detect_is_root(&executor).await?;
    let sudo_available = if is_root {
        false
    } else {
        detect_sudo_available(&executor).await?
    };

    let agent_is_root = if auth_override { false } else { is_root };

    let persisted_sudo_available = if auth_override {
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
    .await?;

    tracing::info!(
        is_root,
        agent_is_root,
        sudo_available,
        auth_override,
        "sudo state detected and persisted"
    );

    // 5. Permission check.
    if !is_root && !sudo_available {
        bail!(Error::InvalidInput(format!(
            "sudo is not available for user '{}' on '{}'. Cannot sync. \
             Either connect as root or ensure the user has passwordless sudo first.",
            connect_username, host.hostname
        )));
    }

    let privileged = !is_root;

    // 6. Collect plugin commands + resolve paths + write sudoers.
    let ssh_executor = Arc::new(SshCommandExecutor::new(Arc::clone(&session)))
        as Arc<dyn uptrakit_command::CommandExecutor>;
    let plugin_sudo_cmds = PluginRegistry::compatible_sudo_commands_for_host(ssh_executor).await;
    let mut resolved: Vec<ResolvedSudoCommand> = Vec::new();

    for (_plugin_type, entries) in &plugin_sudo_cmds {
        for entry in entries {
            if let Some(helper) = &entry.helper_script {
                println!("  Installing helper script '{}'...", helper.install_path);
                install_helper_script(&executor, helper, privileged).await?;
                resolved.push(ResolvedSudoCommand {
                    command_path: helper.install_path.to_string(),
                    explanation: entry.explanation.clone(),
                    needs_setenv: entry.needs_setenv,
                });
            } else {
                match resolve_command_path(&executor, &entry.command).await? {
                    Some(path) => {
                        tracing::debug!(command = %entry.command, path = %path, "resolved command path");
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
                    None => {
                        tracing::debug!(
                            command = %entry.command,
                            "command not found on remote host, skipping"
                        );
                    }
                }
            }
        }
    }

    // Ask infra plugins for additional sudo commands required by this host.
    let infra_registry = create_agent_infra_registry();
    let noop_invoker = NoopInfraActionInvoker;
    let noop_bootstrap = NoopGuestBootstrap;
    let tenant_id_str = args.tenant_id.map(|t| t.to_string());
    let infra_ctx = InfraPluginContext {
        db,
        tenant_id: tenant_id_str.as_deref(),
        service_id: None,
        state_dir: std::path::Path::new("."),
        private_key_der: None,
        action_invoker: &noop_invoker,
        guest_bootstrap: &noop_bootstrap,
    };
    for plugin in infra_registry.plugins() {
        if plugin.has_infra_state(db, host.id).await {
            match plugin.on_host_synced(&infra_ctx, &executor, host.id).await {
                Ok(sync_result) => {
                    for cmd in sync_result.sudo_commands {
                        resolved.push(ResolvedSudoCommand {
                            command_path: cmd.command_path,
                            explanation: cmd.explanation,
                            needs_setenv: cmd.needs_setenv,
                        });
                    }
                    for line in &sync_result.summary_lines {
                        println!("  {line}");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, plugin = %plugin.plugin_type(), "infra plugin sync failed");
                }
            }
        }
    }

    let sudoers_content: Option<SudoersContent> = if !resolved.is_empty() {
        Some(SudoersContent::SpecificCommands(resolved))
    } else if args.allow_all {
        println!("  No plugin commands resolved; using NOPASSWD: ALL (--allow-all).");
        Some(SudoersContent::AllCommands)
    } else {
        println!("  No plugin-specific commands found for this host; nothing to write.");
        println!("  Install supported tools or re-run with --allow-all.");
        None
    };

    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{}", host.username);

    if !args.dry_run {
        if let Some(ref content) = sudoers_content {
            let preview_text = sudoers::generate_sudoers_content(&host.username, content);
            println!("Writing {sudoers_file}...");
            write_sudoers_file(&executor, &host.username, content, privileged).await?;
            update_host_sudo_state(db, host.id, Some(true), Some(agent_is_root), None).await?;

            println!();
            println!("Sudoers updated for host '{}'.", host.name);
            println!("  File: {sudoers_file}");
            println!("  Content:");
            for line in preview_text.lines() {
                println!("    {line}");
            }
        } else {
            println!();
            println!(
                "No sudoers changes for host '{}' — no compatible plugin commands found.",
                host.name
            );
        }
    } else if let Some(ref content) = sudoers_content {
        let preview_text = sudoers::generate_sudoers_content(&host.username, content);
        println!();
        println!("Dry run — sudoers file that would be written to {sudoers_file}:");
        println!("---");
        print!("{preview_text}");
        println!("---");
    } else {
        println!();
        println!("Dry run — no sudoers file would be written (no commands resolved).");
    }

    // Drop the executor before disconnecting so the session Arc has a single
    // owner — `disconnect_shared` requires sole ownership to cleanly close the
    // SSH channel.
    drop(executor);
    SshSession::disconnect_shared(session).await;

    if args.dry_run {
        println!();
        println!("(no changes made — dry run)");
    }

    Ok(())
}

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
pub struct SyncAuthOverride {
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
pub async fn run_for_extension(
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

    let mut summary = Vec::new();

    // Ask infra plugins for additional sudo commands required by this host.
    let infra_registry = create_agent_infra_registry();
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
    for plugin in infra_registry.plugins() {
        if plugin.has_infra_state(db, host.id).await {
            match plugin.on_host_synced(&infra_ctx, &executor, host.id).await {
                Ok(sync_result) => {
                    for cmd in sync_result.sudo_commands {
                        resolved.push(ResolvedSudoCommand {
                            command_path: cmd.command_path,
                            explanation: cmd.explanation,
                            needs_setenv: cmd.needs_setenv,
                        });
                    }
                    for line in sync_result.summary_lines {
                        summary.push(format!("{}: {line}", plugin.plugin_type()));
                    }
                }
                Err(e) => {
                    summary.push(format!("{}: sync failed ({e})", plugin.plugin_type()));
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
