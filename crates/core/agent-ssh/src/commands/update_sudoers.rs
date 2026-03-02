//! `update-sudoers` command: regenerate the sudoers file for an enrolled SSH host.
//!
//! This command detects whether the agent user is root or has passwordless sudo,
//! collects required commands from compatible plugins (running host-compatibility
//! checks over SSH), resolves their absolute paths on the remote host, and writes
//! a minimal sudoers drop-in file.

use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_registry::PluginRegistry;

use crate::commands::sudoers::{
    self, ResolvedSudoCommand, SudoersContent, detect_is_root, detect_sudo_available,
    install_helper_script, resolve_command_path, write_sudoers_file,
};
use crate::db::entity::ssh_host::Model;
use crate::error::{Error, Result};
use crate::host_ops::{self, update_host_sudo_state};
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_target::SshTarget;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig, SshSession};

/// Arguments for the `update-sudoers` command.
pub struct UpdateSudoersArgs {
    /// Host name, UUID, or SSH address (`[user@]host[:port]` /
    /// `ssh://[user@]host[:port]`).
    ///
    /// When the value contains `@` or starts with `ssh://` it is parsed as an
    /// SSH address and the host is looked up by hostname (and port, if
    /// present).  Otherwise the value is treated as a host name or UUID
    /// (existing behaviour).
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
    /// Preview the sudoers file without writing it.
    pub dry_run: bool,
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
        // Name or UUID lookup (existing behaviour).
        let host = host_ops::find_host(db, name_or_id)
            .await?
            .ok_or_else(|| report!(Error::HostNotFound(name_or_id.to_string())))?;
        Ok((host, None))
    }
}

/// Run the `update-sudoers` command.
pub async fn run(args: &UpdateSudoersArgs, db: &DatabaseConnection) -> Result<()> {
    // 1. Load SSH host from DB (supports name, UUID, and SSH address).
    let (host, url_username) = resolve_host(db, &args.name_or_id).await?;

    // 2. Determine connection username and authentication method.
    //
    // When the SSH address includes a username that differs from the stored
    // host username (e.g. `root@myserver`) we connect as that user using the
    // caller-supplied credentials or the local SSH agent.  Otherwise we use
    // the stored private key for the host's agent user.
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
    //
    // The host is already registered, so the fingerprint must be stored.
    // Accepting an unknown host key here would silently undermine the security
    // of the sudoers write that follows.
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

    // Wrap in Arc so the session can be shared with the plugin executor used
    // for host-compatibility checks without copying any state.
    let session = Arc::new(session);

    // 4. Detect sudo state for the *connection* user.
    println!("Detecting privilege context...");
    let is_root = detect_is_root(&session).await?;
    let sudo_available = if is_root {
        false
    } else {
        detect_sudo_available(&session).await?
    };

    // When an auth override is active (e.g. `root@host`), the detection
    // above reflects the override user's privileges, not the host's
    // configured agent user.  The agent user is definitively non-root —
    // otherwise no override would be needed.  Store the corrected value
    // so that `SudoAwareCommandExecutor` will correctly prepend `sudo`
    // for privileged commands during subsequent agent operations.
    let agent_is_root = if auth_override { false } else { is_root };

    // 5. Update DB with detected values (always refresh on this command).
    //
    // When auth_override: sudo_available was detected for the override
    // user and is unreliable for the agent user.  Pass `None` to
    // preserve whatever the DB already has; step 10 will set it to
    // `true` after the sudoers file is written successfully.
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

    // 6. Permission check.
    if !is_root && !sudo_available {
        bail!(Error::InvalidInput(format!(
            "sudo is not available for user '{}' on '{}'. Cannot update sudoers. \
             Either connect as root or ensure the user has passwordless sudo first.",
            connect_username, host.hostname
        )));
    }

    let privileged = !is_root; // root needs no sudo prefix

    // 7. Collect plugin commands + resolve paths.
    //
    // Run host-compatibility checks via an SSH executor so that only the
    // commands applicable to *this* host are included.  For example, the
    // Proxmox Helper Scripts plugin's helper scripts are only installed on
    // Proxmox VE nodes; on Flatcar Linux (read-only /usr/local/bin) they
    // would fail.  The SSH executor is dropped after the call so the session
    // Arc refcount returns to 1 before we call `disconnect_shared`.
    let ssh_executor = Arc::new(SshCommandExecutor::new(Arc::clone(&session)))
        as Arc<dyn uptrakit_command::CommandExecutor>;
    let plugin_sudo_cmds =
        PluginRegistry::compatible_sudo_commands_for_host(ssh_executor).await;
    let mut resolved: Vec<ResolvedSudoCommand> = Vec::new();

    for (_plugin_type, entries) in &plugin_sudo_cmds {
        for entry in entries {
            if let Some(helper) = &entry.helper_script {
                // Install the helper script then use its known path directly
                // as the sudoers command — no `command -v` resolution needed.
                println!("  Installing helper script '{}'...", helper.install_path);
                install_helper_script(&session, helper, privileged).await?;
                resolved.push(ResolvedSudoCommand {
                    command_path: helper.install_path.to_string(),
                    explanation: entry.explanation.clone(),
                    needs_setenv: entry.needs_setenv,
                });
            } else {
                match resolve_command_path(&session, &entry.command).await? {
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

    // 8. Determine sudoers content.
    //
    // When no commands were resolved and --allow-all was not requested, skip
    // writing entirely rather than failing — the host may simply not have any
    // of the supported package managers or tools installed.
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

    // Handle dry run.
    if args.dry_run {
        println!();
        if let Some(ref content) = sudoers_content {
            let preview_text = sudoers::generate_sudoers_content(&host.username, content);
            println!("Dry run — sudoers file that would be written to {sudoers_file}:");
            println!("---");
            print!("{preview_text}");
            println!("---");
        } else {
            println!("Dry run — no sudoers file would be written (no commands resolved).");
        }
        println!("(no changes made)");
        return Ok(());
    }

    if let Some(ref content) = sudoers_content {
        let preview_text = sudoers::generate_sudoers_content(&host.username, content);

        // 9. Write the sudoers file.
        println!("Writing {sudoers_file}...");
        write_sudoers_file(&session, &host.username, content, privileged).await?;

        // 10. Update DB: sudo_available = true (since we just wrote a sudoers file).
        update_host_sudo_state(db, &host.id, Some(true), Some(agent_is_root), None).await?;

        SshSession::disconnect_shared(session).await;

        // 11. Success summary.
        println!();
        println!("Sudoers updated for host '{}'.", host.name);
        println!("  File: {sudoers_file}");
        println!("  Content:");
        for line in preview_text.lines() {
            println!("    {line}");
        }
    } else {
        SshSession::disconnect_shared(session).await;

        println!();
        println!("No sudoers changes for host '{}' — no compatible plugin commands found on this host.", host.name);
    }

    Ok(())
}
