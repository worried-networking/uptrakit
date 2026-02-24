use std::path::{Path, PathBuf};

use rootcause::prelude::*;
use uptrakit_command::SudoPolicy;
use uptrakit_crypto::EncryptedString;

use crate::cli::HostCommands;
use crate::commands::bootstrap::{self, BootstrapParams};
use crate::commands::update_sudoers::{self, UpdateSudoersArgs};
use crate::error::{Error, Result};
use crate::host_ops::{self, AddHostParams, HostUpdates};
use crate::ssh_config;
use crate::ssh_key;

/// Dispatch a host subcommand.
pub async fn run(state_dir: &Path, command: HostCommands) -> Result<()> {
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
        HostCommands::Bootstrap {
            target,
            name,
            auth_password,
            auth_private_key_file,
            target_username,
            target_private_key_file,
            host_key_fingerprint,
            strict_host_key_checking,
            allow_all,
        } => {
            // The bootstrap command manages its own DB connection, so we
            // drop the one opened above and delegate entirely.
            drop(db);
            let cli_params = BootstrapCliParams {
                state_dir,
                target,
                name,
                auth_password,
                auth_private_key_file: auth_private_key_file.as_deref(),
                target_username,
                target_private_key_file: target_private_key_file.as_deref(),
                host_key_fingerprint,
                strict_host_key_checking,
                allow_all,
            };
            run_bootstrap(cli_params).await
        }
        HostCommands::UpdateSudoers {
            name_or_id,
            allow_all,
            dry_run,
        } => {
            let args = UpdateSudoersArgs {
                name_or_id,
                allow_all,
                dry_run,
            };
            update_sudoers::run(&args, &db).await
        }
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
    let encrypted_key = EncryptedString::new(pem)
        .map_err(|e| report!(Error::Crypto(format!("failed to encrypt private key: {e}"))))?;

    let host = host_ops::add_host(
        db,
        AddHostParams {
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
    /// Validated sudo policy string, if provided.
    sudo_policy: Option<String>,
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

/// Encapsulates bootstrap CLI parameters to avoid too many arguments.
struct BootstrapCliParams<'a> {
    state_dir: &'a Path,
    target: crate::ssh_target::SshTarget,
    name: Option<String>,
    /// `None` = not passed, `Some(None)` = prompt, `Some(Some(pw))` = inline.
    auth_password: Option<Option<String>>,
    auth_private_key_file: Option<&'a Path>,
    target_username: Option<String>,
    target_private_key_file: Option<&'a Path>,
    host_key_fingerprint: Option<String>,
    strict_host_key_checking: bool,
    allow_all: bool,
}

async fn run_bootstrap(p: BootstrapCliParams<'_>) -> Result<()> {
    if p.strict_host_key_checking && p.host_key_fingerprint.is_none() {
        bail!(Error::InvalidInput(
            "--strict-host-key-checking requires --host-key-fingerprint to be provided".to_string()
        ));
    }

    // Resolve SSH config defaults for the target hostname.
    let ssh_defaults = ssh_config::resolve_defaults(&p.target.hostname);

    // Derive final values from: target string > SSH config > system defaults.
    let auth_username = resolve_auth_username(
        p.target.username.as_deref(),
        ssh_defaults.username.as_deref(),
    )?;

    let port = p.target.port.or(ssh_defaults.port).unwrap_or(22);

    let hostname = ssh_defaults
        .hostname
        .clone()
        .unwrap_or_else(|| p.target.hostname.clone());

    // Host name defaults to the original target hostname (before HostName
    // resolution), so SSH aliases map naturally to host names.
    let name = p.name.unwrap_or_else(|| p.target.hostname.clone());

    // Log which defaults were applied.
    log_resolved_defaults(
        &p.target,
        &ssh_defaults,
        &auth_username,
        port,
        &hostname,
        &name,
    );

    let port_i32 = i32::from(port);

    // Resolve password from the dual-mode flag.
    let auth_password = match p.auth_password {
        Some(Some(pw)) => Some(pw),
        Some(None) => {
            let password = rpassword::prompt_password("SSH password: ").map_err(|e| {
                report!(Error::InvalidInput(format!("failed to read password: {e}")))
            })?;
            Some(password)
        }
        None => None,
    };

    // Detect SSH agent fallback: no password, no key file, SSH_AUTH_SOCK set.
    let use_ssh_agent = auth_password.is_none()
        && p.auth_private_key_file.is_none()
        && std::env::var_os("SSH_AUTH_SOCK").is_some();

    // Validate that at least one auth method is available.
    if auth_password.is_none() && p.auth_private_key_file.is_none() && !use_ssh_agent {
        bail!(Error::InvalidInput(
            "no authentication method available: use --auth-password, \
             --auth-private-key-file, or ensure SSH_AUTH_SOCK is set for \
             SSH agent forwarding"
                .to_string()
        ));
    }

    let auth_private_key_pem = match p.auth_private_key_file {
        Some(path) => Some(ssh_key::read_private_key(path)?),
        None => None,
    };

    // Resolve target username.
    let resolved_target = match p.target_username {
        Some(name) => name,
        None if auth_username == "root" => {
            println!("NOTE: auth username is 'root'; defaulting target username to 'uptrakit'.");
            "uptrakit".to_string()
        }
        None => auth_username.clone(),
    };

    // Read or generate target key.
    let target_private_key_pem = match p.target_private_key_file {
        Some(path) => Some(ssh_key::read_private_key(path)?),
        None => None,
    };

    let params = BootstrapParams {
        name,
        hostname,
        port: port_i32,
        auth_username,
        auth_password,
        auth_private_key_pem,
        use_ssh_agent,
        target_username: resolved_target,
        target_private_key_pem,
        host_key_fingerprint: p.host_key_fingerprint,
        strict_host_key_checking: p.strict_host_key_checking,
        allow_all: p.allow_all,
    };

    bootstrap::run_bootstrap(p.state_dir, params).await
}

/// Resolve the auth username from: target string > SSH config > system $USER.
fn resolve_auth_username(
    target_username: Option<&str>,
    ssh_config_username: Option<&str>,
) -> Result<String> {
    if let Some(user) = target_username {
        return Ok(user.to_string());
    }
    if let Some(user) = ssh_config_username {
        return Ok(user.to_string());
    }
    if let Ok(user) = std::env::var("USER")
        && !user.is_empty()
    {
        return Ok(user);
    }
    bail!(Error::InvalidInput(
        "could not determine SSH username: specify it in the target \
         (e.g. user@host), in ~/.ssh/config, or set the $USER environment variable"
            .to_string()
    ))
}

/// Log which defaults were applied during resolution.
fn log_resolved_defaults(
    target: &crate::ssh_target::SshTarget,
    ssh_defaults: &ssh_config::SshConfigDefaults,
    auth_username: &str,
    port: u16,
    hostname: &str,
    name: &str,
) {
    // Username source.
    if target.username.is_none() {
        if ssh_defaults.username.is_some() {
            tracing::info!(user = %auth_username, "using username from ~/.ssh/config");
        } else {
            tracing::info!(user = %auth_username, "using username from $USER");
        }
    }

    // Port source.
    if target.port.is_none() {
        if ssh_defaults.port.is_some() {
            tracing::info!(port = %port, "using port from ~/.ssh/config");
        } else {
            tracing::info!(port = %port, "using default port");
        }
    }

    // Hostname resolution.
    if ssh_defaults.hostname.is_some() {
        tracing::info!(
            alias = %target.hostname,
            resolved = %hostname,
            "resolved hostname from ~/.ssh/config HostName"
        );
    }

    // Name derivation.
    if target.hostname != name {
        tracing::info!(name = %name, "using explicit --name");
    } else {
        tracing::info!(name = %name, "derived host name from target hostname");
    }
}

fn format_timestamp(unix_ts: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix_ts)
        .map(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| unix_ts.to_string())
        })
        .unwrap_or_else(|_| unix_ts.to_string())
}
