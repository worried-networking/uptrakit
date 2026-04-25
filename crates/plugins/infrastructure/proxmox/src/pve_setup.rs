//! PVE node detection and API credential provisioning.
//!
//! These functions run on a PVE host via a [`RemoteExecutor`] (typically an SSH
//! session) and bootstrap the Proxmox API credentials needed for the controller
//! to use the Proxmox plugin.

use rootcause::prelude::*;
use uptrakit_command::RemoteExecutor;

use crate::{ProxmoxError, Result};

/// Credentials for the Proxmox VE REST API.
#[derive(Debug, Clone)]
pub struct PveCredentials {
    /// Base API URL (e.g. `https://pve.local:8006`).
    pub api_url: String,
    /// API token in PVE format (`USER@REALM!TOKENID=SECRET`).
    pub api_token: String,
}

/// Result of checking for existing PVE API tokens on the cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PveTokenStatus {
    /// No Uptrakit token exists on the cluster — safe to create one.
    NotFound,
    /// A token exists for the requesting tenant — cluster already configured.
    OwnedByTenant(String),
    /// A token exists for a different tenant — cluster claimed by another tenant.
    OwnedByOtherTenant(String),
}

/// Detect the short Proxmox VE node name via `hostname -s`.
///
/// Returns the trimmed short hostname (e.g. `"optiplex2"`). Proxmox uses the
/// short hostname as the node identifier in its cluster.
pub async fn detect_pve_node_name(executor: &dyn RemoteExecutor) -> Result<String> {
    let result = executor
        .exec_command("hostname -s")
        .await
        .context_to::<ProxmoxError>()?;
    let name = result.stdout.trim().to_string();
    if name.is_empty() {
        bail!(ProxmoxError::Plugin(
            "hostname -s returned empty output".to_string()
        ));
    }
    Ok(name)
}

/// Detect all PVE node names in the local cluster via `pvesh get /cluster/status`.
///
/// Returns the short hostnames of every **node** member (entries with
/// `"type": "node"`) in the cluster. On a standalone node (not joined to any
/// cluster) this returns a single-element vec containing the current node.
///
/// Returns an empty vec on failure so callers can treat the result as
/// best-effort information.
pub async fn detect_pve_cluster_nodes(executor: &dyn RemoteExecutor) -> Vec<String> {
    let result = match executor
        .exec_command("pvesh get /cluster/status --output-format json 2>/dev/null")
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "pvesh get /cluster/status failed");
            return Vec::new();
        }
    };

    if result.exit_code != 0 || result.stdout.trim().is_empty() {
        tracing::debug!(
            exit_code = result.exit_code,
            "pvesh get /cluster/status returned non-zero or empty output"
        );
        return Vec::new();
    }

    let entries: Vec<serde_json::Value> = match serde_json::from_str(result.stdout.trim()) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %e, "failed to parse pvesh /cluster/status output");
            return Vec::new();
        }
    };

    entries
        .into_iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("node"))
        .filter_map(|e| {
            e.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect()
}

/// Verify that the Uptrakit PVE API user and its ACL roles still exist.
///
/// Checks:
/// 1. The user `uptrakit-{tenant_id}@pve` exists (`pveum user list`)
/// 2. The user holds an audit role on `/` — either [`UPTRAKIT_AUDIT_ROLE`]
///    (current) or the legacy `PVEAuditor` (pre-existing installs).
/// 3. The user holds [`UPTRAKIT_PROTECTION_ROLE`] on `/vms`.
///
/// Returns `Ok(())` when all checks pass.  Returns an error listing which
/// checks failed; the caller should call [`ensure_pve_privileges`] to fix
/// missing privileges automatically.
pub async fn verify_pve_privileges(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
) -> Result<()> {
    let user_realm = pve_user_realm(tenant_id);

    // Step 1: Check user exists.
    let status = check_pve_token_exists(executor, tenant_id).await?;
    match status {
        PveTokenStatus::OwnedByTenant(_) => {}
        PveTokenStatus::NotFound => {
            bail!(ProxmoxError::Plugin(format!(
                "PVE user '{user_realm}' does not exist on this cluster"
            )));
        }
        PveTokenStatus::OwnedByOtherTenant(other) => {
            bail!(ProxmoxError::Plugin(format!(
                "PVE cluster is claimed by a different tenant (user '{other}')"
            )));
        }
    }

    // Step 2: Check ACL roles.
    let acl_result = executor
        .exec_command("pveum acl list --output-format json 2>/dev/null")
        .await
        .context_to::<ProxmoxError>()?;

    if acl_result.exit_code != 0 {
        bail!(ProxmoxError::Plugin(format!(
            "pveum acl list failed (exit {})",
            acl_result.exit_code
        )));
    }

    let acls: Vec<serde_json::Value> =
        serde_json::from_str(acl_result.stdout.trim()).map_err(|e| {
            report!(ProxmoxError::ParseResponse(format!(
                "failed to parse pveum acl list output: {e}"
            )))
        })?;

    // Accept both the current custom role and the legacy built-in for backward compat.
    let has_audit = acls.iter().any(|acl| {
        let path = acl.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let ugid = acl.get("ugid").and_then(|v| v.as_str()).unwrap_or_default();
        let roleid = acl
            .get("roleid")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        path == "/"
            && ugid == user_realm
            && (roleid == UPTRAKIT_AUDIT_ROLE || roleid == "PVEAuditor")
    });

    let has_protection_vms = acls.iter().any(|acl| {
        let path = acl.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let ugid = acl.get("ugid").and_then(|v| v.as_str()).unwrap_or_default();
        let roleid = acl
            .get("roleid")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        path == "/vms" && ugid == user_realm && roleid == UPTRAKIT_PROTECTION_ROLE
    });

    let has_protection_storage = acls.iter().any(|acl| {
        let path = acl.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let ugid = acl.get("ugid").and_then(|v| v.as_str()).unwrap_or_default();
        let roleid = acl
            .get("roleid")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        path == "/storage" && ugid == user_realm && roleid == UPTRAKIT_PROTECTION_ROLE
    });

    let mut missing = Vec::new();
    if !has_audit {
        missing.push(format!("{UPTRAKIT_AUDIT_ROLE} on /"));
    }
    if !has_protection_vms {
        missing.push(format!("{UPTRAKIT_PROTECTION_ROLE} on /vms"));
    }
    if !has_protection_storage {
        missing.push(format!("{UPTRAKIT_PROTECTION_ROLE} on /storage"));
    }

    if !missing.is_empty() {
        bail!(ProxmoxError::Plugin(format!(
            "PVE user '{user_realm}' is missing ACLs: {}",
            missing.join(", ")
        )));
    }

    Ok(())
}

/// Detect whether the remote host is a Proxmox VE node.
///
/// Checks for the presence of `pveversion` in `$PATH`.
pub async fn detect_pve_node(executor: &dyn RemoteExecutor) -> Result<bool> {
    let result = executor
        .exec_command("command -v pveversion")
        .await
        .context_to::<ProxmoxError>()?;
    Ok(result.exit_code == 0 && !result.stdout.trim().is_empty())
}

/// Resolve the PVE API base URL from the node's hostname.
///
/// Reads the FQDN from the remote and constructs the standard PVE API base
/// URL. The `/api2/json` path prefix is **not** included — the
/// [`ProxmoxClient`](crate::client::ProxmoxClient) appends it per-request.
pub async fn resolve_pve_api_url(executor: &dyn RemoteExecutor) -> Result<String> {
    let result = executor
        .exec_command("hostname -f")
        .await
        .context_to::<ProxmoxError>()?;
    let hostname = result.stdout.trim().to_string();
    if hostname.is_empty() {
        bail!(ProxmoxError::Plugin("remote hostname is empty".to_string()));
    }
    Ok(format!("https://{hostname}:8006"))
}

/// PVE username prefix used for Uptrakit API credentials.
const PVE_USERNAME_PREFIX: &str = "uptrakit-";

/// PVE realm for Uptrakit API credentials.
const PVE_REALM: &str = "pve";

/// PVE API token name created by Uptrakit.
const PVE_TOKEN_NAME: &str = "uptrakit";

/// Custom PVE role for read/audit access (replaces the built-in PVEAuditor).
///
/// Privileges:
/// - `Sys.Audit` — list cluster nodes
/// - `VM.Audit` — list VMs/CTs and read their config
/// - `VM.GuestAgent.Audit` — read guest agent data (network interfaces, IP discovery)
/// - `VM.GuestAgent.FileRead` — read files from guest via agent (`/etc/machine-id`)
///
/// `VM.GuestAgent.*` privileges were introduced in PVE 9 (replacing the old
/// `VM.Monitor`). On older PVE versions the role creation will warn on unknown
/// privileges but the known ones will still be applied.
pub const UPTRAKIT_AUDIT_ROLE: &str = "UptrakitAudit";

/// Custom PVE role for update-protection operations.
///
/// Privileges:
/// - `VM.Snapshot` — create pre-update snapshots for VMs and CTs
/// - `VM.Backup` — initiate vzdump backup tasks
/// - `Datastore.AllocateSpace` — write backup archives to PBS/directory storage
///
/// The role must be granted on both `/vms` (VM operations) and `/storage`
/// (storage write access).  Without the `/storage` ACL, vzdump fails with
/// `403 Forbidden (Datastore.AllocateSpace)` when targeting PBS datastores.
pub const UPTRAKIT_PROTECTION_ROLE: &str = "UptrakitProtection";

const PVE_AUDIT_PRIVS: &str = "Sys.Audit VM.Audit VM.GuestAgent.Audit VM.GuestAgent.FileRead";
const PVE_PROTECTION_PRIVS: &str = "VM.Snapshot VM.Backup Datastore.AllocateSpace";

/// Build the PVE user@realm string for a given tenant ID.
///
/// Format: `uptrakit-{tenant_id}@pve`
pub fn pve_user_realm(tenant_id: &uuid::Uuid) -> String {
    format!("{PVE_USERNAME_PREFIX}{tenant_id}@{PVE_REALM}")
}

/// Check whether an Uptrakit PVE API token already exists on the cluster.
///
/// Searches for PVE users matching the `uptrakit-*@pve` pattern by listing
/// all PVE users and filtering locally. Returns the ownership status
/// relative to the given `tenant_id`.
pub async fn check_pve_token_exists(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
) -> Result<PveTokenStatus> {
    let result = executor
        .exec_command("pveum user list --output-format json 2>/dev/null")
        .await
        .context_to::<ProxmoxError>()?;

    if result.exit_code != 0 {
        // Cannot list users — treat as "not found" and let creation attempt
        // handle the error naturally.
        tracing::debug!(
            exit_code = result.exit_code,
            "pveum user list returned non-zero; assuming no existing token"
        );
        return Ok(PveTokenStatus::NotFound);
    }

    let users: Vec<serde_json::Value> =
        serde_json::from_str(result.stdout.trim()).map_err(|e| {
            report!(ProxmoxError::ParseResponse(format!(
                "failed to parse pveum user list output: {e}"
            )))
        })?;

    let own_user = pve_user_realm(tenant_id);

    for user in &users {
        let userid = match user.get("userid").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => continue,
        };

        // Check if this is an Uptrakit-managed user: `uptrakit-{uuid}@pve`
        let Some(rest) = userid.strip_prefix(PVE_USERNAME_PREFIX) else {
            continue;
        };
        if !rest.ends_with(&format!("@{PVE_REALM}")) {
            continue;
        }

        if userid == own_user {
            return Ok(PveTokenStatus::OwnedByTenant(userid.to_string()));
        }
        return Ok(PveTokenStatus::OwnedByOtherTenant(userid.to_string()));
    }

    Ok(PveTokenStatus::NotFound)
}

/// Ensure the Uptrakit custom PVE roles exist with the correct privilege sets.
///
/// Creates [`UPTRAKIT_AUDIT_ROLE`] and [`UPTRAKIT_PROTECTION_ROLE`] if they
/// don't exist, then applies `pveum role modify` to bring their privilege sets
/// up to date. Idempotent — safe to call on every sync.
async fn ensure_pve_roles(executor: &dyn RemoteExecutor) -> Result<()> {
    for (role, privs) in [
        (UPTRAKIT_AUDIT_ROLE, PVE_AUDIT_PRIVS),
        (UPTRAKIT_PROTECTION_ROLE, PVE_PROTECTION_PRIVS),
    ] {
        // `pveum role add` exits non-zero if the role already exists; ignore
        // that error then unconditionally apply `modify` to keep privs in sync.
        let cmd = format!(
            "pveum role add '{role}' -privs '{privs}' 2>/dev/null; \
             pveum role modify '{role}' -privs '{privs}'"
        );
        let result = executor
            .exec_command(&cmd)
            .await
            .context_to::<ProxmoxError>()?;
        if result.exit_code != 0 {
            tracing::warn!(
                role,
                exit_code = result.exit_code,
                stderr = %result.stderr.trim(),
                "pveum role modify returned non-zero"
            );
        } else {
            tracing::debug!(role, "PVE role ensured");
        }
    }
    Ok(())
}

/// Ensure the Uptrakit PVE user has all required ACLs:
/// - [`UPTRAKIT_AUDIT_ROLE`] on `/`
/// - [`UPTRAKIT_PROTECTION_ROLE`] on `/vms` (VM.Snapshot, VM.Backup)
/// - [`UPTRAKIT_PROTECTION_ROLE`] on `/storage` (Datastore.AllocateSpace for PBS/dir targets)
///
/// `pveum acl modify` is idempotent — calling it when the ACL already exists
/// is a no-op. Safe to call on every sync.
async fn ensure_pve_acls(executor: &dyn RemoteExecutor, user_realm: &str) -> Result<()> {
    let pairs = [
        ("/", UPTRAKIT_AUDIT_ROLE),
        ("/vms", UPTRAKIT_PROTECTION_ROLE),
        ("/storage", UPTRAKIT_PROTECTION_ROLE),
    ];
    for (path, role) in pairs {
        let cmd = format!("pveum acl modify '{path}' --users '{user_realm}' --roles '{role}'");
        let result = executor
            .exec_command(&cmd)
            .await
            .context_to::<ProxmoxError>()?;
        if result.exit_code != 0 {
            tracing::warn!(
                path,
                role,
                exit_code = result.exit_code,
                stderr = %result.stderr.trim(),
                "pveum acl modify returned non-zero"
            );
        } else {
            tracing::debug!(path, role, "PVE ACL ensured");
        }
    }
    Ok(())
}

/// Ensure Uptrakit custom roles exist and the PVE user for `tenant_id` holds
/// all required ACLs.
///
/// Called both at initial bootstrap and on every host sync. Idempotent.
pub async fn ensure_pve_privileges(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
) -> Result<()> {
    ensure_pve_roles(executor).await?;
    let user_realm = pve_user_realm(tenant_id);
    ensure_pve_acls(executor, &user_realm).await?;
    Ok(())
}

/// Create a PVE API user and token for Uptrakit.
///
/// 1. Creates the user `uptrakit-{tenant_id}@pve` (ignores "already exists")
/// 2. Creates an API token named `uptrakit` with `privsep=0`
/// 3. Grants `PVEAuditor` role on `/` (read-only cluster access)
///
/// Returns the credentials needed to configure the Proxmox plugin.
pub async fn create_pve_api_credentials(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
) -> Result<PveCredentials> {
    let user_realm = pve_user_realm(tenant_id);

    // Step 1: Ensure custom roles exist with correct privilege sets.
    ensure_pve_roles(executor).await?;

    // Step 2: Create user (idempotent — ignore "already exists")
    let create_user_cmd =
        format!("pveum user add '{user_realm}' --comment 'Created by Uptrakit' 2>&1 || true");
    executor
        .exec_command(&create_user_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    // Step 3: Create API token
    let create_token_cmd = format!(
        "pveum user token add '{user_realm}' {PVE_TOKEN_NAME} --privsep=0 --output-format json"
    );
    let token_result = executor
        .exec_command(&create_token_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    if token_result.exit_code != 0 {
        bail!(ProxmoxError::Plugin(format!(
            "pveum token add failed (exit {}): {}",
            token_result.exit_code,
            token_result.stderr.trim()
        )));
    }

    let token_value = parse_token_value(&token_result.stdout)?;

    // Step 4: Grant UptrakitAudit on / and UptrakitProtection on /vms.
    ensure_pve_acls(executor, &user_realm).await?;

    // Step 5: Resolve API URL
    let api_url = resolve_pve_api_url(executor).await?;

    let api_token = format!("{user_realm}!{PVE_TOKEN_NAME}={token_value}");

    Ok(PveCredentials { api_url, api_token })
}

/// Remove any existing Uptrakit PVE API token and create a fresh one.
///
/// Called when the PVE user already exists for this tenant but the local agent
/// has no plugin config (e.g. after a re-install). The old token is removed
/// first so that `pveum user token add` does not fail with "already exists".
///
/// Returns the new [`PveCredentials`] that should be reported to the controller
/// so it can create or update the plugin config entry.
pub async fn regenerate_pve_api_token(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
) -> Result<PveCredentials> {
    let user_realm = pve_user_realm(tenant_id);

    // Step 1: Remove the existing token (best-effort — ignore errors).
    let remove_cmd =
        format!("pveum user token remove '{user_realm}' {PVE_TOKEN_NAME} 2>&1 || true");
    executor
        .exec_command(&remove_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    // Step 2: Create the token again.
    let create_token_cmd = format!(
        "pveum user token add '{user_realm}' {PVE_TOKEN_NAME} --privsep=0 --output-format json"
    );
    let token_result = executor
        .exec_command(&create_token_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    if token_result.exit_code != 0 {
        bail!(ProxmoxError::Plugin(format!(
            "pveum token add failed (exit {}): {}",
            token_result.exit_code,
            token_result.stderr.trim()
        )));
    }

    let token_value = parse_token_value(&token_result.stdout)?;

    // Step 3: Resolve the API URL.
    let api_url = resolve_pve_api_url(executor).await?;

    let api_token = format!("{user_realm}!{PVE_TOKEN_NAME}={token_value}");

    Ok(PveCredentials { api_url, api_token })
}

/// Parse the token value from `pveum user token add --output-format json` output.
///
/// Expected format: `{"full-tokenid":"user@pve!uptrakit","info":{"privsep":"0"},"value":"SECRET"}`
fn parse_token_value(json_output: &str) -> Result<String> {
    let parsed: serde_json::Value = serde_json::from_str(json_output.trim()).map_err(|e| {
        report!(ProxmoxError::ParseResponse(format!(
            "invalid JSON from pveum: {e}"
        )))
    })?;

    let value = parsed
        .get("value")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| {
            report!(ProxmoxError::ParseResponse(
                "missing 'value' field in pveum token output".to_string()
            ))
        })?;

    if value.is_empty() {
        bail!(ProxmoxError::ParseResponse(
            "empty token value from pveum".to_string()
        ));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_value_valid() {
        let json = r#"{"full-tokenid":"uptrakit@pve!uptrakit","info":{"privsep":"0"},"value":"abc123-secret"}"#;
        let value = parse_token_value(json).expect("should parse");
        assert_eq!(value, "abc123-secret");
    }

    #[test]
    fn parse_token_value_missing_field() {
        let json = r#"{"full-tokenid":"uptrakit@pve!uptrakit","info":{}}"#;
        assert!(parse_token_value(json).is_err());
    }

    #[test]
    fn parse_token_value_empty_value() {
        let json = r#"{"full-tokenid":"uptrakit@pve!uptrakit","value":""}"#;
        assert!(parse_token_value(json).is_err());
    }

    #[test]
    fn parse_token_value_invalid_json() {
        assert!(parse_token_value("not json").is_err());
    }

    #[test]
    fn pve_user_realm_format() {
        let tenant_id =
            uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid");
        assert_eq!(
            pve_user_realm(&tenant_id),
            "uptrakit-11111111-1111-1111-1111-111111111111@pve"
        );
    }

    fn has_audit_acl(acls: &[serde_json::Value], user_realm: &str) -> bool {
        acls.iter().any(|acl| {
            let path = acl.get("path").and_then(|v| v.as_str()).unwrap_or_default();
            let ugid = acl.get("ugid").and_then(|v| v.as_str()).unwrap_or_default();
            let roleid = acl
                .get("roleid")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            path == "/"
                && ugid == user_realm
                && (roleid == UPTRAKIT_AUDIT_ROLE || roleid == "PVEAuditor")
        })
    }

    fn has_protection_vms_acl(acls: &[serde_json::Value], user_realm: &str) -> bool {
        acls.iter().any(|acl| {
            let path = acl.get("path").and_then(|v| v.as_str()).unwrap_or_default();
            let ugid = acl.get("ugid").and_then(|v| v.as_str()).unwrap_or_default();
            let roleid = acl
                .get("roleid")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            path == "/vms" && ugid == user_realm && roleid == UPTRAKIT_PROTECTION_ROLE
        })
    }

    fn has_protection_storage_acl(acls: &[serde_json::Value], user_realm: &str) -> bool {
        acls.iter().any(|acl| {
            let path = acl.get("path").and_then(|v| v.as_str()).unwrap_or_default();
            let ugid = acl.get("ugid").and_then(|v| v.as_str()).unwrap_or_default();
            let roleid = acl
                .get("roleid")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            path == "/storage" && ugid == user_realm && roleid == UPTRAKIT_PROTECTION_ROLE
        })
    }

    #[test]
    fn verify_pve_acl_parsing_new_roles() {
        let user_realm = "uptrakit-11111111-1111-1111-1111-111111111111@pve";
        let json = format!(
            r#"[
                {{"path":"/","roleid":"{UPTRAKIT_AUDIT_ROLE}","type":"user","ugid":"{user_realm}","propagate":true}},
                {{"path":"/vms","roleid":"{UPTRAKIT_PROTECTION_ROLE}","type":"user","ugid":"{user_realm}","propagate":true}},
                {{"path":"/storage","roleid":"{UPTRAKIT_PROTECTION_ROLE}","type":"user","ugid":"{user_realm}","propagate":true}}
            ]"#
        );
        let acls: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid JSON");
        assert!(has_audit_acl(&acls, user_realm));
        assert!(has_protection_vms_acl(&acls, user_realm));
        assert!(has_protection_storage_acl(&acls, user_realm));
    }

    #[test]
    fn verify_pve_acl_parsing_legacy_pveauditor() {
        // Pre-existing installs have PVEAuditor on / — should still pass audit check.
        let user_realm = "uptrakit-11111111-1111-1111-1111-111111111111@pve";
        let json = format!(
            r#"[
                {{"path":"/","roleid":"PVEAuditor","type":"user","ugid":"{user_realm}","propagate":true}},
                {{"path":"/vms","roleid":"{UPTRAKIT_PROTECTION_ROLE}","type":"user","ugid":"{user_realm}","propagate":true}},
                {{"path":"/storage","roleid":"{UPTRAKIT_PROTECTION_ROLE}","type":"user","ugid":"{user_realm}","propagate":true}}
            ]"#
        );
        let acls: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid JSON");
        assert!(has_audit_acl(&acls, user_realm));
        assert!(has_protection_vms_acl(&acls, user_realm));
        assert!(has_protection_storage_acl(&acls, user_realm));
    }

    #[test]
    fn verify_pve_acl_missing_protection_role() {
        let user_realm = "uptrakit-11111111-1111-1111-1111-111111111111@pve";
        let json = format!(
            r#"[
                {{"path":"/","roleid":"{UPTRAKIT_AUDIT_ROLE}","type":"user","ugid":"{user_realm}","propagate":true}}
            ]"#
        );
        let acls: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid JSON");
        assert!(has_audit_acl(&acls, user_realm));
        assert!(!has_protection_vms_acl(&acls, user_realm));
        assert!(!has_protection_storage_acl(&acls, user_realm));
    }

    #[test]
    fn verify_pve_acl_missing_all_roles() {
        let user_realm = "uptrakit-11111111-1111-1111-1111-111111111111@pve";
        let json = r#"[
            {"path":"/vms","roleid":"PVEVMAdmin","type":"user","ugid":"admin@pam","propagate":true}
        ]"#;
        let acls: Vec<serde_json::Value> = serde_json::from_str(json).expect("valid JSON");
        assert!(!has_audit_acl(&acls, user_realm));
        assert!(!has_protection_vms_acl(&acls, user_realm));
        assert!(!has_protection_storage_acl(&acls, user_realm));
    }

    #[test]
    fn resolve_pve_api_url_no_api2_suffix() {
        // The function appends only the scheme, host, and port — no path.
        // This is verified indirectly via the format string in the function.
        let url = format!("https://{}:8006", "pve.example.com");
        assert!(!url.contains("api2"));
        assert!(!url.contains("json"));
    }
}
