//! Serde structs for Proxmox VE REST API JSON responses.
//!
//! All Proxmox API responses wrap the payload in `{"data": T}`.

use serde::Deserialize;

/// Proxmox API response wrapper: `{"data": T}`.
#[derive(Debug, Deserialize)]
pub struct PveResponse<T> {
    pub data: T,
}

/// A Proxmox VE cluster node.
#[derive(Debug, Clone, Deserialize)]
pub struct PveNode {
    /// Node name (e.g., `"pve1"`).
    pub node: String,
    /// Node status: `"online"` or `"offline"`.
    pub status: String,
}

/// A QEMU virtual machine entry from `GET /nodes/{node}/qemu`.
#[derive(Debug, Clone, Deserialize)]
pub struct PveQemuVm {
    /// VM identifier (numeric).
    pub vmid: u32,
    /// VM name.
    #[serde(default)]
    pub name: Option<String>,
    /// VM status: `"running"`, `"stopped"`, etc.
    pub status: String,
}

/// An LXC container entry from `GET /nodes/{node}/lxc`.
#[derive(Debug, Clone, Deserialize)]
pub struct PveLxcContainer {
    /// Container identifier (numeric).
    pub vmid: u32,
    /// Container name/hostname.
    #[serde(default)]
    pub name: Option<String>,
    /// Container status: `"running"`, `"stopped"`, etc.
    pub status: String,
}

/// QEMU VM configuration from `GET /nodes/{node}/qemu/{vmid}/config`.
#[derive(Debug, Clone, Deserialize)]
pub struct PveQemuConfig {
    /// VM name.
    #[serde(default)]
    pub name: Option<String>,
}

/// LXC container configuration from `GET /nodes/{node}/lxc/{vmid}/config`.
#[derive(Debug, Clone, Deserialize)]
pub struct PveLxcConfig {
    /// Container hostname.
    #[serde(default)]
    pub hostname: Option<String>,
}

/// A single network interface from the QEMU guest agent.
#[derive(Debug, Clone, Deserialize)]
pub struct PveNetworkInterface {
    /// Interface name (e.g., `"eth0"`).
    pub name: String,
    /// IP addresses on this interface.
    #[serde(default, rename = "ip-addresses")]
    pub ip_addresses: Vec<PveIpAddress>,
}

/// An IP address reported by the QEMU guest agent.
#[derive(Debug, Clone, Deserialize)]
pub struct PveIpAddress {
    /// IP address string.
    #[serde(rename = "ip-address")]
    pub ip_address: String,
    /// Address type: `"ipv4"` or `"ipv6"`.
    #[serde(rename = "ip-address-type")]
    pub ip_address_type: String,
}

/// Guest agent network-get-interfaces response.
#[derive(Debug, Clone, Deserialize)]
pub struct PveAgentNetworkResult {
    /// The list of network interfaces (may be absent if agent is not running).
    #[serde(default)]
    pub result: Vec<PveNetworkInterface>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_node_list() {
        let json =
            r#"{"data":[{"node":"pve1","status":"online"},{"node":"pve2","status":"offline"}]}"#;
        let resp: PveResponse<Vec<PveNode>> = serde_json::from_str(json).expect("deserialize");
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].node, "pve1");
        assert_eq!(resp.data[0].status, "online");
    }

    #[test]
    fn deserialize_qemu_list() {
        let json = r#"{"data":[{"vmid":100,"name":"web-server","status":"running"},{"vmid":101,"status":"stopped"}]}"#;
        let resp: PveResponse<Vec<PveQemuVm>> = serde_json::from_str(json).expect("deserialize");
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].vmid, 100);
        assert_eq!(resp.data[0].name.as_deref(), Some("web-server"));
        assert!(resp.data[1].name.is_none());
    }

    #[test]
    fn deserialize_lxc_list() {
        let json = r#"{"data":[{"vmid":200,"name":"dns","status":"running"}]}"#;
        let resp: PveResponse<Vec<PveLxcContainer>> =
            serde_json::from_str(json).expect("deserialize");
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].vmid, 200);
    }

    #[test]
    fn deserialize_agent_network() {
        let json = r#"{"data":{"result":[{"name":"eth0","ip-addresses":[{"ip-address":"10.0.0.1","ip-address-type":"ipv4"}]}]}}"#;
        let resp: PveResponse<PveAgentNetworkResult> =
            serde_json::from_str(json).expect("deserialize");
        assert_eq!(resp.data.result.len(), 1);
        assert_eq!(resp.data.result[0].ip_addresses[0].ip_address, "10.0.0.1");
    }
}
