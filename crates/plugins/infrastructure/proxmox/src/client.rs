//! HTTP client for the Proxmox VE REST API.

use std::time::Duration;

use rootcause::prelude::*;

use crate::api_types::*;
use crate::config::ProxmoxConfig;
use crate::error::{ProxmoxError, Result};

/// HTTP client for communicating with the Proxmox VE API.
pub struct ProxmoxClient {
    client: reqwest::Client,
    base_url: String,
    auth_header: String,
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

        let auth_header = format!("PVEAPIToken={}", config.api_token.expose_secret());

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .danger_accept_invalid_certs(!config.verify_tls)
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
            status = status.as_u16(),
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

    /// Test connectivity by calling the `/version` endpoint.
    pub async fn test_connection(&self) -> Result<serde_json::Value> {
        tracing::debug!(base_url = %self.base_url, "testing Proxmox API connection");
        let version = self.get("/version").await?;
        tracing::debug!("Proxmox API connection test succeeded");
        Ok(version)
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
            api_token: SecretString::new("root@pam!tok=secret".to_string()),
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
            api_token: SecretString::new("root@pam!tok=secret".to_string()),
            verify_tls: true,
            node_filter: vec![],
        };
        let client = ProxmoxClient::new(&config).expect("client");
        assert_eq!(client.base_url, "https://pve.local:8006");
    }
}
