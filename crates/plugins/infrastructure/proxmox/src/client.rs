//! HTTP client for the Proxmox VE REST API.

use std::time::{Duration, Instant};

use rootcause::prelude::*;
use std::sync::Arc;
use uptrakit_shared_types::ssrf::{
    SsrfSafeResolver, danger_accept_any_cert_client_config, webpki_client_config,
};

use crate::api_types::*;
use crate::config::ProxmoxConfig;
use crate::error::{ProxmoxError, Result};

/// HTTP client for communicating with the Proxmox VE API.
pub struct ProxmoxClient {
    client: reqwest::Client,
    base_url: String,
    auth_header: String,
}

/// Cached backup-capable storage target for a specific Proxmox node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupTarget {
    pub node: String,
    pub storage_id: String,
    pub storage_type: String,
    pub target_key: String,
}

impl ProxmoxClient {
    /// Create a new Proxmox API client from the given configuration.
    pub fn new(config: &ProxmoxConfig) -> Result<Self> {
        tracing::debug!(
            api_url = %config.api_url,
            verify_tls = config.verify_tls,
            node_filter = ?config.node_filter,
            "creating Proxmox API client"
        );
        if !config.verify_tls {
            tracing::warn!(
                api_url = %config.api_url,
                "Proxmox TLS verification is disabled; connection is vulnerable to MitM attacks"
            );
        }

        let auth_header = format!("PVEAPIToken={}", config.api_token.expose_secret());

        // `use_preconfigured_tls` supersedes `danger_accept_invalid_certs` in
        // reqwest 0.13 (the latter is silently ignored when the former is set).
        // Select the appropriate config based on the user's `verify_tls` setting.
        let tls_config = if config.verify_tls {
            webpki_client_config()
        } else {
            danger_accept_any_cert_client_config()
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .dns_resolver(Arc::new(SsrfSafeResolver::permissive()))
            .use_preconfigured_tls(tls_config)
            .build()
            .map_err(|e| {
                report!(ProxmoxError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        // Normalize base URL: strip trailing slash
        let base_url = config.api_url.trim_end_matches('/').to_string();

        Ok(Self {
            client,
            base_url,
            auth_header,
        })
    }

    /// Perform a GET request to the Proxmox API.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}/api2/json{path}", self.base_url);

        tracing::trace!(path, "sending GET request to Proxmox API");

        let response = self
            .client
            .get(&url)
            .header("Authorization", &self.auth_header)
            .send()
            .await
            .map_err(|e| {
                report!(ProxmoxError::Request(format!(
                    "HTTP request to {path} failed: {e}"
                )))
            })?;

        let status = response.status();

        tracing::trace!(
            path,
            status = %status,
            "received Proxmox API response"
        );

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!(ProxmoxError::ApiError {
                status,
                message: body,
            });
        }

        let wrapper: PveResponse<T> = response.json().await.map_err(|e| {
            report!(ProxmoxError::ParseResponse(format!(
                "failed to parse response from {path}: {e}"
            )))
        })?;

        Ok(wrapper.data)
    }

    /// Perform a POST form request to the Proxmox API.
    async fn post_form<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(String, String)],
    ) -> Result<T> {
        let url = format!("{}/api2/json{path}", self.base_url);

        tracing::trace!(
            path,
            param_count = params.len(),
            "sending POST request to Proxmox API"
        );

        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .finish();
        let request = self
            .client
            .post(&url)
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(encoded);

        let response = request.send().await.map_err(|e| {
            report!(ProxmoxError::Request(format!(
                "HTTP request to {path} failed: {e}"
            )))
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!(ProxmoxError::ApiError {
                status,
                message: body,
            });
        }

        let wrapper: PveResponse<T> = response.json().await.map_err(|e| {
            report!(ProxmoxError::ParseResponse(format!(
                "failed to parse response from {path}: {e}"
            )))
        })?;

        Ok(wrapper.data)
    }

    /// Build a stable node-aware backup target key.
    fn backup_target_key(node: &str, storage_id: &str, storage_type: &str) -> String {
        format!("{node}:{storage_id}:{storage_type}")
    }

    fn task_upid_from_data(data: serde_json::Value, operation: &str) -> Result<String> {
        if let Some(upid) = data.as_str()
            && !upid.trim().is_empty()
        {
            return Ok(upid.to_string());
        }

        if let Some(upid) = data.get("upid").and_then(serde_json::Value::as_str)
            && !upid.trim().is_empty()
        {
            return Ok(upid.to_string());
        }

        bail!(ProxmoxError::ParseResponse(format!(
            "{operation} returned no task id"
        )))
    }

    fn storage_supports_backup(content: &str) -> bool {
        content
            .split(',')
            .map(str::trim)
            .any(|kind| kind.eq_ignore_ascii_case("backup"))
    }

    /// List all cluster nodes.
    pub async fn get_nodes(&self) -> Result<Vec<PveNode>> {
        tracing::debug!("fetching Proxmox cluster nodes");
        let nodes: Vec<PveNode> = self.get("/nodes").await?;
        tracing::debug!(count = nodes.len(), "received cluster nodes");
        Ok(nodes)
    }

    /// List QEMU VMs on a given node.
    pub async fn get_qemu_vms(&self, node: &str) -> Result<Vec<PveQemuVm>> {
        tracing::debug!(node, "fetching QEMU VMs");
        let vms: Vec<PveQemuVm> = self.get(&format!("/nodes/{node}/qemu")).await?;
        tracing::debug!(node, count = vms.len(), "received QEMU VMs");
        Ok(vms)
    }

    /// List LXC containers on a given node.
    pub async fn get_lxc_containers(&self, node: &str) -> Result<Vec<PveLxcContainer>> {
        tracing::debug!(node, "fetching LXC containers");
        let cts: Vec<PveLxcContainer> = self.get(&format!("/nodes/{node}/lxc")).await?;
        tracing::debug!(node, count = cts.len(), "received LXC containers");
        Ok(cts)
    }

    /// Get QEMU VM configuration.
    pub async fn get_qemu_config(&self, node: &str, vmid: u32) -> Result<PveQemuConfig> {
        tracing::trace!(node, vmid, "fetching QEMU VM config");
        self.get(&format!("/nodes/{node}/qemu/{vmid}/config")).await
    }

    /// Get LXC container configuration.
    pub async fn get_lxc_config(&self, node: &str, vmid: u32) -> Result<PveLxcConfig> {
        tracing::trace!(node, vmid, "fetching LXC container config");
        self.get(&format!("/nodes/{node}/lxc/{vmid}/config")).await
    }

    /// Get network interfaces from the QEMU guest agent.
    ///
    /// Returns `None` if the guest agent is not running or not installed.
    pub async fn get_qemu_agent_network(
        &self,
        node: &str,
        vmid: u32,
    ) -> Option<Vec<PveNetworkInterface>> {
        tracing::trace!(node, vmid, "querying QEMU guest agent network interfaces");

        let result: std::result::Result<PveAgentNetworkResult, _> = self
            .get(&format!(
                "/nodes/{node}/qemu/{vmid}/agent/network-get-interfaces"
            ))
            .await;

        match result {
            Ok(data) => {
                tracing::trace!(
                    node,
                    vmid,
                    interface_count = data.result.len(),
                    "received guest agent network interfaces"
                );
                Some(data.result)
            }
            Err(e) => {
                tracing::debug!(
                    node,
                    vmid,
                    error = %e,
                    "QEMU guest agent network query failed (agent may not be running)"
                );
                None
            }
        }
    }

    /// Read a file from a QEMU guest via the guest agent.
    ///
    /// Uses `GET /nodes/{node}/qemu/{vmid}/agent/file-read?file={file}`.
    /// Returns `None` on any error (agent not running, file not found, etc.).
    /// Only works for QEMU VMs with the guest agent installed; LXC containers
    /// do not support this endpoint.
    pub async fn read_guest_file(&self, node: &str, vmid: u32, file: &str) -> Option<String> {
        tracing::trace!(node, vmid, file, "reading file from QEMU guest agent");

        // Build path with query parameter. The `file` path (e.g. `/etc/machine-id`)
        // is URL-safe, but we use form_urlencoded for correctness.
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("file", file)
            .finish();
        let result: std::result::Result<PveFileReadResult, _> = self
            .get(&format!(
                "/nodes/{node}/qemu/{vmid}/agent/file-read?{query}"
            ))
            .await;

        match result {
            Ok(data) => {
                tracing::trace!(
                    node,
                    vmid,
                    file,
                    truncated = data.truncated,
                    "read file from guest agent"
                );
                Some(data.content)
            }
            Err(e) => {
                tracing::debug!(
                    node,
                    vmid,
                    file,
                    error = %e,
                    "guest agent file-read failed (agent may not be running or file not found)"
                );
                None
            }
        }
    }

    /// Test connectivity by calling the `/version` endpoint.
    pub async fn test_connection(&self) -> Result<serde_json::Value> {
        tracing::debug!(base_url = %self.base_url, "testing Proxmox API connection");
        let version = self.get("/version").await?;
        tracing::debug!("Proxmox API connection test succeeded");
        Ok(version)
    }

    /// List backup-capable storage targets for one node.
    pub async fn list_backup_targets_for_node(&self, node: &str) -> Result<Vec<BackupTarget>> {
        let storages: Vec<PveStorage> = self.get(&format!("/nodes/{node}/storage")).await?;

        let mut targets = Vec::new();
        for storage in storages {
            let is_enabled = storage.enabled.unwrap_or(1) != 0;
            let is_active = storage.active.unwrap_or(1) != 0;
            let supports_backup = storage
                .content
                .as_deref()
                .is_some_and(Self::storage_supports_backup);

            if !is_enabled || !is_active || !supports_backup {
                continue;
            }

            let storage_type = storage
                .storage_type
                .unwrap_or_else(|| "unknown".to_string());
            let target_key = Self::backup_target_key(node, &storage.storage_id, &storage_type);

            targets.push(BackupTarget {
                node: node.to_string(),
                storage_id: storage.storage_id,
                storage_type,
                target_key,
            });
        }

        Ok(targets)
    }

    /// Create a snapshot for a QEMU VM.
    pub async fn create_qemu_snapshot(
        &self,
        node: &str,
        vmid: u32,
        snapshot_name: &str,
    ) -> Result<String> {
        let data: serde_json::Value = self
            .post_form(
                &format!("/nodes/{node}/qemu/{vmid}/snapshot"),
                &[("snapname".to_string(), snapshot_name.to_string())],
            )
            .await?;
        Self::task_upid_from_data(data, "QEMU snapshot create")
    }

    /// Create a snapshot for an LXC container.
    pub async fn create_lxc_snapshot(
        &self,
        node: &str,
        vmid: u32,
        snapshot_name: &str,
    ) -> Result<String> {
        let data: serde_json::Value = self
            .post_form(
                &format!("/nodes/{node}/lxc/{vmid}/snapshot"),
                &[("snapname".to_string(), snapshot_name.to_string())],
            )
            .await?;
        Self::task_upid_from_data(data, "LXC snapshot create")
    }

    /// Start a backup task for one guest.
    pub async fn start_backup(
        &self,
        node: &str,
        vmid: u32,
        _guest_type: &str,
        storage_id: &str,
    ) -> Result<String> {
        let data: serde_json::Value = self
            .post_form(
                &format!("/nodes/{node}/vzdump"),
                &[
                    ("vmid".to_string(), vmid.to_string()),
                    ("storage".to_string(), storage_id.to_string()),
                    ("mode".to_string(), "snapshot".to_string()),
                    ("quiet".to_string(), "1".to_string()),
                ],
            )
            .await?;
        Self::task_upid_from_data(data, "backup start")
    }

    /// Fetch one Proxmox task status row.
    pub async fn task_status(&self, node: &str, upid: &str) -> Result<PveTaskStatus> {
        self.get(&format!("/nodes/{node}/tasks/{upid}/status"))
            .await
    }

    /// Poll a Proxmox task until it succeeds/fails or timeout is reached.
    pub async fn wait_for_task_completion(
        &self,
        node: &str,
        upid: &str,
        timeout: Duration,
    ) -> Result<PveTaskStatus> {
        let deadline = Instant::now() + timeout;

        loop {
            let status = self.task_status(node, upid).await?;
            if status.status.eq_ignore_ascii_case("stopped") {
                if status.exitstatus.as_deref() == Some("OK") {
                    return Ok(status);
                }

                let exit = status
                    .exitstatus
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                bail!(ProxmoxError::Plugin(format!(
                    "Proxmox task failed with exit status: {exit}"
                )));
            }

            if Instant::now() >= deadline {
                bail!(ProxmoxError::Plugin(
                    "Timed out waiting for Proxmox task completion".to_string()
                ));
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::SecretString;

    #[test]
    fn client_creation_with_valid_config() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006".to_string(),
            api_token: SecretString::new("root@pam!tok=secret"),
            verify_tls: false,
            node_filter: vec![],
        };
        let client = ProxmoxClient::new(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn client_strips_trailing_slash() {
        let config = ProxmoxConfig {
            api_url: "https://pve.local:8006/".to_string(),
            api_token: SecretString::new("root@pam!tok=secret"),
            verify_tls: true,
            node_filter: vec![],
        };
        let client = ProxmoxClient::new(&config).expect("client");
        assert_eq!(client.base_url, "https://pve.local:8006");
    }

    #[test]
    fn backup_target_key_is_node_aware() {
        assert_eq!(
            ProxmoxClient::backup_target_key("pve1", "local", "dir"),
            "pve1:local:dir"
        );
    }
}
