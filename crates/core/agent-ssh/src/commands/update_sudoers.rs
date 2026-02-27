//! `update-sudoers` command: regenerate the sudoers file for an enrolled SSH host.
//!
//! This command detects whether the agent user is root or has passwordless sudo,
//! collects required commands from all registered plugins, resolves their
//! absolute paths on the remote host, and writes a minimal sudoers drop-in file.

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
use crate::ssh_target::SshTarget;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig};
use std::time::Duration;

/// Arguments for the `update-sudoers` command.
pub struct UpdateSudoersArgs {
    /// Host name, UUID, or SSH address (`[user@]host[:port]` /
    /// `ssh://[user@]host[:port]`).
    ///
    /// When the value contains `@` or starts with `ssh://` it is parsed as an
    /// SSH address and the host is looked up by hostname (and port, if
    /// present).  Otherwise the value is treated as a host name or UUID
    /// (existing behaviour).
    pub name_or_id: String,
    /// Force `NOPASSWD: ALL` instead of specific command entries.
    pub allow_all: bool,
    /// Preview the sudoers file without writing it.
    pub dry_run: bool,
}

/// Resolve the target host from `name_or_id`, which may be a host name, UUID,
/// or an SSH address in `[user@]host[:port]` / `ssh://[user@]host[:port]` form.
async fn resolve_host(db: &DatabaseConnection, name_or_id: &str) -> Result<Model> {
    if name_or_id.contains('@') || name_or_id.starts_with("ssh://") {
        // Parse as SSH target and find by hostname (+ optional port).
        let target = name_or_id.parse::<SshTarget>().map_err(|e| {
            report!(Error::InvalidInput(format!(
                "invalid SSH address '{name_or_id}': {e}"
            )))
        })?;

        let matches =
            host_ops::find_hosts_by_hostname(db, &target.hostname, target.port).await?;

        match matches.len() {
            0 => bail!(Error::HostNotFound(name_or_id.to_string())),
            1 => Ok(matches.into_iter().next().expect("length checked above")),
            _ => bail!(Error::InvalidInput(format!(
                "multiple hosts found for '{}'; use the host name or UUID to \
                 disambiguate (see `host list`)",
                name_or_id
            ))),
        }
    } else {
        // Name or UUID lookup (existing behaviour).
        host_ops::find_host(db, name_or_id)
            .await?
            .ok_or_else(|| report!(Error::HostNotFound(name_or_id.to_string())))
    }
}

/// Run the `update-sudoers` command.
pub async fn run(args: &UpdateSudoersArgs, db: &DatabaseConnection) -> Result<()> {
    // 1. Load SSH host from DB (supports name, UUID, and SSH address).
    let host = resolve_host(db, &args.name_or_id).await?;

    // 2. Establish SSH session.
    let config = SshConnectionConfig {
        hostname: host.hostname.clone(),
        port: host.port as u16,
        connect_timeout: Duration::from_secs(30),
    };
    let private_key_pem = host.private_key.expose_secret();
    let auth = AuthMethod::PrivateKey(private_key_pem);

    println!(
        "Connecting to {}:{} as '{}'...",
        host.hostname, host.port, host.username
    );

    let (session, _fingerprint) = crate::ssh_transport::connect_and_authenticate(
        &config,
        &host.username,
        &auth,
        host.host_key_fingerprint.as_deref(),
    )
    .await?;

    // 3. Detect sudo state.
    println!("Detecting privilege context...");
    let is_root = detect_is_root(&session).await?;
    let sudo_available = if is_root {
        false
    } else {
        detect_sudo_available(&session).await?
    };

    // 4. Update DB with detected values (always refresh on this command).
    update_host_sudo_state(db, &host.id, Some(sudo_available), Some(is_root), None).await?;

    tracing::info!(is_root, sudo_available, "sudo state detected and persisted");

    // 5. Permission check.
    if !is_root && !sudo_available {
        bail!(Error::InvalidInput(format!(
            "sudo is not available for user '{}' on '{}'. Cannot update sudoers. \
             Either connect as root or ensure the user has passwordless sudo first.",
            host.username, host.hostname
        )));
    }

    let privileged = !is_root; // root needs no sudo prefix

    // 6. Collect plugin commands + resolve paths.
    let plugin_sudo_cmds = PluginRegistry::all_required_sudo_commands();
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
                });
            } else {
                match resolve_command_path(&session, &entry.command).await? {
                    Some(path) => {
                        tracing::debug!(command = %entry.command, path = %path, "resolved command path");
                        resolved.push(ResolvedSudoCommand {
                            command_path: path,
                            explanation: entry.explanation.clone(),
                        });
                    }
                    None => {
                        tracing::warn!(
                            command = %entry.command,
                            "command not found on remote host, skipping"
                        );
                        println!(
                            "  WARNING: command '{}' not found on remote host, skipping.",
                            entry.command
                        );
                    }
                }
            }
        }
    }

    // 7. Determine sudoers content.
    let sudoers_content = if !resolved.is_empty() {
        SudoersContent::SpecificCommands(resolved)
    } else if args.allow_all {
        println!("  No plugin commands resolved; using NOPASSWD: ALL (--allow-all).");
        SudoersContent::AllCommands
    } else {
        bail!(Error::InvalidInput(
            "No plugin commands could be resolved on the remote host. \
             Ensure the required tools are installed or re-run with --allow-all."
                .to_string()
        ));
    };

    // Show preview.
    let preview_text = sudoers::generate_sudoers_content(&host.username, &sudoers_content);
    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{}", host.username);

    if args.dry_run {
        println!();
        println!("Dry run — sudoers file that would be written to {sudoers_file}:");
        println!("---");
        print!("{preview_text}");
        println!("---");
        println!("(no changes made)");
        return Ok(());
    }

    // 8. Write the sudoers file.
    println!("Writing {sudoers_file}...");
    write_sudoers_file(&session, &host.username, &sudoers_content, privileged).await?;

    // 9. Update DB: sudo_available = true (since we just wrote a sudoers file).
    update_host_sudo_state(db, &host.id, Some(true), Some(is_root), None).await?;

    session.disconnect().await;

    // 10. Success summary.
    println!();
    println!("Sudoers updated for host '{}'.", host.name);
    println!("  File: {sudoers_file}");
    println!("  Content:");
    for line in preview_text.lines() {
        println!("    {line}");
    }

    Ok(())
}
