//! Proxmox guest bootstrap: set up an SSH host inside a PVE guest (LXC/QEMU)
//! by executing commands through the PVE node via `pct exec` / `qm guest exec`.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_command::RemoteExecutor;
use uptrakit_crypto::EncryptedString;
use uptrakit_plugin_infrastructure_core::agent_infra::GuestBootstrapExecutor;
use uptrakit_plugin_infrastructure_registry::PluginRegistry;

use crate::db::entity::ssh_host::SshKeyType;
use crate::error::{Error, Result};
use crate::host_ops::{self, AddHostParams};
use crate::operations::bootstrap;
use crate::operations::sudoers::{
    ResolvedSudoCommand, SudoersContent, install_helper_script, resolve_command_path,
    write_sudoers_file,
};
use crate::remote_exec::SshRemoteExecutor;
use crate::ssh_key;
use crate::ssh_transport::{self, AuthMethod, SshConnectionConfig, SshSession};

use std::path::Path;
use std::time::Duration;

/// Default SSH connect timeout for PVE host.
const PVE_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Parameters for the Proxmox guest bootstrap workflow.
pub(crate) struct ProxmoxBootstrapParams {
    /// Local DB ID of the PVE host to use as gateway.
    pub pve_host_id: String,
    /// VMID of the target guest.
    pub vmid: u32,
    /// Guest type string (e.g. `"lxc"` or `"qemu"`).
    pub guest_type: String,
    /// Friendly name for the new host entry.
    pub name: String,
    /// Username to create/use on the guest.
    pub target_username: String,
    /// Write `NOPASSWD: ALL` instead of specific commands.
    pub allow_all: bool,
    /// Remove existing Uptrakit-managed keys from `authorized_keys` before
    /// writing the new entry (mirrors `--remove-stale-keys` in the normal
    /// bootstrap). Same-service keys are always removed regardless of this flag.
    pub remove_stale_keys: bool,
    /// Pre-generated UUID for the new host DB entry.
    pub host_id: uuid::Uuid,
    /// Service UUID for the `authorized_keys` comment.
    pub service_id: Option<uuid::Uuid>,
}

/// Result of a successful Proxmox guest bootstrap.
pub(crate) struct ProxmoxBootstrapResult {
    /// The hostname or IP address of the guest (for the DB entry).
    ///
    /// Contains the FQDN when one can be confirmed via reverse DNS, otherwise
    /// the raw IP address returned by the PVE API.
    pub hostname: String,
}

/// A [`GuestBootstrapExecutor`] that always returns an error.
///
/// Used in contexts where no bootstrap will actually be performed (e.g., in the
/// `on_post_report_hosts` background task where guest execution is not needed).
pub(crate) struct NoopGuestBootstrapExecutor;

#[async_trait]
impl GuestBootstrapExecutor for NoopGuestBootstrapExecutor {
    async fn bootstrap_guest(
        &self,
        _params: uptrakit_plugin_infrastructure_core::agent_infra::GuestBootstrapParams,
    ) -> std::result::Result<
        uptrakit_plugin_infrastructure_core::agent_infra::GuestBootstrapResult,
        String,
    > {
        Err(
            "NoopGuestBootstrapExecutor: guest bootstrap is not supported in this context"
                .to_string(),
        )
    }
}

/// [`GuestBootstrapExecutor`] implementation for the SSH agent.
///
/// Translates generic `GuestBootstrapParams` from the infra plugin into
/// `ProxmoxBootstrapParams` and calls `run_proxmox_bootstrap`.
pub(crate) struct AgentGuestBootstrapExecutor {
    pub state_dir: std::path::PathBuf,
    pub service_id: Option<uuid::Uuid>,
}

#[async_trait]
impl GuestBootstrapExecutor for AgentGuestBootstrapExecutor {
    async fn bootstrap_guest(
        &self,
        params: uptrakit_plugin_infrastructure_core::agent_infra::GuestBootstrapParams,
    ) -> std::result::Result<
        uptrakit_plugin_infrastructure_core::agent_infra::GuestBootstrapResult,
        String,
    > {
        let proxmox_params = ProxmoxBootstrapParams {
            pve_host_id: params.gateway_host_id,
            vmid: params.guest_id,
            guest_type: params.guest_type,
            name: params.name,
            target_username: params.target_username,
            allow_all: params.allow_all,
            remove_stale_keys: params.remove_stale_keys,
            host_id: params.host_id,
            service_id: params.service_id.or(self.service_id),
        };

        run_proxmox_bootstrap(&self.state_dir, proxmox_params)
            .await
            .map(|r| {
                uptrakit_plugin_infrastructure_core::agent_infra::GuestBootstrapResult::new(
                    r.hostname,
                )
            })
            .map_err(|e| e.to_string())
    }
}

/// Run the Proxmox guest bootstrap workflow.
///
/// 1. Load the PVE host from the local DB
/// 2. Connect to the PVE node via SSH
/// 3. Create guest executors via the infra registry's `GuestExecProvider`
/// 4. Create user, deploy SSH key, configure sudoers inside the guest
/// 5. Get the guest's IP address
/// 6. Verify SSH connectivity to the guest
/// 7. Save the host to the local DB
pub(crate) async fn run_proxmox_bootstrap(
    state_dir: &Path,
    params: ProxmoxBootstrapParams,
) -> Result<ProxmoxBootstrapResult> {
    // 1. LOAD PVE HOST
    let db = crate::db::init_db(state_dir).await.map_err(|e| {
        report!(Error::Database(sea_orm::DbErr::Custom(format!(
            "failed to initialize local database: {e}"
        ))))
    })?;

    let pve_host = host_ops::find_host(&db, &params.pve_host_id)
        .await?
        .ok_or_else(|| {
            report!(Error::HostNotFound(format!(
                "PVE host '{}' not found",
                params.pve_host_id
            )))
        })?;

    // Check name uniqueness.
    let existing = host_ops::find_host(&db, &params.name).await?;
    if existing.is_some() {
        bail!(Error::HostNameConflict(params.name.clone()));
    }

    // 2. CONNECT TO PVE NODE
    let pve_key = pve_host.private_key.expose_secret();

    let port = u16::try_from(pve_host.port).map_err(|_| {
        report!(Error::InvalidInput(format!(
            "PVE host port must be 0-65535, got {}",
            pve_host.port
        )))
    })?;

    let config = SshConnectionConfig {
        hostname: pve_host.hostname.clone(),
        port,
        connect_timeout: PVE_CONNECT_TIMEOUT,
    };

    tracing::info!(
        pve_host = %pve_host.name,
        hostname = %pve_host.hostname,
        vmid = params.vmid,
        guest_type = %params.guest_type,
        "connecting to PVE node for guest bootstrap"
    );

    let (session, _) = ssh_transport::connect_and_authenticate(
        &config,
        &pve_host.username,
        &AuthMethod::PrivateKey(pve_key),
        pve_host.host_key_fingerprint.as_deref(),
    )
    .await?;

    let session = Arc::new(session);
    let pve_executor: Arc<dyn RemoteExecutor> =
        Arc::new(SshRemoteExecutor::new(Arc::clone(&session)));

    // 3. CREATE GUEST EXECUTORS via infra plugins
    let infra_plugins = uptrakit_plugin_infrastructure_registry::create_agent_infra_plugins();
    let guest_exec_provider = infra_plugins
        .iter()
        .find(|p| p.plugin_type_id() == "infrastructure_proxmox")
        .and_then(|p| p.as_guest_exec())
        .and_then(|g| g.guest_exec_provider())
        .ok_or_else(|| {
            report!(Error::InvalidInput(
                "no GuestExecProvider found for infrastructure_proxmox".to_string()
            ))
        })?;

    let guest_executor = guest_exec_provider.create_guest_remote_executor(
        Arc::clone(&pve_executor),
        params.vmid,
        &params.guest_type,
    );
    let guest_cmd_executor = guest_exec_provider.create_guest_command_executor(
        Arc::clone(&pve_executor),
        params.vmid,
        &params.guest_type,
    );

    // 4. GENERATE KEY MATERIAL
    let (target_private_pem, target_public_openssh) = ssh_key::generate_ed25519_keypair()?;

    // 5. REMOTE SETUP INSIDE GUEST
    // Commands inside LXC containers via `pct exec` run as root, so no sudo needed.
    let use_sudo = false;

    // Create user.
    let user_check = guest_executor
        .exec_command(&format!(
            "id -u {}",
            uptrakit_command::shell_escape(&params.target_username)
        ))
        .await
        .context_to::<Error>()?;

    if user_check.exit_code != 0 {
        tracing::info!(username = %params.target_username, "creating user in guest");
        let create_result = guest_executor
            .exec_command(&format!(
                "useradd --create-home --shell /bin/sh {}",
                uptrakit_command::shell_escape(&params.target_username)
            ))
            .await
            .context_to::<Error>()?;
        if create_result.exit_code != 0 {
            bail!(Error::SshCommand(format!(
                "failed to create user '{}' in guest: {}",
                params.target_username,
                create_result.stderr.trim()
            )));
        }
    }

    // Detect home directory.
    let home_result = guest_executor
        .exec_command(&format!(
            "getent passwd {} | cut -d: -f6",
            uptrakit_command::shell_escape(&params.target_username)
        ))
        .await
        .context_to::<Error>()?;
    let home_dir = home_result.stdout.trim().to_string();
    if home_dir.is_empty() {
        bail!(Error::SshCommand(format!(
            "could not determine home directory for user '{}' in guest",
            params.target_username
        )));
    }

    // Deploy authorized_keys (with stale key removal).
    let service_comment = match &params.service_id {
        Some(svc_id) => format!("uptrakit-svc:{svc_id}-host:{}", params.host_id),
        None => format!("uptrakit-host:{}", params.host_id),
    };

    let stripped_pubkey = target_public_openssh
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");

    let escaped_home = uptrakit_command::shell_escape(&home_dir);
    let escaped_user = uptrakit_command::shell_escape(&params.target_username);
    let ak_path = format!("{home_dir}/.ssh/authorized_keys");
    let escaped_ak_path = uptrakit_command::shell_escape(&ak_path);

    // Read existing authorized_keys (tolerate a missing file).
    let read_result = guest_executor
        .exec_command(&format!("cat {escaped_ak_path} 2>/dev/null || true"))
        .await
        .context_to::<Error>()?;
    let existing = bootstrap::parse_existing_authorized_keys(&read_result.stdout);

    // Auto-remove same-service keys (always, no flag required).
    let same_service_lines: Vec<String> = params
        .service_id
        .as_ref()
        .map(|svc_id| {
            existing
                .all_key_lines
                .iter()
                .filter(|l| bootstrap::is_same_service_key_line(l, svc_id))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // Only lines positively identified as Uptrakit-managed (comment starts
    // with `uptrakit`) are ever treated as stale — regardless of username.
    // We never assume exclusive ownership of an account.
    let stale_lines = existing.uptrakit_key_lines.clone();

    let mut to_remove: std::collections::HashSet<&str> =
        same_service_lines.iter().map(String::as_str).collect();
    if params.remove_stale_keys {
        for l in &stale_lines {
            to_remove.insert(l.as_str());
        }
    }

    let ak_entry = format!(
        "{} {stripped_pubkey} {service_comment}",
        bootstrap::authorized_keys_restrictions()
    );

    if to_remove.is_empty() {
        // No removals — append the new key.
        let escaped_entry = uptrakit_command::shell_escape(&ak_entry);
        let ak_cmd = format!(
            "mkdir -p {escaped_home}/.ssh && \
             chmod 700 {escaped_home}/.ssh && \
             echo {escaped_entry} >> {escaped_ak_path} && \
             chmod 600 {escaped_ak_path} && \
             chown -R {escaped_user}:{escaped_user} {escaped_home}/.ssh"
        );
        let ak_result = guest_executor
            .exec_command(&ak_cmd)
            .await
            .context_to::<Error>()?;
        if ak_result.exit_code != 0 {
            bail!(Error::SshCommand(format!(
                "failed to deploy authorized_keys in guest: {}",
                ak_result.stderr.trim()
            )));
        }
    } else {
        // Removals required — atomically rewrite the file.
        let keep_lines: Vec<&str> = existing
            .all_key_lines
            .iter()
            .map(String::as_str)
            .filter(|l| !to_remove.contains(l))
            .collect();
        let new_content = if keep_lines.is_empty() {
            format!("{ak_entry}\n")
        } else {
            format!("{}\n{ak_entry}\n", keep_lines.join("\n"))
        };

        let escaped_content = uptrakit_command::shell_escape(&new_content);
        let ak_cmd = format!(
            "mkdir -p {escaped_home}/.ssh && \
             chmod 700 {escaped_home}/.ssh && \
             printf '%s' {escaped_content} | tee {escaped_ak_path} > /dev/null && \
             chmod 600 {escaped_ak_path} && \
             chown -R {escaped_user}:{escaped_user} {escaped_home}/.ssh"
        );
        let ak_result = guest_executor
            .exec_command(&ak_cmd)
            .await
            .context_to::<Error>()?;
        if ak_result.exit_code != 0 {
            bail!(Error::SshCommand(format!(
                "failed to write authorized_keys in guest: {}",
                ak_result.stderr.trim()
            )));
        }
    }

    // Configure sudoers.
    // Use the guest command executor so compatibility probes (e.g. `which apt`,
    // `which brew`) run against the *guest* rather than the PVE host.
    let plugin_sudo_cmds =
        PluginRegistry::compatible_sudo_commands_for_host(guest_cmd_executor).await;
    let mut resolved: Vec<ResolvedSudoCommand> = Vec::new();

    for (_plugin_type, entries) in &plugin_sudo_cmds {
        for entry in entries {
            if let Some(helper) = &entry.helper_script {
                install_helper_script(guest_executor.as_ref(), helper, use_sudo).await?;
                resolved.push(ResolvedSudoCommand {
                    command_path: helper.install_path.to_string(),
                    explanation: entry.explanation.clone(),
                    needs_setenv: entry.needs_setenv,
                });
            } else if let Some(path) =
                resolve_command_path(guest_executor.as_ref(), &entry.command).await?
            {
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
        }
    }

    let sudoers_content: Option<SudoersContent> = if !resolved.is_empty() {
        Some(SudoersContent::SpecificCommands(resolved))
    } else if params.allow_all {
        Some(SudoersContent::AllCommands)
    } else {
        None
    };

    if let Some(ref content) = sudoers_content {
        write_sudoers_file(
            guest_executor.as_ref(),
            &params.target_username,
            content,
            use_sudo,
        )
        .await?;
    }

    // Get host key fingerprint from guest.
    let fp_result = guest_executor
        .exec_command(
            "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub 2>/dev/null || \
                       ssh-keygen -lf /etc/ssh/ssh_host_rsa_key.pub 2>/dev/null || true",
        )
        .await
        .context_to::<Error>()?;
    let host_key_fingerprint = fp_result
        .stdout
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(String::from);

    // 6. GET GUEST IP
    let guest_ip = guest_exec_provider
        .get_guest_ip(pve_executor.as_ref(), params.vmid, &params.guest_type)
        .await
        .map_err(|e| report!(Error::SshCommand(format!("failed to get guest IP: {e}"))))?;

    tracing::info!(guest_ip = %guest_ip, "resolved guest IP address");

    // 7. VERIFY SSH CONNECTIVITY
    let verify_config = SshConnectionConfig {
        hostname: guest_ip.clone(),
        port: 22,
        connect_timeout: PVE_CONNECT_TIMEOUT,
    };

    let (verify_session, observed_fp) = ssh_transport::connect_and_authenticate(
        &verify_config,
        &params.target_username,
        &AuthMethod::PrivateKey(&target_private_pem),
        host_key_fingerprint.as_deref(),
    )
    .await
    .map_err(|e| {
        report!(Error::BootstrapVerification(format!(
            "failed to verify SSH to guest {}: {e}",
            guest_ip
        )))
    })?;

    let whoami = verify_session.exec_command("whoami").await?;
    if whoami.stdout.trim() != params.target_username {
        bail!(Error::BootstrapVerification(format!(
            "whoami returned '{}', expected '{}'",
            whoami.stdout.trim(),
            params.target_username
        )));
    }

    let hostname = try_detect_fqdn(&verify_session, &guest_ip).await;
    verify_session.disconnect().await;

    // Drop executors before disconnecting so the session Arc has a single
    // owner — `disconnect_shared` requires sole ownership.
    drop(guest_executor);
    drop(pve_executor);
    SshSession::disconnect_shared(session).await;

    // 8. SAVE TO DATABASE
    let encrypted_key =
        EncryptedString::new(target_private_pem.clone(), "uptrakit:ssh_hosts:private_key")
            .map_err(|e| report!(Error::Crypto(format!("failed to encrypt private key: {e}"))))?;

    host_ops::add_host(
        &db,
        AddHostParams {
            host_id: params.host_id,
            name: params.name.clone(),
            hostname: hostname.clone(),
            port: 22,
            username: params.target_username.clone(),
            encrypted_key,
            key_type: SshKeyType::Ed25519,
            host_key_fingerprint: Some(observed_fp),
        },
    )
    .await?;

    tracing::info!(
        host_id = %params.host_id,
        name = %params.name,
        %hostname,
        "Proxmox guest bootstrap complete"
    );

    Ok(ProxmoxBootstrapResult { hostname })
}

/// Attempt to resolve a fully-qualified domain name (FQDN) for a guest.
///
/// Steps:
/// 1. Runs `hostname -f` on the already-authenticated SSH session to obtain
///    the guest's self-reported FQDN.
/// 2. Collects **all** IP addresses assigned to the guest via `hostname -I`
///    (space-separated; falls back to the single `ip` argument on failure).
/// 3. Performs a forward DNS lookup for the FQDN and checks whether **any**
///    resolved address matches **any** of the guest's own IPs.
///
/// This handles containers/VMs with multiple network interfaces where the PVE
/// API may report only one IP while the FQDN resolves to a different interface.
///
/// Returns the FQDN on success or the original `ip` string on any failure.
async fn try_detect_fqdn(session: &SshSession, ip: &str) -> String {
    // Step 1: Obtain FQDN from the guest.
    let fqdn_result = match session
        .exec_command("hostname -f 2>/dev/null || hostname 2>/dev/null")
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "hostname command failed; keeping IP as hostname");
            return ip.to_string();
        }
    };

    let fqdn = fqdn_result.stdout.trim().to_string();

    // Must be non-empty and contain a dot (i.e. be a proper FQDN, not a bare
    // hostname or an IP literal).
    if fqdn.is_empty() || !fqdn.contains('.') || fqdn.parse::<std::net::IpAddr>().is_ok() {
        tracing::debug!(
            fqdn = %fqdn,
            "hostname output is not a valid FQDN; keeping IP as hostname"
        );
        return ip.to_string();
    }

    // Step 2: Collect all IPs assigned to the guest.
    //
    // `hostname -I` prints all addresses separated by spaces.  We fall back to
    // the single IP returned by the PVE API if the command fails or returns
    // nothing useful.
    let guest_addrs: std::collections::HashSet<std::net::IpAddr> = {
        let mut addrs = std::collections::HashSet::new();

        if let Ok(r) = session.exec_command("hostname -I 2>/dev/null").await {
            for token in r.stdout.split_whitespace() {
                if let Ok(a) = token.parse::<std::net::IpAddr>() {
                    addrs.insert(a);
                }
            }
        }

        // Always include the PVE-reported IP as a fallback.
        if let Ok(a) = ip.parse::<std::net::IpAddr>() {
            addrs.insert(a);
        }

        addrs
    };

    // Step 3: Forward-confirm the FQDN via DNS.
    let lookup_target = format!("{fqdn}:0");
    match tokio::net::lookup_host(lookup_target).await {
        Ok(mut resolved) => {
            let confirmed = resolved.any(|sa| guest_addrs.contains(&sa.ip()));
            if confirmed {
                tracing::info!(
                    fqdn = %fqdn,
                    ip,
                    "FQDN confirmed via forward DNS; using FQDN as hostname"
                );
                fqdn
            } else {
                tracing::debug!(
                    fqdn = %fqdn,
                    ip,
                    "FQDN did not resolve to any of the guest's IPs; keeping IP as hostname"
                );
                ip.to_string()
            }
        }
        Err(e) => {
            tracing::debug!(
                error = %e,
                fqdn = %fqdn,
                "DNS lookup for FQDN failed; keeping IP as hostname"
            );
            ip.to_string()
        }
    }
}
