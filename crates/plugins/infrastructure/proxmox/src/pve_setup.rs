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
    /// Full API URL (e.g. `https://pve.local:8006/api2/json`).
    pub api_url: String,
    /// API token in PVE format (`USER@REALM!TOKENID=SECRET`).
    pub api_token: String,
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

/// Resolve the PVE API URL from the node's hostname/IP.
///
/// Reads the hostname from the remote and constructs the standard API endpoint.
pub async fn resolve_pve_api_url(executor: &dyn RemoteExecutor) -> Result<String> {
    let result = executor
        .exec_command("hostname -f")
        .await
        .context_to::<ProxmoxError>()?;
    let hostname = result.stdout.trim().to_string();
    if hostname.is_empty() {
        bail!(ProxmoxError::Plugin("remote hostname is empty".to_string()));
    }
    Ok(format!("https://{hostname}:8006/api2/json"))
}

/// Create a PVE API user and token for Uptrakit.
///
/// 1. Creates the user `{pve_username}@pve` (ignores "already exists" errors)
/// 2. Creates an API token named `uptrakit` with `privsep=0`
/// 3. Grants `PVEAuditor` role on `/` (read-only cluster access)
///
/// Returns the credentials needed to configure the Proxmox plugin.
pub async fn create_pve_api_credentials(
    executor: &dyn RemoteExecutor,
    pve_username: &str,
) -> Result<PveCredentials> {
    let user_realm = format!("{pve_username}@pve");

    // Step 1: Create user (idempotent — ignore "already exists")
    let create_user_cmd =
        format!("pveum user add '{user_realm}' --comment 'Created by Uptrakit' 2>&1 || true");
    executor
        .exec_command(&create_user_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    // Step 2: Create API token
    let create_token_cmd =
        format!("pveum user token add '{user_realm}' uptrakit --privsep=0 --output-format json");
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

    // Step 3: Grant PVEAuditor role
    let acl_cmd = format!("pveum acl modify / --users '{user_realm}' --roles PVEAuditor");
    let acl_result = executor
        .exec_command(&acl_cmd)
        .await
        .context_to::<ProxmoxError>()?;

    if acl_result.exit_code != 0 {
        tracing::warn!(
            stderr = %acl_result.stderr.trim(),
            "pveum acl modify returned non-zero exit code"
        );
    }

    // Step 4: Resolve API URL
    let api_url = resolve_pve_api_url(executor).await?;

    let api_token = format!("{user_realm}!uptrakit={token_value}");

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
}
