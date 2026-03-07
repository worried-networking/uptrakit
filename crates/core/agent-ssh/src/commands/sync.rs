//! `sync` command: reconcile host state with the remote system.
//!
//! Replaces the former `update-sudoers` command. In addition to regenerating
//! the sudoers drop-in file, `sync` also:
//!
//! - Detects and stores the PVE node name (for Proxmox guest matching)
//! - Verifies PVE API user privileges (when a tenant ID is available)

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_proxmox::pve_setup;
use uptrakit_plugin_infrastructure_registry::PluginRegistry;

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
        &host.id,
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
                        resolved.push(ResolvedSudoCommand {
                            command_path: path,
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

    // Handle dry run for sudoers only — PVE sync still runs in dry-run mode
    // to show what would be detected without making sudoers changes.
    if !args.dry_run {
        if let Some(ref content) = sudoers_content {
            let preview_text = sudoers::generate_sudoers_content(&host.username, content);
            println!("Writing {sudoers_file}...");
            write_sudoers_file(&executor, &host.username, content, privileged).await?;
            update_host_sudo_state(db, &host.id, Some(true), Some(agent_is_root), None).await?;

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

    // 7. PVE node name detection and privilege verification.
    if host.is_pve_node {
        sync_pve_state(&executor, db, &host, args.tenant_id.as_ref(), args.dry_run).await?;
    }

    SshSession::disconnect_shared(session).await;

    if args.dry_run {
        println!();
        println!("(no changes made — dry run)");
    }

    Ok(())
}

/// Sync PVE-specific state: node name and privilege verification.
async fn sync_pve_state(
    executor: &SshRemoteExecutor,
    db: &DatabaseConnection,
    host: &Model,
    tenant_id: Option<&uuid::Uuid>,
    dry_run: bool,
) -> Result<()> {
    println!();
    println!("Syncing Proxmox VE state...");

    // Collect PVE node name.
    let node_name = match pve_setup::detect_pve_node_name(executor).await {
        Ok(name) => {
            println!("  PVE node name: {name}");
            Some(name)
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to detect PVE node name");
            println!("  Warning: could not detect PVE node name: {e}");
            None
        }
    };

    // Persist node name (unless dry run).
    if !dry_run && node_name.is_some() {
        host_ops::update_host_pve_state(
            db,
            &host.id,
            true,
            host.pve_plugin_config_id.clone(),
            node_name.clone(),
        )
        .await?;
    }

    // Verify PVE privileges (requires tenant_id).
    if let Some(tid) = tenant_id {
        match pve_setup::verify_pve_privileges(executor, tid).await {
            Ok(()) => {
                println!("  PVE privileges: OK (PVEAuditor on /)");
            }
            Err(e) => {
                println!("  PVE privileges: FAILED — {e}");
                println!(
                    "  Run bootstrap again or manually grant PVEAuditor on / to the Uptrakit user."
                );
            }
        }
    } else {
        println!("  PVE privilege check: skipped (tenant ID not available in CLI mode)");
    }

    Ok(())
}

/// Run the sync command from an extension action (no auth override, no dry run).
///
/// Used by the UI extension to sync a host by its database ID, using the
/// stored SSH key for authentication.
pub async fn run_for_extension(
    host_id: &str,
    db: &DatabaseConnection,
    tenant_id: Option<uuid::Uuid>,
) -> std::result::Result<String, String> {
    let host = host_ops::find_host(db, host_id)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| format!("host '{host_id}' not found"))?;

    let key_pem = host.private_key.expose_secret().to_string();
    let stored_fingerprint = host
        .host_key_fingerprint
        .as_deref()
        .ok_or_else(|| format!("host '{}' has no stored key fingerprint", host.name))?;

    let config = SshConnectionConfig {
        hostname: host.hostname.clone(),
        port: host.port as u16,
        connect_timeout: Duration::from_secs(30),
    };

    let (session, _fingerprint) = crate::ssh_transport::connect_and_authenticate(
        &config,
        &host.username,
        &AuthMethod::PrivateKey(&key_pem),
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

    update_host_sudo_state(db, &host.id, Some(sudo_available), Some(is_root), None)
        .await
        .map_err(|e| format!("failed to update sudo state: {e}"))?;

    if !is_root && !sudo_available {
        SshSession::disconnect_shared(session).await;
        return Err(format!(
            "sudo is not available for user '{}' on '{}'; cannot sync",
            host.username, host.hostname
        ));
    }

    let privileged = !is_root;

    // Collect + write sudoers.
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
                resolved.push(ResolvedSudoCommand {
                    command_path: path,
                    explanation: entry.explanation.clone(),
                    needs_setenv: entry.needs_setenv,
                });
            }
        }
    }

    let mut summary = Vec::new();

    if !resolved.is_empty() {
        let content = SudoersContent::SpecificCommands(resolved);
        write_sudoers_file(&executor, &host.username, &content, privileged)
            .await
            .map_err(|e| format!("failed to write sudoers file: {e}"))?;
        update_host_sudo_state(db, &host.id, Some(true), Some(is_root), None)
            .await
            .map_err(|e| format!("failed to update sudo state: {e}"))?;
        summary.push("sudoers: updated".to_string());
    } else {
        summary.push("sudoers: no commands to write".to_string());
    }

    // PVE sync.
    if host.is_pve_node {
        match pve_setup::detect_pve_node_name(&executor).await {
            Ok(name) => {
                host_ops::update_host_pve_state(
                    db,
                    &host.id,
                    true,
                    host.pve_plugin_config_id.clone(),
                    Some(name.clone()),
                )
                .await
                .map_err(|e| format!("failed to update PVE state: {e}"))?;
                summary.push(format!("pve_node_name: {name}"));
            }
            Err(e) => {
                summary.push(format!("pve_node_name: detection failed ({e})"));
            }
        }

        if let Some(tid) = tenant_id {
            match pve_setup::verify_pve_privileges(&executor, &tid).await {
                Ok(()) => summary.push("pve_privileges: OK".to_string()),
                Err(e) => summary.push(format!("pve_privileges: FAILED ({e})")),
            }
        } else {
            summary.push("pve_privileges: skipped (no tenant ID)".to_string());
        }
    }

    SshSession::disconnect_shared(session).await;

    Ok(summary.join("; "))
}
