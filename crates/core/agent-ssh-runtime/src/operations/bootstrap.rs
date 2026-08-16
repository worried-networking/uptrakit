//! Bootstrap command: automates remote host setup (user creation, SSH key
//! deployment, sudoers configuration) and saves the host entry to the local
//! database.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_crypto::EncryptedString;
use uptrakit_plugin_infrastructure_registry::agent_infra::{
    BootstrapInfraResult, GuestBootstrapError, GuestBootstrapExecutor, GuestBootstrapParams,
    GuestBootstrapResult, InfraActionInvokeError, InfraActionInvoker, InfraPluginContext,
};
use uptrakit_plugin_infrastructure_registry::{
    CatalogConfig, SudoCommandEntry, build_catalog, compatible_sudo_commands_for_host,
};
use uptrakit_shared_types::SecretString;

use crate::error::{Error, Result};
use crate::host_ops::{self, AddHostParams};
use crate::operations::bootstrap_routeros::{
    RouterOsBootstrapParams, execute_bootstrap_routeros, plan_bootstrap_routeros,
};
use crate::operations::sudoers::{
    ResolvedSudoCommand, SudoersContent, detect_is_root, ensure_docker_group_membership,
    install_helper_script, resolve_command_path, write_sudoers_file,
};
use crate::remote_exec::SshRemoteExecutor;
use crate::ssh_executor::{PosixSshCommandExecutor, SshCommandExecutor};
use crate::ssh_key;
use crate::ssh_transport::{self, AuthMethod, SshConnectionConfig, SshSession};

/// Maximum length for POSIX usernames.
const MAX_USERNAME_LEN: usize = 32;

/// Default SSH connect timeout.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for the OS detection probe command.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

// ── Host OS detection ──────────────────────────────────────────────────────

/// Detected operating system class of the remote host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostOs {
    RouterOs,
    Posix,
}

/// Detect whether the remote host is RouterOS or a POSIX system.
///
/// Allow-list banner detection runs first: the SSH identification banner
/// (peeked before authentication and surfaced via [`SshSession::server_software`])
/// distinguishes RouterOS (`ROSSSH`), OpenSSH-based POSIX (`OpenSSH_*`), and
/// Dropbear-based POSIX (`dropbear`) without round-trips. Unknown banners
/// (Cisco IOS, Junos, FortiOS, BMCs, etc.) fall through to the legacy
/// `/system resource print` shell probe — safer than guessing, since
/// mis-classifying a network appliance as POSIX would let bootstrap write
/// `~/.ssh/authorized_keys` and run `sudo` on a non-shell platform.
async fn detect_host_os(session: &SshSession, exec: &SshCommandExecutor) -> Result<HostOs> {
    if let Some(software) = session.server_software() {
        if software.contains("ROSSSH") {
            tracing::debug!(server_software = %software, "RouterOS detected from SSH banner");
            return Ok(HostOs::RouterOs);
        }
        if software.starts_with("OpenSSH_") || software.contains("dropbear") {
            tracing::debug!(server_software = %software, "POSIX host detected from SSH banner");
            return Ok(HostOs::Posix);
        }
        tracing::debug!(
            server_software = %software,
            "ssh banner not in allow-list; falling back to shell probe",
        );
    }
    // Banner peek failed or banner is not in the allow-list — run the shell probe.
    match exec
        .exec_raw("/system resource print", Some(PROBE_TIMEOUT))
        .await
    {
        Ok(output) if output.contains("platform:") || output.contains("MikroTik") => {
            Ok(HostOs::RouterOs)
        }
        Ok(output)
            if output.contains("not enough permissions")
                && !output.contains("No such file or directory")
                && !output.contains("command not found")
                && !output.contains("Permission denied") =>
        {
            bail!(Error::SshCommand(
                "RouterOS device detected but insufficient permissions for \
                 `/system resource print` — grant `read` policy to the connecting account"
                    .to_string()
            ))
        }
        _ => Ok(HostOs::Posix),
    }
}

// ── Bootstrap parameters ─────────────────────────────────────────────

/// Parameters for the bootstrap workflow, mirroring CLI args.
///
/// `Debug` is safe because every secret-bearing field is wrapped in
/// [`SecretString`], which has a redacted `Debug` impl.
#[derive(Debug)]
pub(crate) struct BootstrapParams {
    pub name: String,
    pub hostname: String,
    pub port: i32,
    pub auth_username: String,
    pub auth_password: Option<SecretString>,
    pub auth_private_key_pem: Option<SecretString>,
    /// Use the local SSH agent for authentication (detected from `SSH_AUTH_SOCK`).
    pub use_ssh_agent: bool,
    pub target_username: String,
    pub target_private_key_pem: Option<SecretString>,
    pub host_key_fingerprint: Option<String>,
    /// When `true`, `host_key_fingerprint` must be `Some` and TOFU is disabled.
    pub strict_host_key_checking: bool,
    /// Write `NOPASSWD: ALL` instead of specific command entries.
    ///
    /// Less secure; use only when no plugin commands can be resolved on the
    /// remote host or during development.
    pub allow_all: bool,
    /// Pre-generated UUID for the new host DB entry.
    ///
    /// Generated by the caller before `run_bootstrap` so that the same UUID
    /// can be embedded in the `authorized_keys` comment for identification.
    pub host_id: uuid::Uuid,
    /// Service UUID for the `authorized_keys` comment.  `None` when the
    /// service has not yet been enrolled.
    pub service_id: Option<uuid::Uuid>,
    /// Tenant UUID for PVE API credential naming.  `None` when the tenant
    /// ID has not yet been received from the controller.
    pub tenant_id: Option<uuid::Uuid>,
    /// Remove existing Uptrakit-managed keys from `authorized_keys` before
    /// writing the new entry.
    pub remove_stale_keys: bool,
    /// Whether to grant the `reboot` policy to the RouterOS `uptrakit` group.
    /// Only relevant for RouterOS hosts; ignored for POSIX bootstrap.
    pub allow_reboot: bool,
}

/// Result of a successful bootstrap, carrying metadata for the event loop.
pub(crate) struct BootstrapResult {
    /// Infrastructure detection results from each plugin that ran
    /// `on_host_bootstrapped`. The event loop uses these to send
    /// `ReportPluginConfig` or update host state.
    pub infra_results: Vec<BootstrapInfraResult>,
}

// ── Multi-step bootstrap types ───────────────────────────────────────

/// A planned bootstrap action that the user can review and optionally skip.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct PlannedAction {
    /// Machine-readable action identifier (e.g., "create_user", "deploy_key").
    pub id: String,
    /// Human-readable label for the action.
    pub label: String,
    /// Description of what this action does.
    pub description: String,
    pub security_impact: uptrakit_shared_types::Severity,
    /// Whether this action is enabled by default.
    pub default_enabled: bool,
    /// Whether the user can skip this action.
    pub skippable: bool,
    /// Human-readable preview of the commands this action will run or configure.
    pub commands: Vec<String>,
}

/// Information gathered about the target host during the connect phase.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct BootstrapHostInfo {
    pub hostname: String,
    pub port: u16,
    pub auth_user: String,
    pub is_root: bool,
    pub os_info: Option<String>,
    pub host_key_fingerprint: String,
    pub target_user_exists: bool,
}

/// The result of the connect phase: a plan for the user to review.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct BootstrapPlan {
    pub host_info: BootstrapHostInfo,
    pub actions: Vec<PlannedAction>,
}

// ── Multi-step: connect (read-only probe) ────────────────────────────

/// Probe the remote host and build a plan of actions for the user to review.
///
/// This phase is non-destructive: it connects via SSH, gathers host
/// information, determines which actions are applicable, then disconnects.
pub(crate) async fn bootstrap_connect(
    db: &sea_orm::DatabaseConnection,
    state_dir: &Path,
    params: &BootstrapParams,
) -> Result<BootstrapPlan> {
    // 1. VALIDATE INPUTS
    validate_bootstrap_inputs(params)?;

    // Fail fast: check host name is not in DB.
    let existing = host_ops::find_host(db, &params.name).await?;
    if existing.is_some() {
        bail!(Error::HostNameConflict(params.name.clone()));
    }

    // 2. PREPARE KEY MATERIAL (validate only, not persisted)
    if let Some(pem) = &params.target_private_key_pem {
        ssh_key::extract_public_key_openssh(pem.expose_secret())?;
    }

    // 3. CONNECT & AUTHENTICATE (as auth_username)
    #[expect(
        clippy::map_err_ignore,
        reason = "TryFromIntError carries no contextual information beyond what the formatted error message conveys"
    )]
    let port = u16::try_from(params.port).map_err(|_| {
        report!(Error::InvalidInput(format!(
            "port must be 0-65535, got {}",
            params.port
        )))
    })?;

    let PreparedBootstrapConnection {
        session,
        observed_fingerprint: observed_fp,
        executor,
        use_sudo,
        host_os,
    } = prepare_bootstrap_connection(params, port).await?;

    // 3b. Route to RouterOS plan when applicable. Host OS was detected inside
    // prepare_bootstrap_connection so we can reuse the cached value.
    if matches!(host_os, HostOs::RouterOs) {
        drop(executor);
        SshSession::disconnect_shared(session).await;

        let ros_params = routeros_params_from_bootstrap(params);
        let ros_plan = plan_bootstrap_routeros(&ros_params);
        let actions = routeros_plan_to_planned_actions(&ros_plan, &ros_params);
        let host_info = BootstrapHostInfo {
            hostname: params.hostname.clone(),
            port,
            auth_user: params.auth_username.clone(),
            is_root: true,
            os_info: Some("RouterOS".to_string()),
            host_key_fingerprint: observed_fp,
            target_user_exists: false,
        };
        return Ok(BootstrapPlan { host_info, actions });
    }

    // 4. GATHER HOST INFORMATION
    let remote_info =
        gather_remote_host_info(&session, &executor, params, use_sudo, state_dir, db).await?;

    // Collect plugin sudo commands to build a preview for the review step.
    let ssh_executor = Arc::new(PosixSshCommandExecutor::new(Arc::clone(&session)))
        as Arc<dyn uptrakit_command::CommandExecutor>;
    let plugin_sudo_cmds = compatible_sudo_commands_for_host(ssh_executor).await;
    let sudo_command_previews: Vec<String> = plugin_sudo_cmds
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .map(|entry| {
            if entry.helper_script.is_some() {
                format!("{} [helper script] — {}", entry.command, entry.explanation)
            } else if let Some(ref suffix) = entry.args_suffix {
                format!("{} {} — {}", entry.command, suffix, entry.explanation)
            } else {
                format!("{} — {}", entry.command, entry.explanation)
            }
        })
        .collect();

    // 5. DISCONNECT
    drop(executor);
    SshSession::disconnect_shared(session).await;

    // 6. BUILD PLAN
    let host_info = BootstrapHostInfo {
        hostname: params.hostname.clone(),
        port,
        auth_user: params.auth_username.clone(),
        is_root: !use_sudo,
        os_info: remote_info.os_info.clone(),
        host_key_fingerprint: observed_fp,
        target_user_exists: remote_info.target_user_exists,
    };
    let actions = build_bootstrap_actions(params, &remote_info, sudo_command_previews);

    Ok(BootstrapPlan { host_info, actions })
}

/// Information gathered during the read-only remote probe phase.
struct RemoteHostInfo {
    target_user_exists: bool,
    os_info: Option<String>,
    docker_group_exists: bool,
    stale_keys_found: bool,
    pve_detected: bool,
    pve_planned_actions: Vec<String>,
}

/// Gather host information via SSH: user existence, OS, docker group,
/// stale keys, and infrastructure plugin probing.
async fn gather_remote_host_info(
    session: &SshSession,
    executor: &SshRemoteExecutor,
    params: &BootstrapParams,
    use_sudo: bool,
    state_dir: &Path,
    db: &DatabaseConnection,
) -> Result<RemoteHostInfo> {
    let target_same_as_auth = params.target_username == params.auth_username;
    let target_user_exists = if target_same_as_auth {
        true
    } else {
        let cmd = cmd_check_user_exists(&params.target_username, use_sudo);
        let user_check = session.exec_command(&cmd).await?;
        user_check.exit_code == 0
    };

    let os_result = session
        .exec_command("cat /etc/os-release 2>/dev/null | head -1 || uname -s")
        .await?;
    let os_info = {
        let raw = os_result.stdout.trim().to_string();
        if raw.is_empty() { None } else { Some(raw) }
    };

    let docker_cmd = if use_sudo {
        "sudo getent group docker".to_string()
    } else {
        "getent group docker".to_string()
    };
    let docker_result = session.exec_command(&docker_cmd).await?;
    let docker_group_exists = docker_result.exit_code == 0;

    let stale_keys_found = if !target_same_as_auth || params.remove_stale_keys {
        let home_cmd = cmd_detect_home(&params.target_username, use_sudo);
        let home_result = session.exec_command(&home_cmd).await?;
        let home_dir = home_result.stdout.trim().to_string();
        if !home_dir.is_empty() {
            let read_cmd = cmd_read_authorized_keys(&home_dir, use_sudo);
            let read_result = session.exec_command(&read_cmd).await?;
            let existing_keys = parse_existing_authorized_keys(&read_result.stdout);
            !existing_keys.uptrakit_key_lines.is_empty()
        } else {
            false
        }
    } else {
        false
    };

    let (pve_detected, pve_planned_actions) =
        detect_infra_plugins(executor, params, state_dir, db).await;

    Ok(RemoteHostInfo {
        target_user_exists,
        os_info,
        docker_group_exists,
        stale_keys_found,
        pve_detected,
        pve_planned_actions,
    })
}

/// Run infrastructure plugin detection (read-only probing).
async fn detect_infra_plugins(
    executor: &SshRemoteExecutor,
    params: &BootstrapParams,
    state_dir: &Path,
    db: &DatabaseConnection,
) -> (bool, Vec<String>) {
    let catalog_config = CatalogConfig::default();
    let Ok(catalog) = build_catalog(
        &catalog_config,
        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
    ) else {
        return (false, Vec::new());
    };
    let infra_bundles = catalog.create_infra_bundles(&catalog_config);
    let noop_invoker = NoopInfraActionInvoker;
    let noop_bootstrap = NoopGuestBootstrap;
    let tenant_id_str = params.tenant_id.map(|t| t.to_string());
    let infra_ctx = InfraPluginContext {
        db,
        tenant_id: tenant_id_str.as_deref(),
        service_id: params.service_id,
        state_dir,
        private_key_der: None,
        action_invoker: &noop_invoker,
        guest_bootstrap: &noop_bootstrap,
        // The connect phase never provisions — probe_host ignores this flag
        // today, but leaving it `true` here would be a live trap for the
        // next implementor that adds a provisioning probe.
        provision_credentials: false,
    };
    let mut detected = false;
    let mut planned = Vec::new();
    for bundle in &infra_bundles {
        let Some(lifecycle) = bundle.lifecycle.as_ref() else {
            continue;
        };
        match lifecycle
            .probe_host(&infra_ctx, executor, params.host_id, &params.name)
            .await
        {
            Ok(result) => {
                detected |= result.detected;
                planned.extend(result.planned_actions);
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    plugin = %lifecycle.plugin_type_id(),
                    "infrastructure detection probe failed, skipping"
                );
            }
        }
    }
    (detected, planned)
}

/// Build the list of planned bootstrap actions from gathered host info.
fn build_bootstrap_actions(
    params: &BootstrapParams,
    info: &RemoteHostInfo,
    sudo_command_previews: Vec<String>,
) -> Vec<PlannedAction> {
    let target_same_as_auth = params.target_username == params.auth_username;
    let mut actions = Vec::new();

    if !target_same_as_auth && !info.target_user_exists {
        actions.push(PlannedAction {
            id: "create_user".to_string(),
            label: format!("Create user '{}'", params.target_username),
            description: format!(
                "Create a new system user '{}' with a home directory on the remote host.",
                params.target_username
            ),
            security_impact: uptrakit_shared_types::Severity::Medium,
            default_enabled: true,
            skippable: true,
            commands: vec![],
        });
    }

    actions.push(PlannedAction {
        id: "deploy_key".to_string(),
        label: "Deploy SSH authorized key".to_string(),
        description: format!(
            "Install the Uptrakit SSH public key into ~{}/.ssh/authorized_keys.",
            params.target_username
        ),
        security_impact: uptrakit_shared_types::Severity::Medium,
        default_enabled: true,
        skippable: false,
        commands: vec![],
    });

    actions.push(PlannedAction {
        id: "configure_sudoers".to_string(),
        label: "Configure sudoers".to_string(),
        description: format!(
            "Write /etc/sudoers.d/uptrakit-{} with NOPASSWD entries for {} plugin command(s).",
            params.target_username,
            sudo_command_previews.len()
        ),
        security_impact: uptrakit_shared_types::Severity::High,
        default_enabled: true,
        skippable: true,
        commands: sudo_command_previews,
    });

    if info.stale_keys_found {
        actions.push(PlannedAction {
            id: "remove_stale_keys".to_string(),
            label: "Remove stale Uptrakit keys".to_string(),
            description: "Remove previously-deployed Uptrakit SSH keys from authorized_keys."
                .to_string(),
            security_impact: uptrakit_shared_types::Severity::Low,
            default_enabled: true,
            skippable: true,
            commands: vec![],
        });
    }

    if info.docker_group_exists {
        actions.push(PlannedAction {
            id: "docker_group".to_string(),
            label: "Add user to docker group".to_string(),
            description: format!(
                "Add '{}' to the docker group for container management without sudo.",
                params.target_username
            ),
            security_impact: uptrakit_shared_types::Severity::Low,
            default_enabled: true,
            skippable: true,
            commands: vec![],
        });
    }

    if info.pve_detected {
        actions.push(PlannedAction {
            id: "pve_setup".to_string(),
            label: "Proxmox VE setup".to_string(),
            description: if info.pve_planned_actions.is_empty() {
                "Configure Proxmox VE infrastructure integration.".to_string()
            } else {
                format!(
                    "Configure Proxmox VE infrastructure integration: {}.",
                    info.pve_planned_actions.join("; ")
                )
            },
            security_impact: uptrakit_shared_types::Severity::Medium,
            default_enabled: true,
            skippable: true,
            commands: vec![],
        });
    }

    actions
}

// ── Multi-step: execute (applies changes) ────────────────────────────

/// Execute the bootstrap plan, skipping actions whose IDs are in
/// `skip_actions`.
///
/// Reconnects via SSH, performs the requested modifications, verifies
/// connectivity with the target key, and saves the host to the database.
pub(crate) async fn bootstrap_execute(
    db: &sea_orm::DatabaseConnection,
    state_dir: &Path,
    params: BootstrapParams,
    skip_actions: &HashSet<String>,
) -> Result<BootstrapResult> {
    // Re-validate (caller may have constructed params independently).
    validate_bootstrap_inputs(&params)?;

    // Prepare key material.
    let (target_private_pem, target_public_openssh, generated_key) =
        match &params.target_private_key_pem {
            Some(pem) => {
                let pubkey = ssh_key::extract_public_key_openssh(pem.expose_secret())?;
                (pem.expose_secret().to_owned(), pubkey, false)
            }
            None => {
                tracing::info!("generating Ed25519 keypair for target user");
                let (priv_pem, pub_openssh) = ssh_key::generate_ed25519_keypair()?;
                (priv_pem, pub_openssh, true)
            }
        };

    let key_type = ssh_key::detect_key_type(&target_private_pem)?;

    // Connect via SSH.
    #[expect(
        clippy::map_err_ignore,
        reason = "TryFromIntError carries no contextual information beyond what the formatted error message conveys"
    )]
    let port = u16::try_from(params.port).map_err(|_| {
        report!(Error::InvalidInput(format!(
            "port must be 0-65535, got {}",
            params.port
        )))
    })?;

    let PreparedBootstrapConnection {
        session,
        observed_fingerprint: observed_fp,
        executor,
        use_sudo,
        host_os,
    } = prepare_bootstrap_connection(&params, port).await?;

    // Route to RouterOS execute path when applicable. Host OS was detected
    // inside prepare_bootstrap_connection so we can reuse the cached value.
    if matches!(host_os, HostOs::RouterOs) {
        drop(executor);
        let ros_params = routeros_params_from_bootstrap_with_fp(&params, observed_fp);
        execute_bootstrap_routeros(&ros_params, session, db).await?;
        return Ok(BootstrapResult {
            infra_results: Vec::new(),
        });
    }

    // Execute non-skipped actions.
    let target_same_as_auth = params.target_username == params.auth_username;

    // Create user (if applicable and not skipped).
    if !target_same_as_auth && !skip_actions.contains("create_user") {
        let cmd = cmd_check_user_exists(&params.target_username, use_sudo);
        let user_check = session.exec_command(&cmd).await?;

        if user_check.exit_code != 0 {
            tracing::info!(user = %params.target_username, "creating user");
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
            tracing::debug!(user = %params.target_username, "user already exists, skipping creation");
        }
    }

    // Configure docker group membership (if not skipped).
    if !skip_actions.contains("docker_group") {
        tracing::info!("configuring docker group membership");
        ensure_docker_group_membership(&executor, &params.target_username, use_sudo).await?;
    }

    // Detect home directory (always needed for key deployment).
    let home_cmd = cmd_detect_home(&params.target_username, use_sudo);
    let home_result = session.exec_command(&home_cmd).await?;
    let home_dir = home_result.stdout.trim().to_string();
    if home_dir.is_empty() {
        bail!(Error::SshCommand(format!(
            "could not determine home directory for user '{}'",
            params.target_username
        )));
    }

    // Deploy authorized_keys (not skippable).
    if !skip_actions.contains("deploy_key") {
        let effective_remove_stale =
            params.remove_stale_keys && !skip_actions.contains("remove_stale_keys");
        deploy_authorized_keys(
            &session,
            &home_dir,
            &target_public_openssh,
            &params.target_username,
            params.host_id,
            params.service_id.as_ref(),
            effective_remove_stale,
            use_sudo,
        )
        .await?;
    }

    // Set up sudoers and run infra plugin detection. `setup_sudoers_and_plugins`
    // derives the pve_setup/configure_sudoers skip semantics internally from
    // `skip_actions`; infra detection always runs (credential provisioning is
    // gated inside), and the sudoers write happens exactly once, after
    // infra-contributed entries are merged in.
    //
    // Skip the (remote, multi-command) compatibility probe when its result
    // would be discarded by the fn's skip_sudoers early return.
    let plugin_sudo_cmds = if skip_actions.contains("configure_sudoers") {
        Vec::new()
    } else {
        let ssh_executor = Arc::new(PosixSshCommandExecutor::new(Arc::clone(&session)))
            as Arc<dyn uptrakit_command::CommandExecutor>;
        compatible_sudo_commands_for_host(ssh_executor).await
    };
    let (sudoers_content, infra_results) = setup_sudoers_and_plugins(
        &executor,
        &params,
        db,
        state_dir,
        use_sudo,
        skip_actions,
        plugin_sudo_cmds,
    )
    .await
    .map_err(|e| {
        report!(Error::BootstrapVerification(format!(
            "failed to configure sudoers/infrastructure for host '{}': {e}. \
             The remote host has been partially configured (user created, \
             key deployed). Manual cleanup may be required.",
            params.name
        )))
    })?;

    // Disconnect auth session.
    drop(executor);
    SshSession::disconnect_shared(session).await;

    // Verify connectivity as target user.
    tracing::info!(user = %params.target_username, "verifying connectivity");

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

    verify_remote(
        &verify_session,
        &params.target_username,
        sudoers_content.is_some(),
    )
    .await?;
    verify_session.disconnect().await;

    // Save to database.
    save_host(
        db,
        &params,
        &target_private_pem,
        key_type,
        &observed_fp,
        params.host_id,
    )
    .await?;

    // Log summary.
    tracing::info!(
        host = %params.name,
        hostname = %params.hostname,
        port = params.port,
        target_user = %params.target_username,
        key_type = %key_type,
        host_key = %observed_fp,
        "bootstrap complete"
    );

    if generated_key {
        tracing::info!(
            "Ed25519 private key generated in memory; stored only in encrypted local database"
        );
    }

    if sudoers_content.is_some() {
        tracing::info!(
            sudoers_path = %format!("/etc/sudoers.d/uptrakit-{}", params.target_username),
            "sudoers file written; run 'host sync' to refresh entries when plugins change"
        );
    } else {
        tracing::warn!("no sudoers file written; run 'host sync' after installing supported tools");
    }

    Ok(BootstrapResult { infra_results })
}

// ── RouterOS conversion helpers ───────────────────────────────────────

/// Build [`RouterOsBootstrapParams`] from a generic [`BootstrapParams`]
/// (used in the connect phase, where no fingerprint is available yet).
fn routeros_params_from_bootstrap(params: &BootstrapParams) -> RouterOsBootstrapParams {
    RouterOsBootstrapParams {
        name: params.name.clone(),
        hostname: params.hostname.clone(),
        port: params.port,
        auth_username: params.auth_username.clone(),
        auth_password: params.auth_password.clone(),
        auth_private_key_pem: params.auth_private_key_pem.clone(),
        use_ssh_agent: params.use_ssh_agent,
        host_key_fingerprint: params.host_key_fingerprint.clone(),
        strict_host_key_checking: params.strict_host_key_checking,
        allow_reboot: params.allow_reboot,
        host_id: params.host_id,
    }
}

/// Build [`RouterOsBootstrapParams`] with a confirmed host-key fingerprint
/// (used in the execute phase after the fingerprint has been observed).
fn routeros_params_from_bootstrap_with_fp(
    params: &BootstrapParams,
    observed_fp: String,
) -> RouterOsBootstrapParams {
    RouterOsBootstrapParams {
        host_key_fingerprint: Some(observed_fp),
        ..routeros_params_from_bootstrap(params)
    }
}

/// Convert a RouterOS planned-action list into the generic [`PlannedAction`]
/// format understood by the UI review step.
fn routeros_plan_to_planned_actions(
    plan: &[crate::operations::bootstrap_routeros::RouterOsPlannedAction],
    params: &RouterOsBootstrapParams,
) -> Vec<PlannedAction> {
    use crate::operations::bootstrap_routeros::RouterOsPlannedAction;

    plan.iter()
        .map(|action| match action {
            RouterOsPlannedAction::CreateGroup { policies } => PlannedAction {
                id: "routeros_create_group".to_string(),
                label: "Create uptrakit group".to_string(),
                description: format!(
                    "Create a RouterOS user group named 'uptrakit' with policies: {}.",
                    policies.join(", ")
                ),
                security_impact: uptrakit_shared_types::Severity::Medium,
                default_enabled: true,
                skippable: false,
                commands: vec![format!(
                    "/user group add name=uptrakit policy={}",
                    policies.join(",")
                )],
            },
            RouterOsPlannedAction::CreateUser => PlannedAction {
                id: "routeros_create_user".to_string(),
                label: "Create uptrakit user".to_string(),
                description: "Create a RouterOS user named 'uptrakit' in the 'uptrakit' group."
                    .to_string(),
                security_impact: uptrakit_shared_types::Severity::Medium,
                default_enabled: true,
                skippable: false,
                commands: vec![r#"/user add name=uptrakit group=uptrakit password="""#.to_string()],
            },
            RouterOsPlannedAction::UploadPublicKey { remote_path } => PlannedAction {
                id: "routeros_upload_key".to_string(),
                label: "Upload SSH public key".to_string(),
                description: format!(
                    "Upload the generated Ed25519 public key to '{remote_path}' via SFTP."
                ),
                security_impact: uptrakit_shared_types::Severity::Low,
                default_enabled: true,
                skippable: false,
                commands: vec![],
            },
            RouterOsPlannedAction::ImportSshKey { remote_path } => PlannedAction {
                id: "routeros_import_key".to_string(),
                label: "Import SSH key".to_string(),
                description: format!(
                    "Import '{remote_path}' into `/user ssh-keys` for the 'uptrakit' user."
                ),
                security_impact: uptrakit_shared_types::Severity::Medium,
                default_enabled: true,
                skippable: false,
                commands: vec![format!(
                    "/user ssh-keys import public-key-file={remote_path} user=uptrakit"
                )],
            },
            RouterOsPlannedAction::DeletePublicKey { remote_path } => PlannedAction {
                id: "routeros_delete_key_file".to_string(),
                label: "Remove temporary key file".to_string(),
                description: format!(
                    "Delete '{remote_path}' from the RouterOS device after import."
                ),
                security_impact: uptrakit_shared_types::Severity::Low,
                default_enabled: true,
                skippable: true,
                commands: vec![],
            },
            RouterOsPlannedAction::VerifyTargetLogin => PlannedAction {
                id: "routeros_verify_target_login".to_string(),
                label: "Verify uptrakit user can log in".to_string(),
                description:
                    "Open a fresh SSH session as 'uptrakit' with the generated key to confirm \
                     the bootstrap actually configured the router."
                        .to_string(),
                security_impact: uptrakit_shared_types::Severity::Low,
                default_enabled: true,
                skippable: false,
                commands: vec![],
            },
            RouterOsPlannedAction::SaveHostEntry => PlannedAction {
                id: "routeros_save_host".to_string(),
                label: "Save host entry".to_string(),
                description: format!(
                    "Persist the RouterOS host '{}' (allow_reboot={}) to the local database.",
                    params.name, params.allow_reboot
                ),
                security_impact: uptrakit_shared_types::Severity::Low,
                default_enabled: true,
                skippable: false,
                commands: vec![],
            },
        })
        .collect()
}

// ── Shared input validation ──────────────────────────────────────────

/// Validate bootstrap parameters common to both connect and execute phases.
fn validate_bootstrap_inputs(params: &BootstrapParams) -> Result<()> {
    if params.strict_host_key_checking && params.host_key_fingerprint.is_none() {
        bail!(Error::InvalidInput(
            "--strict-host-key-checking requires --host-key-fingerprint to be provided".to_string()
        ));
    }

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

    Ok(())
}

// ── Connection setup ─────────────────────────────────────────────────

/// Successful output of [`prepare_bootstrap_connection`].
///
/// `use_sudo` is meaningful only when `host_os == HostOs::Posix`. On
/// `HostOs::RouterOs` it is always `false` (RouterOS has neither `id` nor
/// `sudo`) and must not be consulted by the caller.
struct PreparedBootstrapConnection {
    session: Arc<SshSession>,
    observed_fingerprint: String,
    executor: SshRemoteExecutor,
    use_sudo: bool,
    host_os: HostOs,
}

/// Establish the initial SSH connection, detect the remote host OS, and
/// verify sudo access on POSIX hosts.
async fn prepare_bootstrap_connection(
    params: &BootstrapParams,
    port: u16,
) -> Result<PreparedBootstrapConnection> {
    let auth = match (
        &params.auth_password,
        &params.auth_private_key_pem,
        params.use_ssh_agent,
    ) {
        (Some(password), _, _) => AuthMethod::Password(password.expose_secret()),
        (_, Some(pem), _) => AuthMethod::PrivateKey(pem.expose_secret()),
        (_, _, true) => AuthMethod::Agent,
        _ => bail!(Error::InvalidInput(
            "no authentication method available".to_string()
        )),
    };

    let config = SshConnectionConfig {
        hostname: params.hostname.clone(),
        port,
        connect_timeout: CONNECT_TIMEOUT,
    };

    tracing::info!(
        hostname = %params.hostname,
        port,
        auth_user = %params.auth_username,
        "connecting to remote host"
    );

    let (session, observed_fp) = ssh_transport::connect_and_authenticate(
        &config,
        &params.auth_username,
        &auth,
        params.host_key_fingerprint.as_deref(),
    )
    .await?;

    // Wrap in Arc so the session can be shared with the plugin executor used
    // for host-compatibility checks without copying any state.
    let session = Arc::new(session);

    if params.host_key_fingerprint.is_none() {
        tracing::debug!(fingerprint = %observed_fp, "accepted host key via TOFU");
    }

    // Build a RemoteExecutor for the sudoers/detection functions. Build the
    // SshCommandExecutor probe alongside it for OS detection — both wrap the
    // same Arc<SshSession>, no exclusivity issue.
    let executor = SshRemoteExecutor::new(Arc::clone(&session));
    let probe_exec = SshCommandExecutor::new(Arc::clone(&session));
    let host_os = detect_host_os(&session, &probe_exec).await?;
    drop(probe_exec);

    let use_sudo = evaluate_posix_sudo_gate(host_os, &executor, &params.auth_username).await?;

    Ok(PreparedBootstrapConnection {
        session,
        observed_fingerprint: observed_fp,
        executor,
        use_sudo,
        host_os,
    })
}

/// Decide whether bootstrap needs to prefix POSIX commands with `sudo`,
/// gating the check on host OS. RouterOS short-circuits to `false` (no
/// `id`/`sudo` exists). On POSIX, runs `id -u` and (if non-root) verifies
/// the auth user has passwordless sudo via `sudo -n -l`, surfacing the exit
/// code in any failure to aid sudoers debugging.
async fn evaluate_posix_sudo_gate(
    host_os: HostOs,
    executor: &dyn uptrakit_command::RemoteExecutor,
    auth_username: &str,
) -> Result<bool> {
    match host_os {
        HostOs::RouterOs => Ok(false),
        HostOs::Posix => {
            let is_root = detect_is_root(executor).await?;
            if is_root {
                return Ok(false);
            }
            let sudo_check = executor
                .exec_command("sudo -n -l")
                .await
                .context_to::<Error>()?;
            if sudo_check.exit_code != 0 {
                bail!(Error::SshCommand(format!(
                    "auth user '{auth_username}' does not have passwordless sudo access \
                     (exit code {}). Bootstrap requires sudo privileges on the remote host.",
                    sudo_check.exit_code
                )));
            }
            Ok(true)
        }
    }
}

// ── Authorized keys deployment ───────────────────────────────────────

/// Deploy the SSH public key into `authorized_keys` on the remote host.
///
/// Handles same-service key auto-removal, stale key cleanup (when
/// `remove_stale_keys` is set), and atomic rewrite vs. simple append.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the distinct pieces of key deployment state; extracting a struct would add boilerplate without clarity"
)]
async fn deploy_authorized_keys(
    session: &SshSession,
    home_dir: &str,
    target_public_openssh: &str,
    target_username: &str,
    host_id: uuid::Uuid,
    service_id: Option<&uuid::Uuid>,
    remove_stale_keys: bool,
    use_sudo: bool,
) -> Result<()> {
    tracing::info!("deploying SSH public key");

    // Build the authorized_keys comment.
    //
    // Format:
    //   uptrakit-host:<host_id>                          (no service UUID)
    //   uptrakit-svc:<service_id>-host:<host_id>         (service UUID known)
    let service_comment = match service_id {
        Some(svc_id) => format!("uptrakit-svc:{svc_id}-host:{host_id}"),
        None => format!("uptrakit-host:{host_id}"),
    };

    // Strip any trailing comment from the raw public key so we control the
    // comment field ourselves (first two whitespace-separated tokens only).
    let stripped_pubkey = target_public_openssh
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");

    // Read existing authorized_keys from remote.
    let read_cmd = cmd_read_authorized_keys(home_dir, use_sudo);
    let read_result = session.exec_command(&read_cmd).await?;
    let existing = parse_existing_authorized_keys(&read_result.stdout);

    // Compute auto-removal candidates: keys written by this service on
    // previous bootstrap runs.  Removed unconditionally when a service UUID
    // is available to keep authorized_keys clean across re-bootstraps.
    let same_service_lines: Vec<String> = service_id
        .map(|svc_id| {
            existing
                .all_key_lines
                .iter()
                .filter(|l| is_same_service_key_line(l, svc_id))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // Determine which additional lines are stale under --remove-stale-keys.
    let stale_lines = existing.uptrakit_key_lines.clone();

    // Log notice for auto-removed (same-service) keys.
    if !same_service_lines.is_empty() {
        tracing::info!(
            count = same_service_lines.len(),
            "removing key(s) written by this service on a previous bootstrap"
        );
        for line in &same_service_lines {
            tracing::debug!(key_line = %line, "removing same-service key");
        }
    }

    // Log notice for remaining stale keys not covered by auto-removal.
    let remaining_stale: Vec<&String> = stale_lines
        .iter()
        .filter(|l| !same_service_lines.contains(l))
        .collect();
    if !remaining_stale.is_empty() {
        tracing::info!(
            count = remaining_stale.len(),
            "found Uptrakit-managed key(s) in authorized_keys"
        );
        for line in &remaining_stale {
            tracing::debug!(key_line = %line, "stale Uptrakit key");
        }
        if !remove_stale_keys {
            tracing::info!("pass --remove-stale-keys to remove them before writing the new key");
        }
    }

    // Build the unified set of lines to remove:
    //   - always: same_service_lines
    //   - with --remove-stale-keys: stale_lines (superset)
    let mut to_remove: HashSet<&str> = same_service_lines.iter().map(String::as_str).collect();
    if remove_stale_keys {
        for l in &stale_lines {
            to_remove.insert(l.as_str());
        }
    }

    // Write the authorized_keys file.
    if to_remove.is_empty() {
        // No removals: append the new key entry (also creates .ssh if absent).
        let ak_cmd = cmd_setup_authorized_keys(
            home_dir,
            &stripped_pubkey,
            target_username,
            use_sudo,
            &service_comment,
        );
        let ak_result = session.exec_command(&ak_cmd).await?;
        if ak_result.exit_code != 0 {
            bail!(Error::SshCommand(format!(
                "failed to deploy authorized_keys: {}",
                ak_result.stderr.trim()
            )));
        }
    } else {
        // Removals required: rewrite the entire file atomically.
        if remove_stale_keys && !remaining_stale.is_empty() {
            tracing::info!(
                count = remaining_stale.len(),
                "removing stale key(s) from authorized_keys"
            );
        }

        let keep_lines: Vec<&str> = existing
            .all_key_lines
            .iter()
            .map(String::as_str)
            .filter(|l| !to_remove.contains(l))
            .collect();

        let restrictions = authorized_keys_restrictions();
        let new_entry = format!("{restrictions} {stripped_pubkey} {service_comment}");
        let new_content = if keep_lines.is_empty() {
            format!("{new_entry}\n")
        } else {
            format!("{}\n{new_entry}\n", keep_lines.join("\n"))
        };

        let write_cmd =
            cmd_write_authorized_keys(home_dir, &new_content, target_username, use_sudo);
        let write_result = session.exec_command(&write_cmd).await?;
        if write_result.exit_code != 0 {
            bail!(Error::SshCommand(format!(
                "failed to write authorized_keys: {}",
                write_result.stderr.trim()
            )));
        }
    }

    Ok(())
}

// ── Sudoers and plugin setup ─────────────────────────────────────────

/// Resolve plugin-specific sudoers commands on the remote host.
///
/// For each entry in `plugin_sudo_cmds`: helper scripts are SCP'd and their
/// install path is used directly; regular commands are resolved via
/// `command -v` and appended with optional argument suffix.
async fn resolve_plugin_sudo_commands(
    executor: &dyn uptrakit_command::RemoteExecutor,
    plugin_sudo_cmds: &[(uptrakit_shared_types::PluginTypeId, Vec<SudoCommandEntry>)],
    use_sudo: bool,
) -> Result<Vec<ResolvedSudoCommand>> {
    let mut resolved = Vec::new();
    for (_plugin_type, entries) in plugin_sudo_cmds {
        for entry in entries {
            if let Some(helper) = &entry.helper_script {
                // Install the helper script then use its known path directly.
                tracing::debug!(path = %helper.install_path, "installing helper script");
                install_helper_script(executor, helper, use_sudo).await?;
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

/// Run all infra plugins' `on_host_bootstrapped` and collect the results.
///
/// `provision_credentials` is forwarded to each plugin via
/// [`InfraPluginContext::provision_credentials`] — `false` when `pve_setup`
/// is in the caller's `skip_actions`.
async fn collect_infra_results(
    executor: &dyn uptrakit_command::RemoteExecutor,
    params: &BootstrapParams,
    db: &sea_orm::DatabaseConnection,
    state_dir: &std::path::Path,
    provision_credentials: bool,
) -> Result<Vec<BootstrapInfraResult>> {
    let catalog_config = CatalogConfig::default();
    let Ok(catalog) = build_catalog(
        &catalog_config,
        uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
    ) else {
        // Static config error, not a per-host transport failure — keep the
        // silent-empty semantics.
        return Ok(Vec::new());
    };
    let infra_bundles = catalog.create_infra_bundles(&catalog_config);
    let noop_invoker = NoopInfraActionInvoker;
    let noop_bootstrap = NoopGuestBootstrap;
    let tenant_id_str = params.tenant_id.map(|t| t.to_string());
    let infra_ctx = InfraPluginContext {
        db,
        tenant_id: tenant_id_str.as_deref(),
        service_id: params.service_id,
        state_dir,
        private_key_der: None,
        action_invoker: &noop_invoker,
        guest_bootstrap: &noop_bootstrap,
        provision_credentials,
    };
    let mut infra_results = Vec::new();
    for bundle in &infra_bundles {
        let Some(lifecycle) = bundle.lifecycle.as_ref() else {
            continue;
        };
        match lifecycle
            .on_host_bootstrapped(&infra_ctx, executor, params.host_id, &params.name)
            .await
        {
            Ok(result) => {
                if result.detected {
                    tracing::info!(plugin = %lifecycle.plugin_type_id(), "detected infrastructure");
                }
                infra_results.push(result);
            }
            Err(e) => {
                // Proxmox is currently the sole `HostLifecycle` implementor, and
                // the spec mandates loud failure over the previous best-effort
                // (silently-skip) semantics: a genuine transport error must not
                // be conflated with a verified "not this infra" result. If a
                // second implementor lands, revisit whether one plugin's
                // failure should still abort the whole infra pass.
                tracing::warn!(
                    error = %e,
                    plugin = %lifecycle.plugin_type_id(),
                    "infrastructure detection failed"
                );
                Err(e).context_to::<Error>()?;
            }
        }
    }
    Ok(infra_results)
}

/// Resolve plugin sudo commands, run infra plugin detection, merge any
/// infra-discovered sudo entries into the plugin-resolved set, and write the
/// sudoers file exactly once.
///
/// Returns `(sudoers_content, infra_results)` so the caller can verify sudo
/// grants and report infrastructure detections.
///
/// `skip_actions` drives two independent skips: `configure_sudoers` skips
/// sudo-command resolution and the sudoers write (infra detection still
/// runs, since sudo-command collection and host-state persistence must not
/// depend on it); `pve_setup` is forwarded to infra detection as
/// `!skip_pve`, gating only credential provisioning inside the plugin.
async fn setup_sudoers_and_plugins(
    executor: &dyn uptrakit_command::RemoteExecutor,
    params: &BootstrapParams,
    db: &DatabaseConnection,
    state_dir: &Path,
    use_sudo: bool,
    skip_actions: &HashSet<String>,
    plugin_sudo_cmds: Vec<(uptrakit_shared_types::PluginTypeId, Vec<SudoCommandEntry>)>,
) -> Result<(Option<SudoersContent>, Vec<BootstrapInfraResult>)> {
    // 7 params — below the too_many_arguments deny threshold; the two skip
    // flags travel inside skip_actions, derived here.
    let skip_sudoers = skip_actions.contains("configure_sudoers");
    let skip_pve = skip_actions.contains("pve_setup");

    // Infra first: detection + sudo collection always run; credentials
    // gated by skip_pve inside the plugin (InfraPluginContext flag).
    let infra_results = collect_infra_results(executor, params, db, state_dir, !skip_pve).await?;

    if skip_sudoers {
        return Ok((None, infra_results));
    }

    tracing::info!("configuring sudoers");
    let mut resolved = resolve_plugin_sudo_commands(executor, &plugin_sudo_cmds, use_sudo).await?;

    // Merge infra-contributed entries (pct exec / qm guest exec) BEFORE the
    // single write — a split that writes first and merges later (or never)
    // silently drops these grants.
    resolved.extend(
        infra_results
            .iter()
            .flat_map(|r| r.sudo_commands.iter())
            .map(|c| ResolvedSudoCommand {
                command_path: c.command_path.clone(),
                explanation: c.explanation.clone(),
                needs_setenv: c.needs_setenv,
            }),
    );

    let sudoers_content: Option<SudoersContent> = if !resolved.is_empty() {
        Some(SudoersContent::SpecificCommands(resolved))
    } else if params.allow_all {
        tracing::warn!("no plugin commands resolved; using NOPASSWD: ALL (--allow-all)");
        Some(SudoersContent::AllCommands)
    } else {
        tracing::warn!(
            "no plugin-specific commands found for this host; no sudoers file will be written"
        );
        None
    };

    if let Some(ref content) = sudoers_content {
        write_sudoers_file(executor, &params.target_username, content, use_sudo).await?;
    }

    Ok((sudoers_content, infra_results))
}

// ── Noop infra impls for bootstrap context ───────────────────────────

/// No-op [`InfraActionInvoker`] for bootstrap context.
///
/// `on_host_bootstrapped` implementations that don't need to invoke
/// controller-side actions can rely on this.
struct NoopInfraActionInvoker;

type InfraActionInvokeResult =
    std::result::Result<uptrakit_wire::surfaces::SurfaceActionResponse, InfraActionInvokeError>;

#[async_trait]
impl InfraActionInvoker for NoopInfraActionInvoker {
    async fn invoke(
        &self,
        _extension_id: &str,
        _action_id: &str,
        _params: serde_json::Value,
    ) -> InfraActionInvokeResult {
        Err(InfraActionInvokeError::from(
            "InfraActionInvoker not available during bootstrap",
        ))
    }
}

/// No-op [`GuestBootstrapExecutor`] for bootstrap context.
struct NoopGuestBootstrap;

#[async_trait]
impl GuestBootstrapExecutor for NoopGuestBootstrap {
    async fn bootstrap_guest(
        &self,
        _params: GuestBootstrapParams,
    ) -> std::result::Result<GuestBootstrapResult, GuestBootstrapError> {
        Err(GuestBootstrapError::from(
            "GuestBootstrapExecutor not available during bootstrap",
        ))
    }
}

// ── Verification ─────────────────────────────────────────────────────

/// Verify that the remote connection works as `target_username`.
///
/// Always checks `whoami`. When `has_sudo_grants` is `true`, additionally
/// verifies that `sudo -n -l` succeeds — confirming the written sudoers file
/// grants at least one NOPASSWD entry. When `false` (no sudoers file was
/// written) the sudo check is skipped, since there is nothing to verify.
async fn verify_remote(
    session: &SshSession,
    target_username: &str,
    has_sudo_grants: bool,
) -> Result<()> {
    // Verify whoami.
    let whoami = session.exec_command("whoami").await?;
    let actual_user = whoami.stdout.trim();
    if actual_user != target_username {
        bail!(Error::BootstrapVerification(format!(
            "whoami returned '{actual_user}', expected '{target_username}'. \
             The remote host has been partially configured."
        )));
    }

    if has_sudo_grants {
        // Verify sudo. We use `sudo -n -l` (list allowed commands without
        // prompting) because the sudoers file may only grant specific commands
        // (e.g. `/usr/bin/apt-get`), not the ability to run arbitrary
        // executables. `-n -l` exits 0 whenever the user has at least one
        // NOPASSWD entry, which is exactly what we want to confirm here.
        let sudo_check = session.exec_command("sudo -n -l").await?;
        if sudo_check.exit_code != 0 {
            bail!(Error::BootstrapVerification(format!(
                "sudo -n -l failed (exit code {}). Sudoers may not be \
                 configured correctly. The remote host has been partially configured.",
                sudo_check.exit_code
            )));
        }
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
    host_id: uuid::Uuid,
) -> Result<()> {
    let encrypted_key =
        EncryptedString::new(private_pem.to_string(), "uptrakit:ssh_hosts:private_key")
            .map_err(|e| report!(Error::Crypto(format!("failed to encrypt private key: {e}"))))?;

    host_ops::add_host(
        db,
        AddHostParams {
            host_id,
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
    let Some(first) = chars.next() else {
        bail!(Error::InvalidInput(
            "username must not be empty".to_string()
        ));
    };
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
        format!("sudo useradd --create-home --shell /bin/sh {escaped}")
    } else {
        format!("useradd --create-home --shell /bin/sh {escaped}")
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

/// SSH restrictions applied to `authorized_keys` entries.
///
/// Without the `interactive` feature, `no-pty` is included to prevent
/// interactive terminal allocation through the managed account. When the
/// `interactive` feature is enabled, `no-pty` is omitted so the controller
/// can request a PTY for interactive update sessions.
///
/// Both builds always restrict SSH agent and X11 forwarding.
pub(crate) fn authorized_keys_restrictions() -> &'static str {
    if cfg!(feature = "interactive") {
        "no-agent-forwarding,no-X11-forwarding"
    } else {
        "no-pty,no-agent-forwarding,no-X11-forwarding"
    }
}

/// Build a remote command that reads `authorized_keys`, tolerating a missing
/// file (returns empty output).
fn cmd_read_authorized_keys(home: &str, use_sudo: bool) -> String {
    let ak_path = format!("{home}/.ssh/authorized_keys");
    let escaped_ak_path = uptrakit_command::shell_escape(&ak_path);
    let sudo_prefix = if use_sudo { "sudo " } else { "" };
    format!("{sudo_prefix}cat {escaped_ak_path} 2>/dev/null || true")
}

/// Build a remote command that atomically overwrites `authorized_keys` with
/// `content` (already-formatted key lines + new entry) and fixes permissions.
fn cmd_write_authorized_keys(home: &str, content: &str, owner: &str, use_sudo: bool) -> String {
    let ak_path = format!("{home}/.ssh/authorized_keys");
    let escaped_ak_path = uptrakit_command::shell_escape(&ak_path);
    let escaped_content = uptrakit_command::shell_escape(content);
    let escaped_owner = uptrakit_command::shell_escape(owner);
    let sudo_prefix = if use_sudo { "sudo " } else { "" };
    format!(
        "printf '%s' {escaped_content} | {sudo_prefix}tee {escaped_ak_path} > /dev/null && \
         {sudo_prefix}chmod 600 {escaped_ak_path} && \
         {sudo_prefix}chown -R {escaped_owner}:{escaped_owner} {home}/.ssh"
    )
}

fn cmd_setup_authorized_keys(
    home: &str,
    pubkey: &str,
    owner: &str,
    use_sudo: bool,
    service_comment: &str,
) -> String {
    let escaped_home = uptrakit_command::shell_escape(home);
    let restrictions = authorized_keys_restrictions();
    let restricted_key = format!("{restrictions} {pubkey} {service_comment}");
    let escaped_pubkey = uptrakit_command::shell_escape(&restricted_key);
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

/// Returns `true` if the `authorized_keys` line looks like a key that was
/// written by Uptrakit.
///
/// Detection: the line is non-empty, does not start with `#`, and its last
/// whitespace-separated token starts with `uptrakit`.
fn is_uptrakit_key_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }
    trimmed
        .split_whitespace()
        .next_back()
        .map(|tok| tok.starts_with("uptrakit"))
        .unwrap_or(false)
}

/// Returns `true` if `line` is an Uptrakit key written by the service with
/// `service_id`.
///
/// Matches comments of the form `uptrakit-svc:<service_id>-host:<host_id>`.
/// Keys with other service UUIDs, plain `uptrakit-host:<host_id>` entries,
/// and non-Uptrakit keys all return `false`.
pub(crate) fn is_same_service_key_line(line: &str, service_id: &uuid::Uuid) -> bool {
    if !is_uptrakit_key_line(line) {
        return false;
    }
    let expected_prefix = format!("uptrakit-svc:{service_id}-host:");
    // `split_whitespace` already skips leading/trailing whitespace, so no
    // explicit `.trim()` is needed here.
    line.split_whitespace()
        .next_back()
        .map(|tok| tok.starts_with(&expected_prefix))
        .unwrap_or(false)
}

/// Classification of the current `authorized_keys` file content.
pub(crate) struct ExistingAuthorizedKeys {
    /// Non-blank, non-comment lines (actual key entries).
    pub(crate) all_key_lines: Vec<String>,
    /// Subset of `all_key_lines` that pass `is_uptrakit_key_line`.
    pub(crate) uptrakit_key_lines: Vec<String>,
}

/// Parse `authorized_keys` content into classified buckets.
pub(crate) fn parse_existing_authorized_keys(content: &str) -> ExistingAuthorizedKeys {
    let mut all_key_lines = Vec::new();
    let mut uptrakit_key_lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let owned = trimmed.to_string();
        if is_uptrakit_key_line(trimmed) {
            uptrakit_key_lines.push(owned.clone());
        }
        all_key_lines.push(owned);
    }

    ExistingAuthorizedKeys {
        all_key_lines,
        uptrakit_key_lines,
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_command::RemoteCommandResult;
    use uptrakit_command::test_support::ScriptedRemoteExecutor;

    use super::*;

    // ── HostOs compile test ──────────────────────────────────────────

    #[test]
    fn host_os_enum_variants_exist() {
        let _ = HostOs::Posix;
        let _ = HostOs::RouterOs;
    }

    // ── POSIX sudo gate tests ────────────────────────────────────────
    //
    // `ScriptedRemoteExecutor` here is the shared FIFO/matcher double from
    // `uptrakit_command::test_support` (feature `test-support`) — the
    // module-local duplicate that used to live here has been retired in
    // favor of it.

    fn script_result(stdout: &str, exit_code: u32) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code,
        }
    }

    #[tokio::test]
    async fn sudo_gate_router_os_skips_posix_checks() {
        // Empty script: any exec call would panic by default behaviour
        // returning a synthetic ok result, but recorded_calls will catch it.
        let executor = ScriptedRemoteExecutor::new([]);
        let use_sudo = evaluate_posix_sudo_gate(HostOs::RouterOs, &executor, "user1")
            .await
            .expect("RouterOS gate must succeed without invoking shell commands");
        assert!(!use_sudo, "use_sudo must be false on RouterOS");
        assert!(
            executor.recorded_calls().is_empty(),
            "no shell commands must run on RouterOS, recorded: {:?}",
            executor.recorded_calls()
        );
    }

    #[tokio::test]
    async fn sudo_gate_posix_root_skips_sudo_check() {
        let executor = ScriptedRemoteExecutor::new([script_result("0", 0)]);
        let use_sudo = evaluate_posix_sudo_gate(HostOs::Posix, &executor, "root")
            .await
            .expect("POSIX root must succeed");
        assert!(!use_sudo, "use_sudo must be false for root");
        assert_eq!(
            executor.recorded_calls(),
            vec!["id -u".to_string()],
            "only id -u must be issued for root"
        );
    }

    #[tokio::test]
    async fn sudo_gate_posix_with_sudo_passes() {
        let executor =
            ScriptedRemoteExecutor::new([script_result("1000", 0), script_result("", 0)]);
        let use_sudo = evaluate_posix_sudo_gate(HostOs::Posix, &executor, "deploy")
            .await
            .expect("POSIX with passwordless sudo must succeed");
        assert!(use_sudo, "use_sudo must be true for non-root POSIX");
        assert_eq!(
            executor.recorded_calls(),
            vec!["id -u".to_string(), "sudo -n -l".to_string()]
        );
    }

    #[tokio::test]
    async fn sudo_gate_posix_without_sudo_bails_with_exit_code() {
        let executor =
            ScriptedRemoteExecutor::new([script_result("1000", 0), script_result("", 1)]);
        let err = evaluate_posix_sudo_gate(HostOs::Posix, &executor, "user1")
            .await
            .expect_err("missing sudo must surface an error");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("'user1'"),
            "error must name the auth user, got: {msg}"
        );
        assert!(
            msg.contains("exit code 1"),
            "error must surface the sudo check exit code, got: {msg}"
        );
        assert_eq!(
            executor.recorded_calls(),
            vec!["id -u".to_string(), "sudo -n -l".to_string()]
        );
    }

    // ── setup_sudoers_and_plugins: skip semantics + single merged write ──

    /// PVE-positive script shared by `setup_sudoers_and_plugins` flow tests.
    ///
    /// `detect_pve_node` requires exit 0 AND non-empty stdout, so
    /// `command -v pveversion` must answer with a non-empty path — an
    /// empty-stdout exit-0 default would read as NOT detected and green
    /// every downstream assertion vacuously.
    fn pve_positive_script() -> ScriptedRemoteExecutor {
        ScriptedRemoteExecutor::with_matcher(vec![
            (
                "command -v pveversion",
                script_result("/usr/bin/pveversion", 0),
            ),
            ("hostname -s", script_result("pve1", 0)),
            ("test -f /usr/sbin/pct", script_result("", 0)),
            ("test -f /usr/sbin/qm", script_result("", 1)),
            ("command -v echo", script_result("/bin/echo", 0)),
            ("sudoers.d/uptrakit-", script_result("", 0)),
            ("visudo -cf", script_result("", 0)),
        ])
    }

    async fn test_db() -> DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        crate::db::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    fn test_bootstrap_params() -> BootstrapParams {
        BootstrapParams {
            name: "test-host".to_string(),
            hostname: "10.0.0.5".to_string(),
            port: 22,
            auth_username: "root".to_string(),
            auth_password: None,
            auth_private_key_pem: None,
            use_ssh_agent: false,
            target_username: "uptrakit".to_string(),
            target_private_key_pem: None,
            host_key_fingerprint: None,
            strict_host_key_checking: false,
            allow_all: false,
            host_id: uuid::Uuid::now_v7(),
            service_id: None,
            // Deliberately `None`: `create_or_reuse_pve_credentials`
            // short-circuits without a tenant_id, so these
            // `setup_sudoers_and_plugins`-level tests never need to script
            // `pveum` calls. `provision_credentials` itself is exercised
            // directly by the proxmox crate's `skip_pve_skips_only_credentials`.
            tenant_id: None,
            remove_stale_keys: false,
            allow_reboot: false,
        }
    }

    fn one_plugin_sudo_cmd() -> Vec<(uptrakit_shared_types::PluginTypeId, Vec<SudoCommandEntry>)> {
        vec![(
            uptrakit_shared_types::PluginTypeId::new("test.plugin"),
            vec![SudoCommandEntry::new("echo", "test command")],
        )]
    }

    #[tokio::test]
    async fn neither_skipped_merges_infra_sudo_entries() {
        let db = test_db().await;
        let executor = pve_positive_script();
        let params = test_bootstrap_params();
        let skip_actions: HashSet<String> = HashSet::new();

        let (sudoers_content, infra_results) = setup_sudoers_and_plugins(
            &executor,
            &params,
            &db,
            Path::new("."),
            true,
            &skip_actions,
            one_plugin_sudo_cmd(),
        )
        .await
        .expect("setup_sudoers_and_plugins succeeds against a PVE-positive script");

        let content =
            sudoers_content.expect("sudoers content must be written when entries resolve");
        let SudoersContent::SpecificCommands(entries) = content else {
            panic!("expected SpecificCommands sudoers content");
        };
        assert!(
            entries
                .iter()
                .any(|e| e.command_path == "/usr/sbin/pct exec *"),
            "infra-contributed pct entry must be merged into the single write: {:?}",
            entries.iter().map(|e| &e.command_path).collect::<Vec<_>>()
        );
        assert!(
            infra_results.iter().any(|r| r.detected),
            "PVE detection result must be present"
        );

        let calls = executor.recorded_calls();
        assert!(
            calls.iter().any(|c| c.contains("sudoers.d/uptrakit-")),
            "sudoers write command must be recorded: {calls:?}"
        );
    }

    #[tokio::test]
    async fn skip_pve_still_writes_merged_sudoers() {
        let db = test_db().await;
        let executor = pve_positive_script();
        // Unlike the shared fixture's `None`, this test overrides
        // `tenant_id` to a real value so the `pveum`-absence assertion below
        // is genuinely red-able: with `tenant_id: None`,
        // `create_or_reuse_pve_credentials` short-circuits before issuing
        // any `pveum` command regardless of `provision_credentials`, which
        // would leave the assertion green even if `!skip_pve` regressed to
        // a hardcoded `true` at the `collect_infra_results` call site.
        let mut params = test_bootstrap_params();
        params.tenant_id = Some(uuid::Uuid::now_v7());
        let skip_actions: HashSet<String> = ["pve_setup".to_string()].into_iter().collect();

        let (sudoers_content, _infra_results) = setup_sudoers_and_plugins(
            &executor,
            &params,
            &db,
            Path::new("."),
            true,
            &skip_actions,
            one_plugin_sudo_cmd(),
        )
        .await
        .expect("setup_sudoers_and_plugins succeeds with pve_setup skipped");

        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("pveum")),
            "credential provisioning must be skipped: {calls:?}"
        );
        assert!(
            calls.iter().any(|c| c.contains("sudoers.d/uptrakit-")),
            "sudoers write command must still be recorded when only pve_setup is skipped: {calls:?}"
        );

        let content = sudoers_content
            .expect("sudoers content must still be written when only pve_setup is skipped");
        let SudoersContent::SpecificCommands(entries) = content else {
            panic!("expected SpecificCommands sudoers content");
        };
        assert!(
            entries
                .iter()
                .any(|e| e.command_path == "/usr/sbin/pct exec *"),
            "infra-contributed pct entry must still be merged in: {:?}",
            entries.iter().map(|e| &e.command_path).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn skip_sudoers_writes_no_file() {
        let db = test_db().await;
        let executor = pve_positive_script();
        let params = test_bootstrap_params();
        let skip_actions: HashSet<String> = ["configure_sudoers".to_string()].into_iter().collect();

        let (sudoers_content, infra_results) = setup_sudoers_and_plugins(
            &executor,
            &params,
            &db,
            Path::new("."),
            true,
            &skip_actions,
            one_plugin_sudo_cmd(),
        )
        .await
        .expect("setup_sudoers_and_plugins succeeds with configure_sudoers skipped");

        assert!(
            sudoers_content.is_none(),
            "no sudoers content must be returned when configure_sudoers is skipped"
        );
        assert!(
            !infra_results.is_empty(),
            "infra detection must still run when only configure_sudoers is skipped"
        );

        let calls = executor.recorded_calls();
        assert!(
            !calls.iter().any(|c| c.contains("/etc/sudoers.d/")),
            "no sudoers file write command must be recorded: {calls:?}"
        );
    }

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
        assert_eq!(cmd, "sudo useradd --create-home --shell /bin/sh 'uptrakit'");
    }

    #[test]
    fn cmd_create_user_without_sudo() {
        let cmd = cmd_create_user("uptrakit", false);
        assert_eq!(cmd, "useradd --create-home --shell /bin/sh 'uptrakit'");
    }

    #[test]
    fn cmd_detect_home_with_sudo() {
        let cmd = cmd_detect_home("deploy", true);
        assert_eq!(cmd, "sudo getent passwd 'deploy' | cut -d: -f6");
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
            "ssh-ed25519 AAAA...",
            "deploy",
            true,
            "uptrakit",
        );
        assert!(cmd.contains("mkdir -p"));
        assert!(cmd.contains("chmod 700"));
        assert!(cmd.contains("tee -a"));
        assert!(cmd.contains("chmod 600"));
        assert!(cmd.contains("chown -R"));
        let restrictions = authorized_keys_restrictions();
        assert!(
            cmd.contains(restrictions),
            "authorized_keys must include restrictions: {cmd}"
        );
    }

    #[test]
    fn cmd_authorized_keys_includes_restrictions() {
        let cmd =
            cmd_setup_authorized_keys("/home/svc", "ssh-ed25519 AAAA...", "svc", false, "uptrakit");
        let expected = format!("{} ssh-ed25519", authorized_keys_restrictions());
        assert!(
            cmd.contains(&expected),
            "authorized_keys entry must include restriction prefix: {cmd}"
        );
    }

    #[test]
    fn comment_format_with_host_id_only() {
        let host_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let comment = format!("uptrakit-host:{host_id}");
        let cmd =
            cmd_setup_authorized_keys("/home/svc", "ssh-ed25519 AAAA...", "svc", false, &comment);
        assert!(
            cmd.contains(&comment),
            "authorized_keys entry must contain host comment: {cmd}"
        );
    }

    #[test]
    fn comment_format_with_service_and_host_id() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let host_id = uuid::Uuid::parse_str("018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b").unwrap();
        let comment = format!("uptrakit-svc:{svc_id}-host:{host_id}");
        let cmd =
            cmd_setup_authorized_keys("/home/svc", "ssh-ed25519 AAAA...", "svc", false, &comment);
        assert!(
            cmd.contains(&comment),
            "authorized_keys entry must contain svc+host comment: {cmd}"
        );
    }

    // ── is_uptrakit_key_line tests ───────────────────────────────────

    #[test]
    fn is_uptrakit_key_line_detects_plain_uptrakit() {
        assert!(is_uptrakit_key_line("no-pty ssh-ed25519 AAAA... uptrakit"));
    }

    #[test]
    fn is_uptrakit_key_line_detects_uptrakit_uuid() {
        assert!(is_uptrakit_key_line(
            "no-pty ssh-ed25519 AAAA... uptrakit-550e8400-e29b-41d4-a716-446655440000"
        ));
    }

    #[test]
    fn is_uptrakit_key_line_detects_host_id_comment() {
        assert!(is_uptrakit_key_line(
            "no-pty ssh-ed25519 AAAA... uptrakit-host:018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b"
        ));
    }

    #[test]
    fn is_uptrakit_key_line_detects_svc_and_host_id_comment() {
        assert!(is_uptrakit_key_line(
            "no-pty ssh-ed25519 AAAA... \
             uptrakit-svc:550e8400-e29b-41d4-a716-446655440000-host:018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b"
        ));
    }

    #[test]
    fn is_uptrakit_key_line_rejects_user_at_host() {
        assert!(!is_uptrakit_key_line("ssh-ed25519 AAAA... user@host"));
    }

    #[test]
    fn is_uptrakit_key_line_rejects_blank() {
        assert!(!is_uptrakit_key_line(""));
        assert!(!is_uptrakit_key_line("   "));
    }

    #[test]
    fn is_uptrakit_key_line_rejects_comment_line() {
        assert!(!is_uptrakit_key_line("# uptrakit"));
        assert!(!is_uptrakit_key_line("# ssh-ed25519 AAAA... uptrakit"));
    }

    // ── is_same_service_key_line tests ──────────────────────────────

    #[test]
    fn same_service_key_line_matches_correct_service_and_host() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let host_id = uuid::Uuid::parse_str("018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b").unwrap();
        let line = format!(
            "no-pty,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAA... \
             uptrakit-svc:{svc_id}-host:{host_id}"
        );
        assert!(is_same_service_key_line(&line, &svc_id));
    }

    #[test]
    fn same_service_key_line_rejects_different_service() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let other_svc = uuid::Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        let host_id = uuid::Uuid::parse_str("018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b").unwrap();
        let line = format!("no-pty ssh-ed25519 AAAA... uptrakit-svc:{other_svc}-host:{host_id}");
        assert!(!is_same_service_key_line(&line, &svc_id));
    }

    #[test]
    fn same_service_key_line_rejects_host_only_comment() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let host_id = uuid::Uuid::parse_str("018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b").unwrap();
        // uptrakit-host: format has no service prefix — must not match
        let line = format!("no-pty ssh-ed25519 AAAA... uptrakit-host:{host_id}");
        assert!(!is_same_service_key_line(&line, &svc_id));
    }

    #[test]
    fn same_service_key_line_rejects_non_uptrakit_key() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(!is_same_service_key_line(
            "ssh-ed25519 AAAA... user@host",
            &svc_id
        ));
    }

    #[test]
    fn same_service_key_line_rejects_blank() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(!is_same_service_key_line("", &svc_id));
        assert!(!is_same_service_key_line("   ", &svc_id));
    }

    #[test]
    fn same_service_key_line_rejects_comment_line() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let host_id = uuid::Uuid::parse_str("018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b").unwrap();
        let line = format!("# uptrakit-svc:{svc_id}-host:{host_id}");
        assert!(!is_same_service_key_line(&line, &svc_id));
    }

    #[test]
    fn same_service_key_line_matches_any_host_id_for_same_service() {
        let svc_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        for host_id in [
            "018d7f12-3e4a-7000-b1a9-4d8e6c0f2a1b",
            "11111111-2222-3333-4444-555555555555",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
        ] {
            let line = format!("no-pty ssh-ed25519 AAAA... uptrakit-svc:{svc_id}-host:{host_id}");
            assert!(
                is_same_service_key_line(&line, &svc_id),
                "should match host_id={host_id}"
            );
        }
    }

    // ── parse_existing_authorized_keys tests ────────────────────────

    #[test]
    fn parse_empty_file() {
        let result = parse_existing_authorized_keys("");
        assert!(result.all_key_lines.is_empty());
        assert!(result.uptrakit_key_lines.is_empty());
    }

    #[test]
    fn parse_no_uptrakit_keys() {
        let content = "ssh-ed25519 AAAA... user@host\nssh-rsa BBBB... admin@server\n";
        let result = parse_existing_authorized_keys(content);
        assert_eq!(result.all_key_lines.len(), 2);
        assert!(result.uptrakit_key_lines.is_empty());
    }

    #[test]
    fn parse_mixed_keys() {
        let content = "ssh-ed25519 AAAA... user@host\n\
                       no-pty ssh-ed25519 BBBB... uptrakit\n\
                       ssh-rsa CCCC... admin@server\n\
                       no-pty ssh-ed25519 DDDD... uptrakit-550e8400-e29b-41d4-a716-446655440000\n";
        let result = parse_existing_authorized_keys(content);
        assert_eq!(result.all_key_lines.len(), 4);
        assert_eq!(result.uptrakit_key_lines.len(), 2);
        assert!(
            result
                .uptrakit_key_lines
                .iter()
                .all(|l| l.ends_with("uptrakit") || l.contains("uptrakit-"))
        );
    }

    #[test]
    fn parse_comment_only_lines_ignored() {
        let content = "# This is a comment\n\
                       # uptrakit key\n\
                       ssh-ed25519 AAAA... uptrakit\n";
        let result = parse_existing_authorized_keys(content);
        assert_eq!(result.all_key_lines.len(), 1);
        assert_eq!(result.uptrakit_key_lines.len(), 1);
    }

    #[test]
    fn parse_blank_lines_ignored() {
        let content = "\n\n  \nssh-ed25519 AAAA... uptrakit\n\n";
        let result = parse_existing_authorized_keys(content);
        assert_eq!(result.all_key_lines.len(), 1);
        assert_eq!(result.uptrakit_key_lines.len(), 1);
    }

    // ── cmd_read_authorized_keys tests ──────────────────────────────

    #[test]
    fn cmd_read_authorized_keys_without_sudo() {
        let cmd = cmd_read_authorized_keys("/home/uptrakit", false);
        assert!(!cmd.contains("sudo"));
        assert!(cmd.contains("/home/uptrakit/.ssh/authorized_keys"));
        assert!(cmd.contains("2>/dev/null || true"));
    }

    #[test]
    fn cmd_read_authorized_keys_with_sudo() {
        let cmd = cmd_read_authorized_keys("/home/uptrakit", true);
        assert!(cmd.starts_with("sudo "));
        assert!(cmd.contains("/home/uptrakit/.ssh/authorized_keys"));
    }

    // ── cmd_write_authorized_keys tests ─────────────────────────────

    #[test]
    fn cmd_write_authorized_keys_structure() {
        let content = "no-pty ssh-ed25519 AAAA... uptrakit\n";
        let cmd = cmd_write_authorized_keys("/home/uptrakit", content, "uptrakit", true);
        assert!(cmd.contains("printf '%s'"));
        assert!(cmd.contains("sudo tee"));
        assert!(cmd.contains("/home/uptrakit/.ssh/authorized_keys"));
        assert!(cmd.contains("chmod 600"));
        assert!(cmd.contains("chown -R"));
    }

    #[test]
    fn cmd_write_authorized_keys_without_sudo() {
        let content = "no-pty ssh-ed25519 AAAA... uptrakit\n";
        let cmd = cmd_write_authorized_keys("/home/deploy", content, "deploy", false);
        assert!(!cmd.contains("sudo"));
        assert!(cmd.contains("tee"));
    }

    #[test]
    fn cmd_write_authorized_keys_escapes_content() {
        // Content with single-quotes should be safely escaped.
        let content = "no-pty ssh-ed25519 AAAA it's uptrakit\n";
        let cmd = cmd_write_authorized_keys("/home/deploy", content, "deploy", false);
        // shell_escape wraps in single quotes and escapes embedded single-quotes.
        assert!(
            !cmd.contains("it's"),
            "raw single quote must not appear unescaped: {cmd}"
        );
    }
}
