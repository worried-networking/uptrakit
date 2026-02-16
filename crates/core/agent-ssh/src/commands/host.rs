use std::path::Path;

use rootcause::prelude::*;
use uptrakit_shared_db::crypto::EncryptedString;

use crate::cli::HostCommands;
use crate::commands::bootstrap::{self, BootstrapParams};
use crate::error::{Error, Result};
use crate::host_ops::{self, AddHostParams, HostUpdates};
use crate::ssh_key;

/// Dispatch a host subcommand.
pub async fn run(state_dir: &Path, command: HostCommands) -> Result<()> {
    let db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(format!(
            "failed to initialize local database: {e}"
        )))
    })?;

    match command {
        HostCommands::Add {
            name,
            hostname,
            port,
            username,
            private_key_file,
            host_key_fingerprint,
        } => {
            run_add(
                &db,
                name,
                hostname,
                port,
                username,
                &private_key_file,
                host_key_fingerprint,
            )
            .await
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
        } => {
            let params = UpdateParams {
                name_or_id: &name_or_id,
                name,
                hostname,
                port,
                username,
                private_key_file: private_key_file.as_deref(),
                host_key_fingerprint,
            };
            run_update(&db, params).await
        }
        HostCommands::Remove { name_or_id } => run_remove(&db, &name_or_id).await,
        HostCommands::Bootstrap {
            name,
            hostname,
            auth_username,
            auth_password,
            auth_private_key_file,
            target_username,
            target_private_key_file,
            port,
            host_key_fingerprint,
        } => {
            // The bootstrap command manages its own DB connection, so we
            // drop the one opened above and delegate entirely.
            drop(db);
            let cli_params = BootstrapCliParams {
                state_dir,
                name,
                hostname,
                auth_username,
                auth_password_flag: auth_password,
                auth_private_key_file: auth_private_key_file.as_deref(),
                target_username,
                target_private_key_file: target_private_key_file.as_deref(),
                port,
                host_key_fingerprint,
            };
            run_bootstrap(cli_params).await
        }
    }
}

async fn run_add(
    db: &sea_orm::DatabaseConnection,
    name: String,
    hostname: String,
    port: i32,
    username: String,
    private_key_file: &Path,
    host_key_fingerprint: Option<String>,
) -> Result<()> {
    let pem = ssh_key::read_private_key(private_key_file)?;
    let key_type = ssh_key::detect_key_type(&pem)?;
    let encrypted_key = EncryptedString::new(pem)
        .map_err(|e| report!(Error::Crypto(format!("failed to encrypt private key: {e}"))))?;

    let host = host_ops::add_host(
        db,
        AddHostParams {
            name,
            hostname,
            port,
            username,
            encrypted_key,
            key_type,
            host_key_fingerprint,
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
        "{:<36}  {:<20}  {:<30}  {:>5}  {:<15}  {:<10}",
        "ID", "NAME", "HOSTNAME", "PORT", "USERNAME", "KEY TYPE"
    );
    for host in &hosts {
        println!(
            "{:<36}  {:<20}  {:<30}  {:>5}  {:<15}  {:<10}",
            host.id, host.name, host.hostname, host.port, host.username, host.key_type
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
    println!("Created at:      {}", format_timestamp(host.created_at));
    println!("Updated at:      {}", format_timestamp(host.updated_at));

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
}

async fn run_update(db: &sea_orm::DatabaseConnection, params: UpdateParams<'_>) -> Result<()> {
    let (encrypted_key, key_type) = match params.private_key_file {
        Some(path) => {
            let pem = ssh_key::read_private_key(path)?;
            let kt = ssh_key::detect_key_type(&pem)?;
            let ek = EncryptedString::new(pem).map_err(|e| {
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

/// Encapsulates bootstrap CLI parameters to avoid too many arguments.
struct BootstrapCliParams<'a> {
    state_dir: &'a Path,
    name: String,
    hostname: String,
    auth_username: String,
    auth_password_flag: bool,
    auth_private_key_file: Option<&'a Path>,
    target_username: Option<String>,
    target_private_key_file: Option<&'a Path>,
    port: i32,
    host_key_fingerprint: Option<String>,
}

async fn run_bootstrap(p: BootstrapCliParams<'_>) -> Result<()> {
    // Validate that at least one auth method is provided.
    if !p.auth_password_flag && p.auth_private_key_file.is_none() {
        bail!(Error::InvalidInput(
            "at least one of --auth-password or --auth-private-key-file is required".to_string()
        ));
    }

    // Read auth credentials.
    let auth_password = if p.auth_password_flag {
        let password = rpassword::prompt_password("SSH password: ")
            .map_err(|e| report!(Error::InvalidInput(format!("failed to read password: {e}"))))?;
        Some(password)
    } else {
        None
    };

    let auth_private_key_pem = match p.auth_private_key_file {
        Some(path) => Some(ssh_key::read_private_key(path)?),
        None => None,
    };

    // Resolve target username.
    let resolved_target = p.target_username.unwrap_or_else(|| p.auth_username.clone());

    // Read or generate target key.
    let target_private_key_pem = match p.target_private_key_file {
        Some(path) => Some(ssh_key::read_private_key(path)?),
        None => None,
    };

    let params = BootstrapParams {
        name: p.name,
        hostname: p.hostname,
        port: p.port,
        auth_username: p.auth_username,
        auth_password,
        auth_private_key_pem,
        target_username: resolved_target,
        target_private_key_pem,
        host_key_fingerprint: p.host_key_fingerprint,
    };

    bootstrap::run_bootstrap(p.state_dir, params).await
}

fn format_timestamp(unix_ts: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix_ts)
        .map(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| unix_ts.to_string())
        })
        .unwrap_or_else(|_| unix_ts.to_string())
}
