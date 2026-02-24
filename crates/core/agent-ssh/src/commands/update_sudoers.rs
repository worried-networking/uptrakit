//! `update-sudoers` command: regenerate the sudoers file for an enrolled SSH host.
//!
//! This command detects whether the agent user is root or has passwordless sudo,
//! collects required commands from all registered providers, resolves their
//! absolute paths on the remote host, and writes a minimal sudoers drop-in file.

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_provider_registry::ProviderRegistry;

use crate::commands::sudoers::{
    self, ResolvedSudoCommand, SudoersContent, detect_is_root, detect_sudo_available,
    resolve_command_path, write_sudoers_file,
};
use crate::error::{Error, Result};
use crate::host_ops::{self, update_host_sudo_state};
use crate::ssh_transport::{AuthMethod, SshConnectionConfig};
use std::time::Duration;

/// Arguments for the `update-sudoers` command.
pub struct UpdateSudoersArgs {
    /// Host name or UUID.
    pub name_or_id: String,
    /// Force `NOPASSWD: ALL` instead of specific command entries.
    pub allow_all: bool,
    /// Preview the sudoers file without writing it.
    pub dry_run: bool,
}

/// Run the `update-sudoers` command.
pub async fn run(args: &UpdateSudoersArgs, db: &DatabaseConnection) -> Result<()> {
    // 1. Load SSH host from DB.
    let host = host_ops::find_host(db, &args.name_or_id)
        .await?
        .ok_or_else(|| report!(Error::HostNotFound(args.name_or_id.clone())))?;

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

    tracing::info!(
        is_root,
        sudo_available,
        "sudo state detected and persisted"
    );

    // 5. Permission check.
    if !is_root && !sudo_available {
        bail!(Error::InvalidInput(format!(
            "sudo is not available for user '{}' on '{}'. Cannot update sudoers. \
             Either connect as root or ensure the user has passwordless sudo first.",
            host.username, host.hostname
        )));
    }

    let privileged = !is_root; // root needs no sudo prefix

    // 6. Collect provider commands + resolve paths.
    let provider_sudo_cmds = ProviderRegistry::all_required_sudo_commands();
    let mut resolved: Vec<ResolvedSudoCommand> = Vec::new();

    for (_provider_type, entries) in &provider_sudo_cmds {
        for entry in entries {
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

    // 7. Determine sudoers content.
    let sudoers_content = if !resolved.is_empty() {
        SudoersContent::SpecificCommands(resolved)
    } else if args.allow_all {
        println!("  No provider commands resolved; using NOPASSWD: ALL (--allow-all).");
        SudoersContent::AllCommands
    } else {
        bail!(Error::InvalidInput(
            "No provider commands could be resolved on the remote host. \
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
