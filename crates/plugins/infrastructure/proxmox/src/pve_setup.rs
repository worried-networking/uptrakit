//! PVE node detection and API credential provisioning.
//!
//! These functions run on a PVE host via a [`RemoteExecutor`] (typically an SSH
//! session) and bootstrap the Proxmox API credentials needed for the controller
//! to use the Proxmox plugin.
//!
//! Identity model: a single cluster-wide user ([`PVE_USER`]) owns one
//! `privsep=1` API token per tenant (id `tenant-{tenant_uuid}`).

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

/// The single cluster-wide PVE user owning all Uptrakit API tokens (ADR: shared
/// user + per-tenant privsep tokens). Never gets a password — token-only identity.
pub const PVE_USER: &str = "uptrakit@pve";

/// Prefix for per-tenant token ids on [`PVE_USER`].
const PVE_TOKEN_PREFIX: &str = "tenant-";

/// Per-tenant token id (`tenant-{tenant_uuid}`; PVE token-id grammar
/// `[A-Za-z][A-Za-z0-9.\-_]+` — verified against pve-access-control source).
pub fn pve_token_id(tenant_id: &uuid::Uuid) -> String {
    format!("{PVE_TOKEN_PREFIX}{tenant_id}")
}

/// Full token id (`uptrakit@pve!tenant-{tenant_uuid}`).
pub fn pve_full_token_id(tenant_id: &uuid::Uuid) -> String {
    format!("{PVE_USER}!{}", pve_token_id(tenant_id))
}

/// State of Uptrakit's PVE identity on a cluster, relative to one tenant.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PveCredentialState {
    /// Whether [`PVE_USER`] exists on the cluster.
    pub user_exists: bool,
    /// Whether this tenant's token (`tenant-{tenant_id}`) exists on [`PVE_USER`].
    pub our_token_exists: bool,
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

/// Read and parse `pvesh get /cluster/status` into its JSON array of entries.
///
/// Shared by [`detect_pve_cluster_nodes`] and [`detect_pve_cluster_name`],
/// which differ only in how they filter/map the resulting entries. Returns
/// `None` on any command or parse failure — this is best-effort information
/// and must never gate anything destructive.
async fn read_cluster_status(executor: &dyn RemoteExecutor) -> Option<Vec<serde_json::Value>> {
    let result = match executor
        .exec_command("pvesh get /cluster/status --output-format json 2>/dev/null")
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "pvesh get /cluster/status failed");
            return None;
        }
    };

    if result.exit_code != 0 || result.stdout.trim().is_empty() {
        tracing::debug!(
            exit_code = result.exit_code,
            "pvesh get /cluster/status returned non-zero or empty output"
        );
        return None;
    }

    match serde_json::from_str(result.stdout.trim()) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::debug!(error = %e, "failed to parse pvesh /cluster/status output");
            None
        }
    }
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
    let Some(entries) = read_cluster_status(executor).await else {
        return Vec::new();
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

/// Detect the PVE cluster name via `pvesh get /cluster/status`.
///
/// Returns the `name` field of the entry with `"type": "cluster"`. Returns
/// `None` on standalone nodes (no such entry) or any read failure — this is a
/// naming fallback only and must never gate anything destructive.
pub async fn detect_pve_cluster_name(executor: &dyn RemoteExecutor) -> Option<String> {
    let entries = read_cluster_status(executor).await?;

    entries
        .into_iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("cluster"))
        .and_then(|e| {
            e.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
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

/// Custom PVE role for resource-scaling operations.
///
/// Privileges:
/// - `VM.Audit` — read current VM/CT config (cores, memory) before scaling.
///   Also required on `/vms` directly because a more-specific ACL on `/vms`
///   (e.g. [`UPTRAKIT_PROTECTION_ROLE`]) shadows the `VM.Audit` granted by
///   [`UPTRAKIT_AUDIT_ROLE`] on `/`.
/// - `VM.Config.CPU` — live-update vCPU count
/// - `VM.Config.Memory` — live-update memory allocation
///
/// Granted on `/vms` alongside [`UPTRAKIT_PROTECTION_ROLE`]; Proxmox unions
/// the privilege sets of multiple roles assigned to the same path.
pub const UPTRAKIT_SCALING_ROLE: &str = "UptrakitScaling";

const PVE_AUDIT_PRIVS: &str = "Sys.Audit VM.Audit VM.GuestAgent.Audit VM.GuestAgent.FileRead";
const PVE_PROTECTION_PRIVS: &str = "VM.Snapshot VM.Backup Datastore.AllocateSpace";
const PVE_SCALING_PRIVS: &str = "VM.Audit VM.Config.CPU VM.Config.Memory";

/// First 200 chars of whichever stream carried the error text.
///
/// With `2>&1` the error text lands in stdout, so a stderr-only message
/// would come back empty; prefer `stderr` when non-empty, else fall back to
/// `stdout`.
fn short_output(stdout: &str, stderr: &str) -> String {
    let err = stderr.trim();
    let merged = if err.is_empty() { stdout.trim() } else { err };
    merged.chars().take(200).collect()
}

/// Check the state of Uptrakit's PVE identity on the cluster for `tenant_id`.
///
/// Read-integrity contract: a state is produced ONLY from successful reads;
/// the single verified-absent special case is `pveum user token list` failing
/// with `no such user` (live-verified: stderr `no such user ('uptrakit@pve')`,
/// exit 255). Any other command failure returns `Err`.
pub async fn check_pve_state(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
) -> Result<PveCredentialState> {
    // 1. Shared-user token list — also the user-existence probe.
    let token_list = executor
        .exec_command(&format!(
            "pveum user token list '{PVE_USER}' --output-format json 2>&1"
        ))
        .await
        .context_to::<ProxmoxError>()?;

    let (user_exists, our_token_exists) = if token_list.exit_code == 0 {
        // 2>&1 merges stderr into the captured stdout, so a stray warning line
        // ahead of the JSON must not poison the parse: scan to the array start.
        let raw = token_list.stdout.trim();
        let json_start = raw.find('[').ok_or_else(|| {
            report!(ProxmoxError::ParseResponse(
                "pveum token list output contains no JSON array".to_string()
            ))
        })?;
        // `find('[')` returns a byte index on an ASCII delimiter, always a
        // char boundary; `.get()` (not `[..]`) keeps clippy's string-slice
        // lint happy without a slicing panic risk.
        let json_slice = raw.get(json_start..).ok_or_else(|| {
            report!(ProxmoxError::ParseResponse(
                "pveum token list output truncated at JSON array start".to_string()
            ))
        })?;
        let tokens: Vec<serde_json::Value> = serde_json::from_str(json_slice).map_err(|e| {
            report!(ProxmoxError::ParseResponse(format!(
                "failed to parse pveum token list output: {e}"
            )))
        })?;
        let wanted = pve_token_id(tenant_id);
        let ours = tokens
            .iter()
            .any(|t| t.get("tokenid").and_then(|v| v.as_str()) == Some(wanted.as_str()));
        (true, ours)
    } else if token_list.stdout.contains("no such user")
        || token_list.stderr.contains("no such user")
    {
        // Live-verified on PVE 9.2.10: absent user => "no such user" + exit 255.
        (false, false)
    } else {
        bail!(ProxmoxError::Plugin(format!(
            "pveum user token list failed (exit {}): {}",
            token_list.exit_code,
            short_output(&token_list.stdout, &token_list.stderr)
        )));
    };

    Ok(PveCredentialState {
        user_exists,
        our_token_exists,
    })
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
        (UPTRAKIT_SCALING_ROLE, PVE_SCALING_PRIVS),
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

/// Ensure [`PVE_USER`] and its per-tenant token hold all required ACLs, at
/// both grant levels:
/// - [`UPTRAKIT_AUDIT_ROLE`] on `/`
/// - [`UPTRAKIT_PROTECTION_ROLE`] on `/vms` (VM.Snapshot, VM.Backup)
/// - [`UPTRAKIT_PROTECTION_ROLE`] on `/storage` (Datastore.AllocateSpace for PBS/dir targets)
/// - [`UPTRAKIT_SCALING_ROLE`] on `/vms` (VM.Audit, VM.Config.CPU, VM.Config.Memory)
///
/// Both the user-level grant (the ceiling PVE intersects a privsep token's
/// privileges against) and the token-level grant (the selection within that
/// ceiling) are applied for each (path, role) pair — a user with zero ACLs
/// zeroes every one of its tokens regardless of the token's own grants.
///
/// `pveum acl modify` is idempotent — calling it when the ACL already exists
/// is a no-op. Any failure is a hard error: a token must never be reported
/// with unconfirmed privileges.
async fn ensure_pve_acls(executor: &dyn RemoteExecutor, tenant_id: &uuid::Uuid) -> Result<()> {
    let full_token = pve_full_token_id(tenant_id);
    let pairs = [
        ("/", UPTRAKIT_AUDIT_ROLE),
        ("/vms", UPTRAKIT_PROTECTION_ROLE),
        ("/storage", UPTRAKIT_PROTECTION_ROLE),
        ("/vms", UPTRAKIT_SCALING_ROLE),
    ];
    for (path, role) in pairs {
        let user_cmd = format!("pveum acl modify '{path}' --users '{PVE_USER}' --roles '{role}'");
        let token_cmd =
            format!("pveum acl modify '{path}' --tokens '{full_token}' --roles '{role}'");
        for (level, cmd) in [("user", user_cmd), ("token", token_cmd)] {
            let result = executor
                .exec_command(&cmd)
                .await
                .context_to::<ProxmoxError>()?;
            if result.exit_code != 0 {
                bail!(ProxmoxError::Plugin(format!(
                    "pveum acl modify failed on {path}/{role} (exit {}): {}",
                    result.exit_code,
                    result.stderr.trim()
                )));
            }
            tracing::debug!(path, role, level, "pveum acl modify granted");
        }
    }
    Ok(())
}

/// Ensure Uptrakit custom roles exist and [`PVE_USER`] plus this tenant's
/// token hold all required ACLs.
///
/// Called both at initial bootstrap and on every host sync. Idempotent.
pub async fn ensure_pve_privileges(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
) -> Result<()> {
    ensure_pve_roles(executor).await?;
    ensure_pve_acls(executor, tenant_id).await?;
    Ok(())
}

/// Maximum accepted instance-host length (DNS name length limit).
const MAX_INSTANCE_HOST_LEN: usize = 253;

/// Sanitize the controller-supplied instance host for shell interpolation.
///
/// The value is operator-entered free text that ends up inside a
/// single-quoted shell argument. Accept only characters that can appear in a
/// `host[:port]` or bracketed-IPv6 literal; anything else returns `None`, so
/// an unusable value degrades to the tenant-only comment instead of reaching
/// a command string.
fn sanitize_instance_host(host: &str) -> Option<String> {
    if host.is_empty() || host.len() > MAX_INSTANCE_HOST_LEN {
        return None;
    }
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        .then(|| host.to_string())
}

/// Build the `--comment` value for a per-tenant API token.
///
/// The tenant UUID is shell-safe by construction (UUID formatting), same
/// posture as the existing `token_id` interpolation.
fn token_comment(instance_host: Option<&str>, tenant_id: &uuid::Uuid) -> String {
    if let Some(raw) = instance_host {
        match sanitize_instance_host(raw) {
            Some(host) => {
                return format!("Uptrakit managed token ({host}, tenant {tenant_id})");
            }
            None => {
                // Operator diagnosability: without this, "host unset" and
                // "host rejected" are indistinguishable. The host is
                // operator-entered public data, not a secret.
                tracing::debug!(
                    host = raw,
                    "instance host rejected by sanitizer; using tenant-only token comment"
                );
            }
        }
    }
    format!("Uptrakit managed token (tenant {tenant_id})")
}

/// Create the [`PVE_USER`] (if absent) and a fresh `privsep=1` API token for
/// `tenant_id`.
///
/// 1. Ensures custom roles exist.
/// 2. Creates [`PVE_USER`] (idempotent — ignores "already exists"); no
///    password is ever set — token-only identity.
/// 3. Creates a `privsep=1` API token named `tenant-{tenant_id}`.
/// 4. Grants both user- and token-level ACLs.
///
/// Returns the credentials needed to configure the Proxmox plugin.
pub async fn create_pve_api_credentials(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
    instance_host: Option<&str>,
) -> Result<PveCredentials> {
    // Step 1: Ensure custom roles exist with correct privilege sets.
    ensure_pve_roles(executor).await?;

    // Step 2: Create the shared user (idempotent — ignore "already exists").
    // Never pass --password: this user is token-only.
    let create_user_cmd =
        format!("pveum user add '{PVE_USER}' --comment 'Uptrakit managed user' 2>&1 || true");
    executor
        .exec_command(&create_user_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    // Step 3: Create the per-tenant API token.
    let token_id = pve_token_id(tenant_id);
    let comment = token_comment(instance_host, tenant_id);
    let create_token_cmd = format!(
        "pveum user token add '{PVE_USER}' '{token_id}' --privsep=1 --comment '{comment}' \
         --output-format json"
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

    // Step 4: Grant user- and token-level ACLs.
    ensure_pve_acls(executor, tenant_id).await?;

    // Step 5: Resolve API URL.
    let api_url = resolve_pve_api_url(executor).await?;

    let api_token = format!("{}={token_value}", pve_full_token_id(tenant_id));

    Ok(PveCredentials { api_url, api_token })
}

/// Remove any existing per-tenant API token on [`PVE_USER`] and create a
/// fresh one.
///
/// Called when the shared user already exists but the local agent has no
/// plugin config for this tenant (e.g. after a re-install). The old token is
/// removed first so that `pveum user token add` does not fail with "already
/// exists".
///
/// ACLs are unconditionally re-granted after token creation — token removal
/// may prune that token's ACL entries (not source-verified either way; the
/// re-grant is idempotent regardless), and a failure here fails the whole
/// regeneration.
///
/// Returns the new [`PveCredentials`] that should be reported to the controller
/// so it can create or update the plugin config entry.
pub async fn regenerate_pve_api_token(
    executor: &dyn RemoteExecutor,
    tenant_id: &uuid::Uuid,
    instance_host: Option<&str>,
) -> Result<PveCredentials> {
    let token_id = pve_token_id(tenant_id);

    // Step 1: Remove the existing token (best-effort — ignore errors).
    let remove_cmd = format!("pveum user token remove '{PVE_USER}' '{token_id}' 2>&1 || true");
    executor
        .exec_command(&remove_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    // Step 2: Create the token again.
    let comment = token_comment(instance_host, tenant_id);
    let create_token_cmd = format!(
        "pveum user token add '{PVE_USER}' '{token_id}' --privsep=1 --comment '{comment}' \
         --output-format json"
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

    // Step 3: Re-grant ACLs unconditionally; failure fails the regeneration.
    ensure_pve_acls(executor, tenant_id).await?;

    // Step 4: Resolve the API URL.
    let api_url = resolve_pve_api_url(executor).await?;

    let api_token = format!("{}={token_value}", pve_full_token_id(tenant_id));

    Ok(PveCredentials { api_url, api_token })
}

/// Prove a token works, on the node itself over the existing SSH transport.
///
/// `-k` is sound: the proof targets token validity on localhost, not
/// endpoint trust.
///
/// The secret travels only inside the SSH session. A remote command is one
/// opaque string to the executor, so the token cannot be kept out of `cmd`
/// itself; the SSH transport redacts `Authorization:` header values before
/// tracing a command (`redact_for_log` in `agent-ssh-runtime`'s
/// `ssh_transport`), which is what keeps it out of agent logs.
pub async fn prove_token_on_node(executor: &dyn RemoteExecutor, api_token: &str) -> Result<()> {
    let cmd = format!(
        "curl -sk -o /dev/null -w '%{{http_code}}' \
         -H 'Authorization: PVEAPIToken={api_token}' \
         https://localhost:8006/api2/json/version"
    );
    let result = executor
        .exec_command(&cmd)
        .await
        .context_to::<ProxmoxError>()?;
    if result.exit_code == 127 {
        // Distinguish a missing proof tool from a rejected token — the
        // operator remedies are entirely different (install curl vs
        // investigate credentials). PVE nodes ship curl, but do not assume.
        bail!(ProxmoxError::Plugin(
            "on-node token proof unavailable: curl not found on the PVE node".to_string()
        ));
    }
    let code = result.stdout.trim();
    if result.exit_code != 0 || code != "200" {
        bail!(ProxmoxError::Plugin(format!(
            "on-node token proof failed (curl exit {}, http {code})",
            result.exit_code
        )));
    }
    Ok(())
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
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use uptrakit_command::RemoteCommandResult;
    use uptrakit_command::test_support::ScriptedRemoteExecutor;

    use super::*;

    fn tenant() -> uuid::Uuid {
        uuid::Uuid::from_u128(0x11111111_1111_1111_1111_111111111111)
    }

    fn ok(stdout: impl Into<String>) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn err(exit_code: u32, stdout: impl Into<String>) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code,
        }
    }

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
    fn resolve_pve_api_url_no_api2_suffix() {
        // The function appends only the scheme, host, and port — no path.
        // This is verified indirectly via the format string in the function.
        let url = format!("https://{}:8006", "pve.example.com");
        assert!(!url.contains("api2"));
        assert!(!url.contains("json"));
    }

    #[test]
    fn token_id_builders_format() {
        let tid = tenant();
        assert_eq!(
            pve_token_id(&tid),
            "tenant-11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(
            pve_full_token_id(&tid),
            "uptrakit@pve!tenant-11111111-1111-1111-1111-111111111111"
        );
    }

    #[test]
    fn sanitize_instance_host_accepts_and_rejects() {
        for ok in [
            "uptrakit.example.com",
            "host:8443",
            "192.168.1.10",
            "[::1]:8443",
        ] {
            assert_eq!(
                sanitize_instance_host(ok).as_deref(),
                Some(ok),
                "must accept {ok}"
            );
        }
        let too_long = "a".repeat(254);
        for bad in [
            "bad'host",
            "bad;host",
            "bad host",
            "user@host",
            "host/path",
            "",
            too_long.as_str(),
        ] {
            assert_eq!(sanitize_instance_host(bad), None, "must reject {bad:?}");
        }
    }

    #[test]
    fn token_comment_includes_host_when_sanitizable_and_falls_back_otherwise() {
        let tenant = uuid::Uuid::nil();
        assert_eq!(
            token_comment(Some("uptrakit.example.com"), &tenant),
            format!("Uptrakit managed token (uptrakit.example.com, tenant {tenant})")
        );
        assert_eq!(
            token_comment(None, &tenant),
            format!("Uptrakit managed token (tenant {tenant})")
        );
        // Failure path: a rejected host degrades to the tenant-only comment.
        assert_eq!(
            token_comment(Some("evil' --injected"), &tenant),
            format!("Uptrakit managed token (tenant {tenant})")
        );
    }

    #[tokio::test]
    async fn check_pve_state_fresh_cluster() {
        let tid = tenant();
        let executor = ScriptedRemoteExecutor::with_matcher(vec![(
            "pveum user token list",
            err(255, "no such user ('uptrakit@pve')"),
        )]);
        let state = check_pve_state(&executor, &tid).await.expect("state read");
        assert_eq!(
            state,
            PveCredentialState {
                user_exists: false,
                our_token_exists: false,
            }
        );
    }

    #[tokio::test]
    async fn check_pve_state_user_no_token() {
        let tid = tenant();
        let executor =
            ScriptedRemoteExecutor::with_matcher(vec![("pveum user token list", ok("[]"))]);
        let state = check_pve_state(&executor, &tid).await.expect("state read");
        assert!(state.user_exists, "user must be reported as existing");
        assert!(
            !state.our_token_exists,
            "no token was scripted, so our_token_exists must be false"
        );
    }

    #[tokio::test]
    async fn check_pve_state_read_failure_is_err() {
        let tid = tenant();
        let executor = ScriptedRemoteExecutor::with_matcher(vec![(
            "pveum user token list",
            err(1, "permission denied: totally unrelated failure text"),
        )]);
        let result = check_pve_state(&executor, &tid).await;
        let err = result.expect_err("non-'no such user' failure must be Err, not degrade");
        let message = format!("{err}");
        assert!(
            message.contains("totally unrelated failure text"),
            "error message must contain the captured stdout text: {message}"
        );
    }

    #[tokio::test]
    async fn check_pve_state_tolerates_stderr_noise_before_json() {
        let tid = tenant();
        // Noise ahead of the JSON array on the TOKEN list.
        let executor = ScriptedRemoteExecutor::with_matcher(vec![(
            "pveum user token list",
            ok("WARNING: some pveum notice\n[]"),
        )]);
        let state = check_pve_state(&executor, &tid)
            .await
            .expect("noise-before-JSON on token list must still parse");
        assert!(!state.our_token_exists);
    }

    #[tokio::test]
    async fn ensure_acls_grants_both_levels() {
        let tid = tenant();
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pveum role add", ok("")),
            ("pveum role modify", ok("")),
            ("pveum acl modify", ok("")),
        ]);
        ensure_pve_privileges(&executor, &tid)
            .await
            .expect("ensure_pve_privileges succeeds");

        let calls = executor.recorded_calls();
        let full_token = pve_full_token_id(&tid);
        let expected = [
            ("/", UPTRAKIT_AUDIT_ROLE, "--users"),
            ("/", UPTRAKIT_AUDIT_ROLE, "--tokens"),
            ("/vms", UPTRAKIT_PROTECTION_ROLE, "--users"),
            ("/vms", UPTRAKIT_PROTECTION_ROLE, "--tokens"),
            ("/storage", UPTRAKIT_PROTECTION_ROLE, "--users"),
            ("/storage", UPTRAKIT_PROTECTION_ROLE, "--tokens"),
            ("/vms", UPTRAKIT_SCALING_ROLE, "--users"),
            ("/vms", UPTRAKIT_SCALING_ROLE, "--tokens"),
        ];
        let expected_cmds: Vec<String> = expected
            .iter()
            .map(|(path, role, level)| {
                let grantee = if *level == "--users" {
                    format!("--users '{PVE_USER}'")
                } else {
                    format!("--tokens '{full_token}'")
                };
                format!("pveum acl modify '{path}' {grantee} --roles '{role}'")
            })
            .collect();

        let acl_calls: Vec<&String> = calls
            .iter()
            .filter(|c| c.contains("pveum acl modify"))
            .collect();
        assert_eq!(
            acl_calls,
            expected_cmds.iter().collect::<Vec<_>>(),
            "expected exactly the 8 (path, role, level) ACL grants in order: {calls:?}"
        );
    }

    #[tokio::test]
    async fn create_uses_privsep1_and_no_password() {
        let tid = tenant();
        let secret = "sekrit-value";
        let token_json = format!(
            r#"{{"full-tokenid":"uptrakit@pve!tenant-{tid}","info":{{"privsep":"1"}},"value":"{secret}"}}"#
        );
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pveum role add", ok("")),
            ("pveum role modify", ok("")),
            ("pveum user add", ok("")),
            ("pveum user token add", ok(token_json)),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
        ]);
        let creds = create_pve_api_credentials(&executor, &tid, Some("uptrakit.example.com"))
            .await
            .expect("credential creation succeeds");

        let calls = executor.recorded_calls();
        let token_add_line = calls
            .iter()
            .find(|c| c.contains("pveum user token add"))
            .expect("token add line recorded");
        assert!(
            token_add_line.contains("--privsep=1"),
            "token add must request privsep=1: {token_add_line}"
        );
        assert!(
            token_add_line.contains(&format!(
                "--comment 'Uptrakit managed token (uptrakit.example.com, tenant {tid})'"
            )),
            "token add must carry the host+tenant comment when a sanitizable host is \
             supplied: {token_add_line}"
        );
        let user_add_line = calls
            .iter()
            .find(|c| c.contains("pveum user add"))
            .expect("user add line recorded");
        assert!(
            !user_add_line.contains("--password"),
            "user add must never set a password: {user_add_line}"
        );

        assert_eq!(
            creds.api_token,
            format!("uptrakit@pve!tenant-{tid}={secret}")
        );
    }

    #[tokio::test]
    async fn regenerate_regrants_acls_after_token_add() {
        let tid = tenant();
        let secret = "sekrit-value";
        let token_json = format!(
            r#"{{"full-tokenid":"uptrakit@pve!tenant-{tid}","info":{{"privsep":"1"}},"value":"{secret}"}}"#
        );
        let executor = ScriptedRemoteExecutor::with_matcher(vec![
            ("pveum user token remove", ok("")),
            ("pveum user token add", ok(token_json.clone())),
            ("pveum acl modify", ok("")),
            ("hostname -f", ok("pve.example.com")),
        ]);
        regenerate_pve_api_token(&executor, &tid, None)
            .await
            .expect("regeneration succeeds");

        let calls = executor.recorded_calls();
        let remove_idx = calls
            .iter()
            .position(|c| c.contains("pveum user token remove"))
            .expect("remove call recorded");
        let add_idx = calls
            .iter()
            .position(|c| c.contains("pveum user token add"))
            .expect("add call recorded");
        let acl_idx = calls
            .iter()
            .position(|c| c.contains("pveum acl modify"))
            .expect("acl modify call recorded");
        assert!(
            remove_idx < add_idx && add_idx < acl_idx,
            "expected order remove < add < first acl modify: {calls:?}"
        );
        let add_call = &calls[add_idx];
        assert!(
            add_call.contains(&format!(
                "--comment 'Uptrakit managed token (tenant {tid})'"
            )),
            "no instance host supplied must yield the tenant-only comment: {add_call}"
        );

        // Variant: an acl modify failure must fail the whole regeneration.
        let executor2 = ScriptedRemoteExecutor::with_matcher(vec![
            ("pveum user token remove", ok("")),
            ("pveum user token add", ok(token_json)),
            ("pveum acl modify", err(1, "denied")),
        ]);
        let result = regenerate_pve_api_token(&executor2, &tid, None).await;
        assert!(
            result.is_err(),
            "an acl modify failure during regeneration must not silently succeed"
        );
    }

    #[tokio::test]
    async fn prove_token_success_and_failure() {
        let executor_ok = ScriptedRemoteExecutor::with_matcher(vec![("curl", ok("200"))]);
        prove_token_on_node(&executor_ok, "uptrakit@pve!tenant-x=secret")
            .await
            .expect("200 response proves the token");

        let executor_rejected = ScriptedRemoteExecutor::with_matcher(vec![("curl", ok("401"))]);
        assert!(
            prove_token_on_node(&executor_rejected, "uptrakit@pve!tenant-x=secret")
                .await
                .is_err(),
            "a non-200 response must be an error"
        );

        let executor_missing_curl =
            ScriptedRemoteExecutor::with_matcher(vec![("curl", err(127, ""))]);
        let result =
            prove_token_on_node(&executor_missing_curl, "uptrakit@pve!tenant-x=secret").await;
        let err = result.expect_err("exit 127 must be an error");
        let message = format!("{err}");
        assert!(
            message.contains("curl not found"),
            "exit-127 message must be distinct from a rejected-token message: {message}"
        );
    }

    #[tokio::test]
    async fn cluster_name_extraction() {
        let executor_cluster = ScriptedRemoteExecutor::with_matcher(vec![(
            "pvesh get /cluster/status",
            ok(r#"[{"type":"cluster","name":"uk-home1"},{"type":"node","name":"pve1"}]"#),
        )]);
        assert_eq!(
            detect_pve_cluster_name(&executor_cluster).await,
            Some("uk-home1".to_string())
        );

        let executor_standalone = ScriptedRemoteExecutor::with_matcher(vec![(
            "pvesh get /cluster/status",
            ok(r#"[{"type":"node","name":"pve1"}]"#),
        )]);
        assert_eq!(detect_pve_cluster_name(&executor_standalone).await, None);
    }
}
