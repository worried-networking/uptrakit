//! CLI dispatch for local-DB host operations (add, list, show, update, remove).
//!
//! Bootstrap and sync are handled exclusively through the extension framework.

use std::path::{Path, PathBuf};

use rootcause::prelude::*;
use uptrakit_command::SudoPolicy;
use uptrakit_crypto::EncryptedString;

use crate::cli::HostCommands;
use crate::error::{Error, Result};
use crate::host_ops::{self, AddHostParams, HostUpdates};
use crate::ssh_key;

/// Dispatch a host subcommand.
pub(crate) async fn run(state_dir: &Path, command: HostCommands) -> Result<()> {
    let db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(sea_orm::DbErr::Custom(format!(
            "failed to initialize local database: {e}"
        ))))
    })?;

    match command {
        HostCommands::Add {
            name,
            hostname,
            port,
            username,
            private_key_file,
            host_key_fingerprint,
            strict_host_key_checking,
        } => {
            let params = AddCliParams {
                name,
                hostname,
                port,
                username,
                private_key_file,
                host_key_fingerprint,
                strict_host_key_checking,
            };
            run_add(&db, params).await
        }
        HostCommands::List => run_list(&db).await,
        HostCommands::Show { name_or_id } => run_show(&db, &name_or_id).await,
        HostCommands::Update {
            name_or_id,
            name,
            hostname,
            port,
            username,
            private_key_file,
            host_key_fingerprint,
            sudo_policy,
        } => {
            // Validate sudo_policy string if provided.
            let validated_policy = match sudo_policy {
                Some(ref s) => {
                    let policy = s.parse::<SudoPolicy>().map_err(|e| {
                        report!(Error::InvalidInput(format!(
                            "invalid --sudo-policy '{}': {}",
                            s, e
                        )))
                    })?;
                    Some(policy.to_string())
                }
                None => None,
            };
            let params = UpdateParams {
                name_or_id: &name_or_id,
                name,
                hostname,
                port,
                username,
                private_key_file: private_key_file.as_deref(),
                host_key_fingerprint,
                sudo_policy: validated_policy,
            };
            run_update(&db, params).await
        }
        HostCommands::Remove { name_or_id } => run_remove(&db, &name_or_id).await,
    }
}

struct AddCliParams {
    name: String,
    hostname: String,
    port: i32,
    username: String,
    private_key_file: PathBuf,
    host_key_fingerprint: Option<String>,
    strict_host_key_checking: bool,
}

async fn run_add(db: &sea_orm::DatabaseConnection, p: AddCliParams) -> Result<()> {
    if p.strict_host_key_checking && p.host_key_fingerprint.is_none() {
        bail!(Error::InvalidInput(
            "--strict-host-key-checking requires --host-key-fingerprint to be provided".to_string()
        ));
    }

    let pem = ssh_key::read_private_key(&p.private_key_file)?;
    let key_type = ssh_key::detect_key_type(&pem)?;
    let encrypted_key = EncryptedString::new(pem, "uptrakit:ssh_hosts:private_key")
        .map_err(|e| report!(Error::Crypto(format!("failed to encrypt private key: {e}"))))?;

    let host = host_ops::add_host(
        db,
        AddHostParams {
            host_id: uuid::Uuid::now_v7(),
            name: p.name,
            hostname: p.hostname,
            port: p.port,
            username: p.username,
            encrypted_key,
            key_type,
            host_key_fingerprint: p.host_key_fingerprint,
        },
    )
    .await?;

    println!("Added host '{}'", host.name);
    println!("  ID:       {}", host.id);
    println!("  Hostname: {}:{}", host.hostname, host.port);
    println!("  Username: {}", host.username);
    println!("  Key type: {}", host.key_type);
    if let Some(ref fp) = host.host_key_fingerprint {
        println!("  Host key: {fp}");
    }

    Ok(())
}

async fn run_list(db: &sea_orm::DatabaseConnection) -> Result<()> {
    let hosts = host_ops::list_hosts(db).await?;

    if hosts.is_empty() {
        println!("No SSH hosts configured.");
        return Ok(());
    }

    println!(
        "{:<36}  {:<20}  {:<30}  {:>5}  {:<15}  {:<10}  {:<12}",
        "ID", "NAME", "HOSTNAME", "PORT", "USERNAME", "KEY TYPE", "SUDO POLICY"
    );
    for host in &hosts {
        println!(
            "{:<36}  {:<20}  {:<30}  {:>5}  {:<15}  {:<10}  {:<12}",
            host.id,
            host.name,
            host.hostname,
            host.port,
            host.username,
            host.key_type,
            host.sudo_policy
        );
    }

    Ok(())
}

async fn run_show(db: &sea_orm::DatabaseConnection, name_or_id: &str) -> Result<()> {
    let host = host_ops::find_host(db, name_or_id)
        .await?
        .ok_or_else(|| report!(Error::HostNotFound(name_or_id.to_string())))?;

    println!("ID:              {}", host.id);
    println!("Name:            {}", host.name);
    println!("Hostname:        {}", host.hostname);
    println!("Port:            {}", host.port);
    println!("Username:        {}", host.username);
    println!("Key type:        {}", host.key_type);
    println!("Private key:     ***REDACTED***");
    println!(
        "Host key:        {}",
        host.host_key_fingerprint.as_deref().unwrap_or("(not set)")
    );
    println!("Sudo policy:     {}", host.sudo_policy);
    println!(
        "Is root:         {}",
        host.is_root
            .map(|v| if v { "yes" } else { "no" })
            .unwrap_or("unknown")
    );
    println!(
        "Sudo available:  {}",
        host.sudo_available
            .map(|v| if v { "yes" } else { "no" })
            .unwrap_or("unknown")
    );
    println!(
        "PVE node name:   {}",
        host.pve_node_name.as_deref().unwrap_or("(not set)")
    );
    println!(
        "Created at:      {}",
        host.created_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| format!("{:?}", host.created_at))
    );
    println!(
        "Updated at:      {}",
        host.updated_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| format!("{:?}", host.updated_at))
    );

    Ok(())
}

/// Encapsulates update parameters to avoid too many arguments.
struct UpdateParams<'a> {
    name_or_id: &'a str,
    name: Option<String>,
    hostname: Option<String>,
    port: Option<i32>,
    username: Option<String>,
    private_key_file: Option<&'a Path>,
    host_key_fingerprint: Option<String>,
    /// Validated sudo policy string, if provided.
    sudo_policy: Option<String>,
}

async fn run_update(db: &sea_orm::DatabaseConnection, params: UpdateParams<'_>) -> Result<()> {
    let (encrypted_key, key_type) = match params.private_key_file {
        Some(path) => {
            let pem = ssh_key::read_private_key(path)?;
            let kt = ssh_key::detect_key_type(&pem)?;
            let ek = EncryptedString::new(pem, "uptrakit:ssh_hosts:private_key").map_err(|e| {
                report!(Error::Crypto(format!("failed to encrypt private key: {e}")))
            })?;
            (Some(ek), Some(kt))
        }
        None => (None, None),
    };

    let updates = HostUpdates {
        name: params.name,
        hostname: params.hostname,
        port: params.port,
        username: params.username,
        private_key: encrypted_key,
        key_type,
        host_key_fingerprint: if params.host_key_fingerprint.is_some() {
            Some(params.host_key_fingerprint)
        } else {
            None
        },
        sudo_policy: params.sudo_policy,
    };

    let host = host_ops::update_host(db, params.name_or_id, updates).await?;

    println!("Updated host '{}'", host.name);
    println!("  ID:       {}", host.id);
    println!("  Hostname: {}:{}", host.hostname, host.port);
    println!("  Username: {}", host.username);
    println!("  Key type: {}", host.key_type);
    if let Some(ref fp) = host.host_key_fingerprint {
        println!("  Host key: {fp}");
    }

    Ok(())
}

async fn run_remove(db: &sea_orm::DatabaseConnection, name_or_id: &str) -> Result<()> {
    let removed = host_ops::remove_host(db, name_or_id).await?;

    if removed {
        println!("Removed host '{name_or_id}'.");
    } else {
        bail!(Error::HostNotFound(name_or_id.to_string()));
    }

    Ok(())
}
