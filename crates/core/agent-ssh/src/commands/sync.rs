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

use uptrakit_command::RemoteExecutor as _;

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

    // PVE nodes require sudo access to pct/qm for guest bootstrap.
    if host.is_pve_node {
        resolved.extend(pve_sudo_commands(&executor).await);
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

/// Sync PVE-specific state and print a human-readable summary to stdout.
///
/// Thin CLI wrapper around [`sync_pve_state_inner`].
async fn sync_pve_state(
    executor: &SshRemoteExecutor,
    db: &DatabaseConnection,
    host: &Model,
    tenant_id: Option<&uuid::Uuid>,
    dry_run: bool,
) -> Result<()> {
    println!();
    println!("Syncing Proxmox VE state...");
    for line in sync_pve_state_inner(executor, db, host, tenant_id.copied(), dry_run).await? {
        println!("  {line}");
    }
    Ok(())
}

/// Core PVE sync logic shared by the CLI and extension paths.
///
/// Performs three steps and returns a list of human-readable summary lines:
///
/// 1. **Node name detection** — reads `hostname -s` and stores it so that
///    `bootstrap-proxmox-guest` can locate the correct SSH host for each guest.
///
/// 2. **Plugin config ID reconciliation** — detects a desync that arises when
///    multiple PVE nodes in the same cluster hold different `pve_plugin_config_id`
///    values (e.g. because a second plugin config was created during a bootstrap
///    where `pveum user list` failed transiently). Strategy:
///    - Get the cluster node list via `pvesh get /cluster/status`.
///    - Collect distinct config IDs held by confirmed cluster peers.
///    - One unique value → adopt it. Multiple → pick the newest UUID v7 (the
///      most-recently-created config is the one the controller actively uses for
///      discovery).
///
/// 3. **Privilege verification** — confirms the Uptrakit PVE API user still
///    holds the `PVEAuditor` role on `/`.
///
/// When `dry_run` is `true` the DB write is skipped; all other steps still run.
async fn sync_pve_state_inner(
    executor: &SshRemoteExecutor,
    db: &DatabaseConnection,
    host: &Model,
    tenant_id: Option<uuid::Uuid>,
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();

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
    let canonical_config_id: Option<String> = if let Some(ref tid) = tenant_id {
        match pve_setup::check_pve_token_exists(executor, tid).await {
            Ok(pve_setup::PveTokenStatus::OwnedByTenant(_)) => {
                let cluster_nodes = pve_setup::detect_pve_cluster_nodes(executor).await;
                if cluster_nodes.is_empty() {
                    None
                } else {
                    reconcile_pve_config(db, &host.id, &cluster_nodes).await
                }
            }
            _ => None,
        }
    } else {
        None
    };

    let config_id_to_store = canonical_config_id
        .as_ref()
        .or(host.pve_plugin_config_id.as_ref())
        .cloned();

    if let Some(ref new_id) = canonical_config_id {
        if host.pve_plugin_config_id.as_deref() != Some(new_id.as_str()) {
            lines.push(format!(
                "plugin config: corrected from {} to {new_id}",
                host.pve_plugin_config_id.as_deref().unwrap_or("(none)")
            ));
        } else {
            lines.push(format!("plugin config: OK ({new_id})"));
        }
    }

    // Persist (unless dry run).
    if !dry_run && node_name.is_some() {
        host_ops::update_host_pve_state(db, &host.id, true, config_id_to_store, node_name).await?;
    }

    // Step 3: privilege verification.
    if let Some(ref tid) = tenant_id {
        match pve_setup::verify_pve_privileges(executor, tid).await {
            Ok(()) => lines.push("privileges: OK (PVEAuditor on /)".to_string()),
            Err(e) => {
                lines.push(format!("privileges: FAILED — {e}"));
                lines.push(
                    "run bootstrap again or manually grant PVEAuditor on / to the Uptrakit user"
                        .to_string(),
                );
            }
        }
    } else {
        lines.push(
            "privilege check: skipped (tenant ID not yet available — \
             ensure the agent has connected to the controller at least once)"
                .to_string(),
        );
    }

    Ok(lines)
}

/// Collect sudoers entries for PVE-specific management tools.
///
/// `/usr/sbin/pct` and `/usr/sbin/qm` live outside the default PATH of
/// non-root users and must be invoked as `sudo /usr/sbin/pct …` by the agent.
/// This helper checks which tools are present on the remote host and returns
/// [`ResolvedSudoCommand`] entries for each one found.  Called during sync
/// when [`Model::is_pve_node`] is `true` so that the generated sudoers file
/// grants the stored agent user `NOPASSWD` access to these binaries.
async fn pve_sudo_commands(executor: &SshRemoteExecutor) -> Vec<ResolvedSudoCommand> {
    let tools = [
        (
            "/usr/sbin/pct",
            "Proxmox LXC container management (pct exec for guest bootstrap)",
        ),
        (
            "/usr/sbin/qm",
            "Proxmox QEMU VM management (qm guest exec for guest bootstrap)",
        ),
    ];

    let mut cmds = Vec::new();
    for (path, explanation) in tools {
        let exists = executor
            .exec_command(&format!("test -f {path}"))
            .await
            .map(|r| r.exit_code == 0)
            .unwrap_or(false);
        if exists {
            cmds.push(ResolvedSudoCommand {
                command_path: path.to_string(),
                explanation: explanation.to_string(),
                needs_setenv: false,
            });
        }
    }
    cmds
}

/// Determine the canonical `pve_plugin_config_id` for the cluster this host
/// belongs to, by inspecting other local PVE hosts that are confirmed cluster
/// peers.
///
/// Returns `None` when no peer has a config, or when the result is ambiguous
/// and logging a warning is sufficient (caller falls back to the stored value).
async fn reconcile_pve_config(
    db: &DatabaseConnection,
    current_host_id: &str,
    cluster_nodes: &[String],
) -> Option<String> {
    let all_pve_hosts = match host_ops::find_pve_hosts(db).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list local PVE hosts for config reconciliation");
            return None;
        }
    };

    // Collect distinct config IDs held by cluster peers (exclude current host).
    let mut peer_configs: Vec<String> = all_pve_hosts
        .iter()
        .filter(|h| h.id != current_host_id)
        .filter(|h| {
            h.pve_node_name
                .as_deref()
                .is_some_and(|n| cluster_nodes.contains(&n.to_string()))
        })
        .filter_map(|h| h.pve_plugin_config_id.clone())
        .collect();

    // Deduplicate while preserving the maximum (newest UUID v7 = newest config).
    peer_configs.sort_unstable();
    peer_configs.dedup();

    match peer_configs.len() {
        0 => None,
        1 => Some(peer_configs.remove(0)),
        _ => {
            // Multiple configs among confirmed cluster peers (split-brain from
            // a failed dedup on a previous bootstrap).  Pick the newest UUID
            // (highest sort value for v7) — that is the most recently created
            // config and therefore the one the controller is actively using for
            // discovery.
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
        &host.id,
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
                resolved.push(ResolvedSudoCommand {
                    command_path: path,
                    explanation: entry.explanation.clone(),
                    needs_setenv: entry.needs_setenv,
                });
            }
        }
    }

    let mut summary = Vec::new();

    // PVE nodes require sudo access to pct/qm for guest bootstrap.
    if host.is_pve_node {
        resolved.extend(pve_sudo_commands(&executor).await);
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
            update_host_sudo_state(db, &host.id, Some(true), Some(is_root), None)
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

    // PVE sync.
    if host.is_pve_node {
        match sync_pve_state_inner(&executor, db, &host, tenant_id, false).await {
            Ok(lines) => {
                for line in lines {
                    summary.push(format!("pve: {line}"));
                }
            }
            Err(e) => summary.push(format!("pve: sync failed ({e})")),
        }
    }

    drop(executor);
    SshSession::disconnect_shared(session).await;

    Ok(summary.join("; "))
}
