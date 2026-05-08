//! RouterOS bootstrap: user/group creation, SSH key upload, host entry persistence.

use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_crypto::EncryptedString;
use uptrakit_shared_types::SecretString;

use crate::db::entity::routeros_host_config;
use crate::db::entity::ssh_host::SshKeyType;
use crate::error::{Error, Result};
use crate::host_ops::{self, AddHostParams};
use crate::operations::bootstrap::CONNECT_TIMEOUT;
use crate::routeros_executor::RouterOsSshExecutor;
use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_key;
use crate::ssh_transport::{self, AuthMethod, SshConnectionConfig, SshSession};

/// Temporary file path on the RouterOS device for the public key upload.
const KEY_REMOTE_PATH: &str = "uptrakit-bootstrap.pub";

/// Parameters for the RouterOS bootstrap workflow.
#[expect(
    dead_code,
    reason = "auth credential fields (auth_username, auth_password, auth_private_key_pem, \
              use_ssh_agent, strict_host_key_checking) are stored for completeness but the \
              bootstrap execute path receives an already-established SshSession rather than \
              reconnecting with these credentials"
)]
pub(crate) struct RouterOsBootstrapParams {
    pub name: String,
    pub hostname: String,
    pub port: i32,
    pub auth_username: String,
    pub auth_password: Option<SecretString>,
    pub auth_private_key_pem: Option<SecretString>,
    pub use_ssh_agent: bool,
    pub host_key_fingerprint: Option<String>,
    pub strict_host_key_checking: bool,
    pub allow_reboot: bool,
    pub host_id: uuid::Uuid,
}

/// A single step in the RouterOS bootstrap sequence.
#[derive(Debug, Clone)]
pub(crate) enum RouterOsPlannedAction {
    /// Create the `uptrakit` group with the given comma-separated policy list.
    CreateGroup { policies: Vec<String> },
    /// Create the `uptrakit` user in the `uptrakit` group.
    CreateUser,
    /// Upload the generated public key to a temporary file on the device.
    UploadPublicKey { remote_path: String },
    /// Import the uploaded public key into `/user ssh-keys` for the `uptrakit` user.
    ImportSshKey { remote_path: String },
    /// Remove the temporary public-key file.
    DeletePublicKey { remote_path: String },
    /// Verify that the freshly-created `uptrakit` user can SSH in with the
    /// generated private key. Runs *before* `SaveHostEntry` so a botched key
    /// import or a missing `ssh` policy fails the bootstrap loudly rather than
    /// being discovered on the next operation.
    VerifyTargetLogin,
    /// Persist the host entry and RouterOS-specific config to the local database.
    SaveHostEntry,
}

/// Build the ordered plan for RouterOS bootstrap (pure, no I/O).
///
/// Always includes: create-group → create-user → upload-key → import-key →
/// delete-key → verify-target-login → save-entry. When `params.allow_reboot` is
/// `true`, the group policy list additionally includes `"reboot"`.
///
/// Policy list rationale:
/// - `ssh` — required for the `uptrakit` user to SSH in at all.
/// - `read` — query system state.
/// - `write` — apply changes including `/system package update install`.
/// - `test` — `ping`/`traceroute`/`bandwidth-test` used by health checks.
/// - `reboot` (conditional) — required to complete `package update install`,
///   which reboots to apply. Operator-controlled via `allow_reboot`.
pub(crate) fn plan_bootstrap_routeros(
    params: &RouterOsBootstrapParams,
) -> Vec<RouterOsPlannedAction> {
    let mut policies = vec![
        "ssh".to_string(),
        "read".to_string(),
        "write".to_string(),
        "test".to_string(),
    ];
    if params.allow_reboot {
        policies.push("reboot".to_string());
    }
    vec![
        RouterOsPlannedAction::CreateGroup { policies },
        RouterOsPlannedAction::CreateUser,
        RouterOsPlannedAction::UploadPublicKey {
            remote_path: KEY_REMOTE_PATH.to_string(),
        },
        RouterOsPlannedAction::ImportSshKey {
            remote_path: KEY_REMOTE_PATH.to_string(),
        },
        RouterOsPlannedAction::DeletePublicKey {
            remote_path: KEY_REMOTE_PATH.to_string(),
        },
        RouterOsPlannedAction::VerifyTargetLogin,
        RouterOsPlannedAction::SaveHostEntry,
    ]
}

/// Execute the RouterOS bootstrap plan.
///
/// Generates an Ed25519 key pair, executes each planned action in order,
/// and saves the host entry and RouterOS-specific config to the database.
pub(crate) async fn execute_bootstrap_routeros(
    params: &RouterOsBootstrapParams,
    session: Arc<SshSession>,
    db: &DatabaseConnection,
) -> Result<()> {
    let base_exec = SshCommandExecutor::new(Arc::clone(&session));
    let ros_exec = RouterOsSshExecutor::new(Arc::clone(&session));
    let plan = plan_bootstrap_routeros(params);

    // Generate an Ed25519 key pair in memory.  The private key (PEM) is
    // stored encrypted in the database; the public key is uploaded to the
    // RouterOS device and then deleted after import.
    let (private_key_pem, public_key_openssh) = ssh_key::generate_ed25519_keypair()?;
    let public_key_bytes = public_key_openssh.as_bytes();

    for action in &plan {
        match action {
            RouterOsPlannedAction::CreateGroup { policies } => {
                ros_exec
                    .create_group(&policies.join(","))
                    .await
                    .map_err(|e| {
                        report!(Error::SshCommand(format!("create RouterOS group: {e}")))
                    })?;
            }
            RouterOsPlannedAction::CreateUser => {
                ros_exec.create_user().await.map_err(|e| {
                    report!(Error::SshCommand(format!("create RouterOS user: {e}")))
                })?;
            }
            RouterOsPlannedAction::UploadPublicKey { remote_path } => {
                base_exec
                    .sftp_put(remote_path, public_key_bytes)
                    .await
                    .map_err(|e| {
                        report!(Error::SshCommand(format!("sftp_put '{remote_path}': {e}")))
                    })?;
            }
            RouterOsPlannedAction::ImportSshKey { remote_path } => {
                ros_exec.import_ssh_key(remote_path).await.map_err(|e| {
                    report!(Error::SshCommand(format!("import RouterOS ssh key: {e}")))
                })?;
            }
            RouterOsPlannedAction::DeletePublicKey { remote_path } => {
                // Best-effort: log on failure but do not abort the bootstrap.
                if let Err(e) = base_exec.sftp_remove(remote_path).await {
                    tracing::warn!(
                        remote_path,
                        error = %e,
                        "failed to delete temporary public key from RouterOS device"
                    );
                }
            }
            RouterOsPlannedAction::VerifyTargetLogin => {
                verify_routeros_uptrakit_login(params, &private_key_pem).await?;
            }
            RouterOsPlannedAction::SaveHostEntry => {
                save_routeros_host_entry(params, &private_key_pem, db).await?;
            }
        }
    }

    tracing::info!(
        host = %params.name,
        hostname = %params.hostname,
        port = params.port,
        host_id = %params.host_id,
        allow_reboot = params.allow_reboot,
        "RouterOS bootstrap complete"
    );

    Ok(())
}

/// Verify that the freshly-created `uptrakit` user can SSH in with the
/// generated private key. Opens a *separate* SSH session — RouterOS has a
/// low concurrent-session limit, so we drop the verify session immediately
/// after authentication succeeds.
async fn verify_routeros_uptrakit_login(
    params: &RouterOsBootstrapParams,
    private_key_pem: &str,
) -> Result<()> {
    #[expect(
        clippy::map_err_ignore,
        reason = "TryFromIntError carries no contextual information beyond what the formatted error message conveys"
    )]
    let port = u16::try_from(params.port).map_err(|_| {
        report!(Error::BootstrapVerification(format!(
            "invalid port {} for verify connection",
            params.port
        )))
    })?;

    // The auth-user connect captured the host-key fingerprint and stored it
    // on params (see `routeros_params_from_bootstrap_with_fp`). Verify pins
    // the connection to that exact fingerprint so we cannot silently TOFU a
    // different key.
    let expected_fp = params.host_key_fingerprint.as_deref().ok_or_else(|| {
        report!(Error::BootstrapVerification(
            "missing host key fingerprint for verify; the auth-user connect did not record one"
                .to_string()
        ))
    })?;

    let config = SshConnectionConfig {
        hostname: params.hostname.clone(),
        port,
        connect_timeout: CONNECT_TIMEOUT,
    };

    let (verify_session, _fp) = ssh_transport::connect_and_authenticate(
        &config,
        "uptrakit",
        &AuthMethod::PrivateKey(private_key_pem),
        Some(expected_fp),
    )
    .await
    .map_err(|e| {
        report!(Error::BootstrapVerification(format!(
            "uptrakit user was created on '{}' but cannot SSH in: {e}. \
             Likely causes: group policy missing `ssh`, the imported key did not register, \
             or a previous bootstrap attempt left an `uptrakit` user/group behind with a \
             different key. Clean up with `/user remove uptrakit; /user group remove uptrakit` \
             on the router before retrying.",
            params.hostname
        )))
    })?;

    // RouterOS has a low concurrent-session limit. Disconnect the verify
    // session now so subsequent operations don't trip "too many sessions".
    verify_session.disconnect().await;

    Ok(())
}

/// Persist the SSH host entry and RouterOS-specific config to the local database.
async fn save_routeros_host_entry(
    params: &RouterOsBootstrapParams,
    private_key_pem: &str,
    db: &DatabaseConnection,
) -> Result<()> {
    use sea_orm::ActiveValue::Set;
    use sea_orm::EntityTrait as _;

    let encrypted_key = EncryptedString::new(
        private_key_pem.to_string(),
        "uptrakit:ssh_hosts:private_key",
    )
    .map_err(|e| {
        report!(Error::Crypto(format!(
            "failed to encrypt RouterOS private key: {e}"
        )))
    })?;

    // Insert the base ssh_host row.  RouterOS devices use the `uptrakit`
    // account created during bootstrap, and the fingerprint will be verified
    // on first connection (TOFU or pre-supplied).
    host_ops::add_host(
        db,
        AddHostParams {
            host_id: params.host_id,
            name: params.name.clone(),
            hostname: params.hostname.clone(),
            port: params.port,
            username: "uptrakit".to_string(),
            encrypted_key,
            key_type: SshKeyType::Ed25519,
            host_key_fingerprint: params.host_key_fingerprint.clone(),
        },
    )
    .await?;

    // Insert the RouterOS-specific config row (FK → ssh_host.id).
    let config = routeros_host_config::ActiveModel {
        ssh_host_id: Set(params.host_id),
        allow_reboot: Set(params.allow_reboot),
    };
    routeros_host_config::Entity::insert(config)
        .exec(db)
        .await
        .map_err(|e| report!(Error::Database(e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_params() -> RouterOsBootstrapParams {
        RouterOsBootstrapParams {
            name: "test".to_string(),
            hostname: "192.168.1.1".to_string(),
            port: 22,
            auth_username: "admin".to_string(),
            auth_password: None,
            auth_private_key_pem: None,
            use_ssh_agent: false,
            host_key_fingerprint: None,
            strict_host_key_checking: false,
            allow_reboot: false,
            host_id: uuid::Uuid::nil(),
        }
    }

    /// Canonical RouterOS 7 `/user group` policy enum (per
    /// help.mikrotik.com/docs/spaces/ROS/pages/8978498/Console). Any policy
    /// emitted by `plan_bootstrap_routeros` MUST appear in this list — the
    /// router rejects the whole `policy=` argument as
    /// "input does not match any value of policy" otherwise.
    const VALID_ROUTEROS_7_POLICIES: &[&str] = &[
        "local",
        "telnet",
        "ssh",
        "ftp",
        "reboot",
        "read",
        "write",
        "policy",
        "test",
        "winbox",
        "password",
        "web",
        "sniff",
        "sensitive",
        "api",
        "romon",
        "dude",
        "tikapp",
        "rest-api",
    ];

    fn extracted_policies(plan: &[RouterOsPlannedAction]) -> Vec<String> {
        plan.iter()
            .find_map(|a| match a {
                RouterOsPlannedAction::CreateGroup { policies } => Some(policies.clone()),
                _ => None,
            })
            .expect("plan must contain CreateGroup")
    }

    #[test]
    fn plan_includes_reboot_policy_when_allowed() {
        let params = RouterOsBootstrapParams {
            allow_reboot: true,
            ..stub_params()
        };
        let plan = plan_bootstrap_routeros(&params);
        assert_eq!(
            extracted_policies(&plan),
            vec!["ssh", "read", "write", "test", "reboot"]
        );
    }

    #[test]
    fn plan_excludes_reboot_policy_when_not_allowed() {
        let plan = plan_bootstrap_routeros(&stub_params());
        assert!(!extracted_policies(&plan).iter().any(|p| p == "reboot"));
    }

    #[test]
    fn plan_upload_precedes_import_precedes_delete() {
        use std::mem::discriminant;
        let plan = plan_bootstrap_routeros(&stub_params());
        let ds: Vec<_> = plan.iter().map(discriminant).collect();
        let upload = ds
            .iter()
            .position(|&d| {
                d == discriminant(&RouterOsPlannedAction::UploadPublicKey {
                    remote_path: String::new(),
                })
            })
            .unwrap();
        let import = ds
            .iter()
            .position(|&d| {
                d == discriminant(&RouterOsPlannedAction::ImportSshKey {
                    remote_path: String::new(),
                })
            })
            .unwrap();
        let delete = ds
            .iter()
            .position(|&d| {
                d == discriminant(&RouterOsPlannedAction::DeletePublicKey {
                    remote_path: String::new(),
                })
            })
            .unwrap();
        assert!(upload < import && import < delete);
    }

    #[test]
    fn plan_default_policies_are_ssh_read_write_test() {
        let plan = plan_bootstrap_routeros(&stub_params());
        assert_eq!(
            extracted_policies(&plan),
            vec!["ssh", "read", "write", "test"],
            "default policy list (allow_reboot=false) must be exactly the \
             RouterOS 7 enum values needed for SSH login + read/write + test"
        );
    }

    #[test]
    fn plan_policies_are_all_valid_routeros_7_values() {
        // Default and reboot variants both must use only canonical enum values.
        for params in [
            stub_params(),
            RouterOsBootstrapParams {
                allow_reboot: true,
                ..stub_params()
            },
        ] {
            let plan = plan_bootstrap_routeros(&params);
            for policy in extracted_policies(&plan) {
                assert!(
                    VALID_ROUTEROS_7_POLICIES.contains(&policy.as_str()),
                    "policy '{policy}' is not a valid RouterOS 7 enum value; \
                     router will reject the whole `/user group add` command"
                );
            }
        }
    }

    #[test]
    fn plan_verify_precedes_save_entry() {
        use std::mem::discriminant;
        let plan = plan_bootstrap_routeros(&stub_params());
        let ds: Vec<_> = plan.iter().map(discriminant).collect();
        let verify = ds
            .iter()
            .position(|&d| d == discriminant(&RouterOsPlannedAction::VerifyTargetLogin))
            .expect("plan must contain VerifyTargetLogin");
        let save = ds
            .iter()
            .position(|&d| d == discriminant(&RouterOsPlannedAction::SaveHostEntry))
            .expect("plan must contain SaveHostEntry");
        assert!(
            verify < save,
            "verify must run before save — saving a host that cannot be \
             reached as 'uptrakit' would persist a broken entry"
        );
    }
}
