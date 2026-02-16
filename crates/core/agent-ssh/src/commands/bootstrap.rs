//! Bootstrap command: automates remote host setup (user creation, SSH key
//! deployment, sudoers configuration) and saves the host entry to the local
//! database.

use std::path::Path;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_shared_db::crypto::EncryptedString;

use crate::error::{Error, Result};
use crate::host_ops::{self, AddHostParams};
use crate::ssh_key;
use crate::ssh_transport::{self, AuthMethod, SshConnectionConfig, SshSession};

/// Maximum length for POSIX usernames.
const MAX_USERNAME_LEN: usize = 32;

/// Default SSH connect timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

// ── Bootstrap parameters ─────────────────────────────────────────────

/// Parameters for the bootstrap workflow, mirroring CLI args.
pub struct BootstrapParams {
    pub name: String,
    pub hostname: String,
    pub port: i32,
    pub auth_username: String,
    pub auth_password: Option<String>,
    pub auth_private_key_pem: Option<String>,
    /// Use the local SSH agent for authentication (detected from `SSH_AUTH_SOCK`).
    pub use_ssh_agent: bool,
    pub target_username: String,
    pub target_private_key_pem: Option<String>,
    pub host_key_fingerprint: Option<String>,
}

// ── Main orchestrator ────────────────────────────────────────────────

/// Run the full bootstrap workflow.
pub async fn run_bootstrap(state_dir: &Path, params: BootstrapParams) -> Result<()> {
    // 1. VALIDATE INPUTS
    validate_posix_username(&params.auth_username)?;
    validate_posix_username(&params.target_username)?;

    if params.auth_password.is_none()
        && params.auth_private_key_pem.is_none()
        && !params.use_ssh_agent
    {
        bail!(Error::InvalidInput(
            "no authentication method available: use --auth-password, \
             --auth-private-key-file, or ensure SSH_AUTH_SOCK is set for \
             SSH agent forwarding"
                .to_string()
        ));
    }

    // Fail fast: check host name is not in DB.
    let db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(format!(
            "failed to initialize local database: {e}"
        )))
    })?;
    let existing = host_ops::find_host(&db, &params.name).await?;
    if existing.is_some() {
        bail!(Error::HostNameConflict(params.name.clone()));
    }

    // 2. PREPARE KEY MATERIAL
    let (target_private_pem, target_public_openssh, generated_key) =
        match &params.target_private_key_pem {
            Some(pem) => {
                let pubkey = ssh_key::extract_public_key_openssh(pem)?;
                (pem.clone(), pubkey, false)
            }
            None => {
                println!("Generating Ed25519 keypair for target user...");
                let (priv_pem, pub_openssh) = ssh_key::generate_ed25519_keypair()?;
                (priv_pem, pub_openssh, true)
            }
        };

    let key_type = ssh_key::detect_key_type(&target_private_pem)?;

    // 3. CONNECT & AUTHENTICATE (as auth_username)
    let auth = match (
        &params.auth_password,
        &params.auth_private_key_pem,
        params.use_ssh_agent,
    ) {
        (Some(password), _, _) => AuthMethod::Password(password),
        (_, Some(pem), _) => AuthMethod::PrivateKey(pem),
        (_, _, true) => AuthMethod::Agent,
        _ => bail!(Error::InvalidInput(
            "no authentication method available".to_string()
        )),
    };

    let port = u16::try_from(params.port).map_err(|_| {
        report!(Error::InvalidInput(format!(
            "port must be 0-65535, got {}",
            params.port
        )))
    })?;

    let config = SshConnectionConfig {
        hostname: params.hostname.clone(),
        port,
        connect_timeout: CONNECT_TIMEOUT,
    };

    println!(
        "Connecting to {}:{} as '{}'...",
        params.hostname, port, params.auth_username
    );

    let (session, observed_fp) = ssh_transport::connect_and_authenticate(
        &config,
        &params.auth_username,
        &auth,
        params.host_key_fingerprint.as_deref(),
    )
    .await?;

    if params.host_key_fingerprint.is_none() {
        println!("Host key (TOFU): {observed_fp}");
    }

    // 4. Detect if auth user is root
    let result = session.exec_command("id -u").await?;
    let is_root = result.stdout.trim() == "0";
    let use_sudo = !is_root;

    if use_sudo {
        // Verify auth user has sudo access.
        let sudo_check = session.exec_command("sudo -n true").await?;
        if sudo_check.exit_code != 0 {
            bail!(Error::SshCommand(format!(
                "auth user '{}' does not have passwordless sudo access (exit code {}). \
                 Bootstrap requires sudo privileges on the remote host.",
                params.auth_username, sudo_check.exit_code
            )));
        }
    }

    // 5. REMOTE SETUP
    let target_same_as_auth = params.target_username == params.auth_username;

    if !target_same_as_auth {
        // Check if target user exists.
        let cmd = cmd_check_user_exists(&params.target_username, use_sudo);
        let user_check = session.exec_command(&cmd).await?;

        if user_check.exit_code != 0 {
            println!("Creating user '{}'...", params.target_username);
            let cmd = cmd_create_user(&params.target_username, use_sudo);
            let create_result = session.exec_command(&cmd).await?;
            if create_result.exit_code != 0 {
                bail!(Error::SshCommand(format!(
                    "failed to create user '{}': {}",
                    params.target_username,
                    create_result.stderr.trim()
                )));
            }
        } else {
            println!(
                "User '{}' already exists, skipping creation.",
                params.target_username
            );
        }
    }

    // Detect home directory.
    let home_cmd = cmd_detect_home(&params.target_username, use_sudo);
    let home_result = session.exec_command(&home_cmd).await?;
    let home_dir = home_result.stdout.trim().to_string();
    if home_dir.is_empty() {
        bail!(Error::SshCommand(format!(
            "could not determine home directory for user '{}'",
            params.target_username
        )));
    }

    // Deploy authorized_keys.
    println!("Deploying SSH public key...");
    let ak_cmd = cmd_setup_authorized_keys(
        &home_dir,
        &target_public_openssh,
        &params.target_username,
        use_sudo,
    );
    let ak_result = session.exec_command(&ak_cmd).await?;
    if ak_result.exit_code != 0 {
        bail!(Error::SshCommand(format!(
            "failed to deploy authorized_keys: {}",
            ak_result.stderr.trim()
        )));
    }

    // Set up sudoers.
    println!("Configuring sudoers...");
    let sudoers_cmd = cmd_setup_sudoers(&params.target_username, use_sudo);
    let sudoers_result = session.exec_command(&sudoers_cmd).await?;
    if sudoers_result.exit_code != 0 {
        bail!(Error::SshCommand(format!(
            "failed to configure sudoers: {}",
            sudoers_result.stderr.trim()
        )));
    }

    // Validate sudoers.
    let validate_cmd = cmd_validate_sudoers(&params.target_username, use_sudo);
    let validate_result = session.exec_command(&validate_cmd).await?;
    if validate_result.exit_code != 0 {
        bail!(Error::SshCommand(format!(
            "sudoers validation failed (visudo -cf): {}",
            validate_result.stderr.trim()
        )));
    }

    // 6. DISCONNECT auth session.
    session.disconnect().await;

    // 7. VERIFY — reconnect as target_username with target key.
    println!("Verifying connectivity as '{}'...", params.target_username);

    let verify_config = SshConnectionConfig {
        hostname: params.hostname.clone(),
        port,
        connect_timeout: CONNECT_TIMEOUT,
    };

    let (verify_session, _) = ssh_transport::connect_and_authenticate(
        &verify_config,
        &params.target_username,
        &AuthMethod::PrivateKey(&target_private_pem),
        Some(&observed_fp),
    )
    .await
    .map_err(|e| {
        report!(Error::BootstrapVerification(format!(
            "failed to connect as target user '{}': {e}. \
             The remote host has been partially configured (user created, \
             key deployed, sudoers written). Manual cleanup may be required.",
            params.target_username
        )))
    })?;

    verify_remote(&verify_session, &params.target_username).await?;
    verify_session.disconnect().await;

    // 8. SAVE TO DATABASE
    save_host(&db, &params, &target_private_pem, key_type, &observed_fp).await?;

    // 9. OUTPUT
    println!();
    println!("Bootstrap complete for host '{}'.", params.name);
    println!("  Hostname: {}:{}", params.hostname, params.port);
    println!("  Target user: {}", params.target_username);
    println!("  Key type: {key_type}");
    println!("  Host key: {observed_fp}");

    if generated_key {
        println!();
        println!(
            "NOTE: The Ed25519 private key was generated in memory and is \
             stored only in the encrypted local database. No key file was \
             written to disk."
        );
    }

    println!();
    println!(
        "WARNING: Sudoers grants NOPASSWD: ALL to user '{}'. \
         Review /etc/sudoers.d/uptrakit-{} on the remote host and \
         restrict commands as needed.",
        params.target_username, params.target_username
    );

    Ok(())
}

// ── Verification ─────────────────────────────────────────────────────

async fn verify_remote(session: &SshSession, target_username: &str) -> Result<()> {
    // Verify whoami.
    let whoami = session.exec_command("whoami").await?;
    let actual_user = whoami.stdout.trim();
    if actual_user != target_username {
        bail!(Error::BootstrapVerification(format!(
            "whoami returned '{actual_user}', expected '{target_username}'. \
             The remote host has been partially configured."
        )));
    }

    // Verify sudo.
    let sudo_check = session.exec_command("sudo -n true").await?;
    if sudo_check.exit_code != 0 {
        bail!(Error::BootstrapVerification(format!(
            "sudo -n true failed (exit code {}). Sudoers may not be \
             configured correctly. The remote host has been partially configured.",
            sudo_check.exit_code
        )));
    }

    Ok(())
}

// ── Database save ────────────────────────────────────────────────────

async fn save_host(
    db: &DatabaseConnection,
    params: &BootstrapParams,
    private_pem: &str,
    key_type: crate::db::entity::ssh_host::SshKeyType,
    fingerprint: &str,
) -> Result<()> {
    let encrypted_key = EncryptedString::new(private_pem.to_string())
        .map_err(|e| report!(Error::Crypto(format!("failed to encrypt private key: {e}"))))?;

    host_ops::add_host(
        db,
        AddHostParams {
            name: params.name.clone(),
            hostname: params.hostname.clone(),
            port: params.port,
            username: params.target_username.clone(),
            encrypted_key,
            key_type,
            host_key_fingerprint: Some(fingerprint.to_string()),
        },
    )
    .await?;

    Ok(())
}

// ── Input validation ─────────────────────────────────────────────────

/// Validate that a string is a valid POSIX username.
///
/// Rules: `[a-z_][a-z0-9_-]*`, max 32 characters.
fn validate_posix_username(username: &str) -> Result<()> {
    if username.is_empty() {
        bail!(Error::InvalidInput(
            "username must not be empty".to_string()
        ));
    }
    if username.len() > MAX_USERNAME_LEN {
        bail!(Error::InvalidInput(format!(
            "username '{}' exceeds maximum length of {MAX_USERNAME_LEN} characters",
            username
        )));
    }

    let mut chars = username.chars();
    let first = chars.next().expect("non-empty validated above");
    if !first.is_ascii_lowercase() && first != '_' {
        bail!(Error::InvalidInput(format!(
            "username '{username}' must start with a lowercase letter or underscore"
        )));
    }
    for ch in chars {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' && ch != '-' {
            bail!(Error::InvalidInput(format!(
                "username '{username}' contains invalid character '{ch}' \
                 (allowed: a-z, 0-9, _, -)"
            )));
        }
    }

    Ok(())
}

// ── Remote command builders ──────────────────────────────────────────

fn cmd_check_user_exists(username: &str, use_sudo: bool) -> String {
    let escaped = uptrakit_command::shell_escape(username);
    if use_sudo {
        format!("sudo id -u {escaped}")
    } else {
        format!("id -u {escaped}")
    }
}

fn cmd_create_user(username: &str, use_sudo: bool) -> String {
    let escaped = uptrakit_command::shell_escape(username);
    if use_sudo {
        format!("sudo useradd --create-home --shell /bin/bash {escaped}")
    } else {
        format!("useradd --create-home --shell /bin/bash {escaped}")
    }
}

fn cmd_detect_home(username: &str, use_sudo: bool) -> String {
    let escaped = uptrakit_command::shell_escape(username);
    if use_sudo {
        format!("sudo getent passwd {escaped} | cut -d: -f6")
    } else {
        format!("getent passwd {escaped} | cut -d: -f6")
    }
}

fn cmd_setup_authorized_keys(home: &str, pubkey: &str, owner: &str, use_sudo: bool) -> String {
    let escaped_home = uptrakit_command::shell_escape(home);
    let escaped_pubkey = uptrakit_command::shell_escape(pubkey);
    let escaped_owner = uptrakit_command::shell_escape(owner);
    let ssh_dir = format!("{home}/.ssh");
    let escaped_ssh_dir = uptrakit_command::shell_escape(&ssh_dir);
    let ak_path = format!("{home}/.ssh/authorized_keys");
    let escaped_ak_path = uptrakit_command::shell_escape(&ak_path);

    let sudo_prefix = if use_sudo { "sudo " } else { "" };

    format!(
        "{sudo_prefix}mkdir -p {escaped_ssh_dir} && \
         {sudo_prefix}chmod 700 {escaped_ssh_dir} && \
         echo {escaped_pubkey} | {sudo_prefix}tee -a {escaped_ak_path} > /dev/null && \
         {sudo_prefix}chmod 600 {escaped_ak_path} && \
         {sudo_prefix}chown -R {escaped_owner}:{escaped_owner} {escaped_home}/.ssh"
    )
}

fn cmd_setup_sudoers(target: &str, use_sudo: bool) -> String {
    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{target}");
    let escaped_sudoers = uptrakit_command::shell_escape(&sudoers_file);
    let content = format!("{target} ALL=(ALL) NOPASSWD: ALL");
    let escaped_content = uptrakit_command::shell_escape(&content);

    let sudo_prefix = if use_sudo { "sudo " } else { "" };

    format!(
        "echo {escaped_content} | {sudo_prefix}tee {escaped_sudoers} > /dev/null && \
         {sudo_prefix}chmod 440 {escaped_sudoers}"
    )
}

fn cmd_validate_sudoers(target: &str, use_sudo: bool) -> String {
    let sudoers_file = format!("/etc/sudoers.d/uptrakit-{target}");
    let escaped_sudoers = uptrakit_command::shell_escape(&sudoers_file);

    if use_sudo {
        format!("sudo visudo -cf {escaped_sudoers}")
    } else {
        format!("visudo -cf {escaped_sudoers}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Username validation tests ────────────────────────────────────

    #[test]
    fn valid_usernames() {
        for name in ["root", "ubuntu", "deploy_user", "a-b-c", "_svc"] {
            assert!(
                validate_posix_username(name).is_ok(),
                "expected valid: {name}"
            );
        }
    }

    #[test]
    fn invalid_username_empty() {
        assert!(validate_posix_username("").is_err());
    }

    #[test]
    fn invalid_username_too_long() {
        let long = "a".repeat(MAX_USERNAME_LEN + 1);
        assert!(validate_posix_username(&long).is_err());
    }

    #[test]
    fn invalid_username_starts_with_digit() {
        assert!(validate_posix_username("1user").is_err());
    }

    #[test]
    fn invalid_username_uppercase() {
        assert!(validate_posix_username("Root").is_err());
    }

    #[test]
    fn invalid_username_special_chars() {
        assert!(validate_posix_username("user@host").is_err());
        assert!(validate_posix_username("user.name").is_err());
    }

    // ── Command builder tests ────────────────────────────────────────

    #[test]
    fn cmd_check_user_exists_with_sudo() {
        let cmd = cmd_check_user_exists("deploy", true);
        assert_eq!(cmd, "sudo id -u 'deploy'");
    }

    #[test]
    fn cmd_check_user_exists_without_sudo() {
        let cmd = cmd_check_user_exists("deploy", false);
        assert_eq!(cmd, "id -u 'deploy'");
    }

    #[test]
    fn cmd_create_user_with_sudo() {
        let cmd = cmd_create_user("uptrakit", true);
        assert_eq!(
            cmd,
            "sudo useradd --create-home --shell /bin/bash 'uptrakit'"
        );
    }

    #[test]
    fn cmd_create_user_without_sudo() {
        let cmd = cmd_create_user("uptrakit", false);
        assert_eq!(cmd, "useradd --create-home --shell /bin/bash 'uptrakit'");
    }

    #[test]
    fn cmd_detect_home_with_sudo() {
        let cmd = cmd_detect_home("deploy", true);
        assert_eq!(cmd, "sudo getent passwd 'deploy' | cut -d: -f6");
    }

    #[test]
    fn cmd_setup_sudoers_content() {
        let cmd = cmd_setup_sudoers("uptrakit", true);
        assert!(cmd.contains("uptrakit ALL=(ALL) NOPASSWD: ALL"));
        assert!(cmd.contains("/etc/sudoers.d/uptrakit-uptrakit"));
        assert!(cmd.contains("chmod 440"));
    }

    #[test]
    fn cmd_validate_sudoers_with_sudo() {
        let cmd = cmd_validate_sudoers("deploy", true);
        assert_eq!(cmd, "sudo visudo -cf '/etc/sudoers.d/uptrakit-deploy'");
    }

    #[test]
    fn cmd_validate_sudoers_without_sudo() {
        let cmd = cmd_validate_sudoers("deploy", false);
        assert_eq!(cmd, "visudo -cf '/etc/sudoers.d/uptrakit-deploy'");
    }

    #[test]
    fn cmd_shell_escape_prevents_injection() {
        let cmd = cmd_check_user_exists("user'; rm -rf /; echo '", true);
        // The dangerous characters should be escaped inside single quotes.
        assert!(cmd.contains("'user'\\''"));
    }

    #[test]
    fn cmd_authorized_keys_structure() {
        let cmd = cmd_setup_authorized_keys(
            "/home/deploy",
            "ssh-ed25519 AAAA... comment",
            "deploy",
            true,
        );
        assert!(cmd.contains("mkdir -p"));
        assert!(cmd.contains("chmod 700"));
        assert!(cmd.contains("tee -a"));
        assert!(cmd.contains("chmod 600"));
        assert!(cmd.contains("chown -R"));
    }
}
