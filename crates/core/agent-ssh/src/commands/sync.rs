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
use uptrakit_plugin_infrastructure_registry::{PluginRegistry, create_agent_infra_plugins};

use crate::commands::sudoers::{
    self, ResolvedSudoCommand, SudoersContent, detect_is_root, detect_sudo_available,
    ensure_docker_group_membership, install_helper_script, resolve_command_path,
    write_sudoers_file,
};
use crate::db::entity::ssh_host::Model;
use crate::error::{Error, Result};
use crate::host_ops::{self, update_host_sudo_state};
use crate::operations::sync::{NoopGuestBootstrap, NoopInfraActionInvoker};
use crate::remote_exec::SshRemoteExecutor;
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_target::SshTarget;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig, SshSession};
use uptrakit_plugin_infrastructure_core::agent_infra::InfraPluginContext;

/// Arguments for the `sync` command.
pub(crate) struct SyncArgs {
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

/// Resolve the connection username and `AuthMethod` for a CLI sync invocation.
///
/// Returns `(connect_username, auth, auth_override)` where `auth_override` is
/// `true` when the caller supplied an explicit username that differs from the
/// stored host username.
///
/// `key_pem` must be declared by the caller so its lifetime covers the
/// returned `AuthMethod` borrow.
fn resolve_cli_auth<'a>(
    host: &'a Model,
    url_username: Option<&'a str>,
    auth_password: Option<&'a str>,
    auth_private_key_pem: Option<&'a str>,
    use_ssh_agent: bool,
    key_pem: &'a str,
) -> Result<(&'a str, AuthMethod<'a>, bool)> {
    let auth_override = url_username
        .map(|u| u != host.username.as_str())
        .unwrap_or(false);

    if auth_override {
        let override_user = url_username.ok_or_else(|| {
            report!(Error::InvalidInput(
                "expected username from SSH address but none was present".to_string()
            ))
        })?;
        let auth = match (auth_password, auth_private_key_pem, use_ssh_agent) {
            (Some(pw), _, _) => AuthMethod::Password(pw),
            (_, Some(pem), _) => AuthMethod::PrivateKey(pem),
            (_, _, true) => AuthMethod::Agent,
            _ => bail!(Error::InvalidInput(format!(
                "no authentication method available for '{override_user}': use \
                 --auth-password, --auth-private-key-file, or ensure SSH_AUTH_SOCK \
                 is set for SSH agent forwarding"
            ))),
        };
        Ok((override_user, auth, true))
    } else {
        Ok((&host.username, AuthMethod::PrivateKey(key_pem), false))
    }
}

/// Open an SSH connection and return an authenticated session.
async fn connect_ssh(
    host: &Model,
    connect_username: &str,
    auth: &AuthMethod<'_>,
) -> Result<Arc<SshSession>> {
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
        auth,
        Some(stored_fingerprint),
    )
    .await?;

    Ok(Arc::new(session))
}

/// Detect root / sudo state and persist it (when not using an auth override).
///
/// Returns `(is_root, sudo_available)`.
async fn detect_and_persist_sudo(
    executor: &SshRemoteExecutor,
    db: &DatabaseConnection,
    host: &Model,
    auth_override: bool,
) -> Result<(bool, bool)> {
    println!("Detecting privilege context...");
    let is_root = detect_is_root(executor).await?;
    let sudo_available = if is_root {
        false
    } else {
        detect_sudo_available(executor).await?
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

    Ok((is_root, sudo_available))
}

/// Collect sudo commands from all registered plugins and resolve their paths
/// on the remote host, installing any required helper scripts.
///
/// Returns the list of [`ResolvedSudoCommand`] entries, with unresolved
/// commands silently skipped.
async fn collect_plugin_sudo_commands(
    session: &Arc<SshSession>,
    executor: &SshRemoteExecutor,
    privileged: bool,
) -> Result<Vec<ResolvedSudoCommand>> {
    let ssh_executor = Arc::new(SshCommandExecutor::new(Arc::clone(session)))
        as Arc<dyn uptrakit_command::CommandExecutor>;
    let plugin_sudo_cmds = PluginRegistry::compatible_sudo_commands_for_host(ssh_executor).await;
    let mut resolved: Vec<ResolvedSudoCommand> = Vec::new();

    for (_plugin_type, entries) in &plugin_sudo_cmds {
        for entry in entries {
            if let Some(helper) = &entry.helper_script {
                println!("  Installing helper script '{}'...", helper.install_path);
                install_helper_script(executor, helper, privileged).await?;
                resolved.push(ResolvedSudoCommand {
                    command_path: helper.install_path.to_string(),
                    explanation: entry.explanation.clone(),
                    needs_setenv: entry.needs_setenv,
                });
            } else {
                match resolve_command_path(executor, &entry.command).await? {
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

    Ok(resolved)
}

/// Run infra plugin host-sync hooks and append their sudo commands to
/// `resolved`.
///
/// Summary lines from each plugin are printed to stdout. Failures from
/// individual plugins are logged at `warn` level and do not abort the sync.
async fn run_infra_sync(
    db: &DatabaseConnection,
    executor: &SshRemoteExecutor,
    host: &Model,
    tenant_id: Option<uuid::Uuid>,
    resolved: &mut Vec<ResolvedSudoCommand>,
) -> Result<()> {
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
            match lifecycle.on_host_synced(&infra_ctx, executor, host.id).await {
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
                    tracing::warn!(error = %e, plugin = %plugin.plugin_type_id(), "infra plugin sync failed");
                }
            }
        }
    }

    Ok(())
}

/// Determine the sudoers content from resolved commands and `allow_all` flag.
fn build_sudoers_content(
    resolved: Vec<ResolvedSudoCommand>,
    allow_all: bool,
) -> Option<SudoersContent> {
    if !resolved.is_empty() {
        Some(SudoersContent::SpecificCommands(resolved))
    } else if allow_all {
        println!("  No plugin commands resolved; using NOPASSWD: ALL (--allow-all).");
        Some(SudoersContent::AllCommands)
    } else {
        println!("  No plugin-specific commands found for this host; nothing to write.");
        println!("  Install supported tools or re-run with --allow-all.");
        None
    }
}

/// Write the sudoers file (or print the dry-run preview) and update DB state.
async fn write_or_preview_sudoers(
    executor: &SshRemoteExecutor,
    db: &DatabaseConnection,
    host: &Model,
    sudoers_content: Option<&SudoersContent>,
    privileged: bool,
    agent_is_root: bool,
    dry_run: bool,
) -> Result<()> {
    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{}", host.username);

    if !dry_run {
        if let Some(content) = sudoers_content {
            let preview_text = sudoers::generate_sudoers_content(&host.username, content);
            println!("Writing {sudoers_file}...");
            write_sudoers_file(executor, &host.username, content, privileged).await?;
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
    } else if let Some(content) = sudoers_content {
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

    Ok(())
}

/// Run the `sync` command.
pub(crate) async fn run(args: &SyncArgs, db: &DatabaseConnection) -> Result<()> {
    // 1. Load SSH host from DB (supports name, UUID, and SSH address).
    let (host, url_username) = resolve_host(db, &args.name_or_id).await?;

    // 2. Determine connection username and authentication method.
    //    `key_pem` owns the PEM string when we borrow from the stored host key.
    //    Declared here so its lifetime covers the `auth` borrow below.
    let key_pem: String = host.private_key.expose_secret().to_string();
    let (connect_username, auth, auth_override) = resolve_cli_auth(
        &host,
        url_username.as_deref(),
        args.auth_password.as_deref(),
        args.auth_private_key_pem.as_deref(),
        args.use_ssh_agent,
        &key_pem,
    )?;

    // 3. Establish SSH session (strict host key checking — no TOFU allowed).
    let session = connect_ssh(&host, connect_username, &auth).await?;
    let executor = SshRemoteExecutor::new(Arc::clone(&session));

    // 4. Detect sudo state and persist it.
    let (is_root, sudo_available) =
        detect_and_persist_sudo(&executor, db, &host, auth_override).await?;

    let agent_is_root = if auth_override { false } else { is_root };

    // 5. Permission check.
    if !is_root && !sudo_available {
        bail!(Error::InvalidInput(format!(
            "sudo is not available for user '{}' on '{}'. Cannot sync. \
             Either connect as root or ensure the user has passwordless sudo first.",
            connect_username, host.hostname
        )));
    }

    let privileged = !is_root;

    // Configure docker group membership when Docker is installed on the host.
    println!("Configuring docker group membership...");
    ensure_docker_group_membership(&executor, &host.username, privileged).await?;

    // 6. Collect plugin commands + resolve paths.
    let mut resolved = collect_plugin_sudo_commands(&session, &executor, privileged).await?;

    // 7. Run infra plugin sync hooks (appends additional sudo commands).
    run_infra_sync(db, &executor, &host, args.tenant_id, &mut resolved).await?;

    // 8. Build sudoers content and write (or preview for dry-run).
    let sudoers_content = build_sudoers_content(resolved, args.allow_all);
    write_or_preview_sudoers(
        &executor,
        db,
        &host,
        sudoers_content.as_ref(),
        privileged,
        agent_is_root,
        args.dry_run,
    )
    .await?;

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
