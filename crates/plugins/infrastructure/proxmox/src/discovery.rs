//! Discovery logic: query Proxmox nodes for VMs and containers.

use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::api_types::{PveLxcContainer, PveQemuVm};
use crate::client::ProxmoxClient;
use crate::error::{ProxmoxError, Result};

/// A discovered Proxmox guest (VM or container) before DB persistence.
#[derive(Debug, Clone)]
pub struct DiscoveredGuest {
    /// Proxmox node name.
    pub node: String,
    /// VMID.
    pub vmid: u32,
    /// Guest type: `"qemu"` or `"lxc"`.
    pub guest_type: &'static str,
    /// Guest name (from the list endpoint or config).
    pub name: Option<String>,
    /// Current status (`"running"`, `"stopped"`, etc.).
    pub status: String,
    /// Hostname from config (LXC) or name (QEMU).
    pub hostname: Option<String>,
    /// IP addresses discovered via QEMU guest agent.
    pub ip_addresses: Vec<String>,
}

/// Discover all VMs and containers from the Proxmox API.
///
/// Queries all nodes (or only those matching `node_filter`), then enumerates
/// QEMU VMs and LXC containers on each node. For running QEMU VMs, attempts
/// to query the guest agent for IP addresses.
pub async fn discover_guests(
    client: &ProxmoxClient,
    node_filter: &[String],
) -> Result<Vec<DiscoveredGuest>> {
    let nodes = client.get_nodes().await?;
    let mut guests = Vec::new();

    for node in &nodes {
        if node.status != "online" {
            tracing::debug!(node = %node.node, status = %node.status, "skipping offline node");
            continue;
        }

        if !node_filter.is_empty() && !node_filter.contains(&node.node) {
            tracing::debug!(node = %node.node, "skipping node not in filter");
            continue;
        }

        // Discover QEMU VMs
        match client.get_qemu_vms(&node.node).await {
            Ok(vms) => {
                for vm in vms {
                    let guest = discover_qemu_guest(client, &node.node, vm).await;
                    guests.push(guest);
                }
            }
            Err(e) => {
                tracing::warn!(node = %node.node, error = %e, "failed to list QEMU VMs");
            }
        }

        // Discover LXC containers
        match client.get_lxc_containers(&node.node).await {
            Ok(cts) => {
                for ct in cts {
                    let guest = discover_lxc_guest(client, &node.node, ct).await;
                    guests.push(guest);
                }
            }
            Err(e) => {
                tracing::warn!(node = %node.node, error = %e, "failed to list LXC containers");
            }
        }
    }

    Ok(guests)
}

/// Build a `DiscoveredGuest` for a QEMU VM, optionally querying the guest agent.
async fn discover_qemu_guest(client: &ProxmoxClient, node: &str, vm: PveQemuVm) -> DiscoveredGuest {
    let hostname = vm.name.clone();

    // Try to get IP addresses from guest agent (only for running VMs)
    let ip_addresses = if vm.status == "running" {
        if let Some(interfaces) = client.get_qemu_agent_network(node, vm.vmid).await {
            interfaces
                .into_iter()
                .flat_map(|iface| {
                    iface
                        .ip_addresses
                        .into_iter()
                        .filter(|ip| ip.ip_address_type == "ipv4")
                        .filter(|ip| !ip.ip_address.starts_with("127."))
                        .map(|ip| ip.ip_address)
                })
                .collect()
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    DiscoveredGuest {
        node: node.to_string(),
        vmid: vm.vmid,
        guest_type: "qemu",
        name: vm.name,
        status: vm.status,
        hostname,
        ip_addresses,
    }
}

/// Build a `DiscoveredGuest` for an LXC container.
async fn discover_lxc_guest(
    client: &ProxmoxClient,
    node: &str,
    ct: PveLxcContainer,
) -> DiscoveredGuest {
    // For LXC, the hostname is in the config
    let hostname = match client.get_lxc_config(node, ct.vmid).await {
        Ok(config) => config.hostname.or_else(|| ct.name.clone()),
        Err(_) => ct.name.clone(),
    };

    DiscoveredGuest {
        node: node.to_string(),
        vmid: ct.vmid,
        guest_type: "lxc",
        name: ct.name,
        status: ct.status,
        hostname,
        ip_addresses: vec![],
    }
}

/// Persist discovered guests to the `proxmox_host_mappings` table.
///
/// Uses upsert semantics: existing mappings for the same
/// `(plugin_config_id, node, vmid)` are updated; new ones are inserted.
/// Returns the number of upserted rows.
pub async fn persist_discovered_guests(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
    guests: &[DiscoveredGuest],
) -> Result<usize> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    let now = OffsetDateTime::now_utc();
    let mut count = 0usize;

    for guest in guests {
        let ip_json = if guest.ip_addresses.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&guest.ip_addresses).unwrap_or_default())
        };

        // Check for existing mapping
        let existing = proxmox_host_mapping::Entity::find()
            .filter(proxmox_host_mapping::Column::PluginConfigId.eq(plugin_config_id))
            .filter(proxmox_host_mapping::Column::ProxmoxNode.eq(&guest.node))
            .filter(proxmox_host_mapping::Column::ProxmoxVmid.eq(guest.vmid as i32))
            .one(db)
            .await
            .map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to query existing mapping: {e}"
                )))
            })?;

        if let Some(existing) = existing {
            // Update existing mapping
            let mut active: proxmox_host_mapping::ActiveModel = existing.into();
            active.proxmox_name = Set(guest.name.clone());
            active.proxmox_status = Set(guest.status.clone());
            active.hostname = Set(guest.hostname.clone());
            active.ip_addresses = Set(ip_json);
            active.updated_at = Set(now);
            active.update(db).await.map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to update mapping: {e}"
                )))
            })?;
        } else {
            // Insert new mapping
            let active = proxmox_host_mapping::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                plugin_config_id: Set(plugin_config_id),
                host_id: Set(None),
                proxmox_node: Set(guest.node.clone()),
                proxmox_vmid: Set(guest.vmid as i32),
                proxmox_type: Set(guest.guest_type.to_string()),
                proxmox_name: Set(guest.name.clone()),
                proxmox_status: Set(guest.status.clone()),
                hostname: Set(guest.hostname.clone()),
                ip_addresses: Set(ip_json),
                match_method: Set(None),
                discovered_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(db).await.map_err(|e| {
                rootcause::report!(ProxmoxError::Database(format!(
                    "failed to insert mapping: {e}"
                )))
            })?;
        }

        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_guest_debug() {
        let guest = DiscoveredGuest {
            node: "pve1".to_string(),
            vmid: 100,
            guest_type: "qemu",
            name: Some("web-server".to_string()),
            status: "running".to_string(),
            hostname: Some("web-server".to_string()),
            ip_addresses: vec!["10.0.0.1".to_string()],
        };
        let debug = format!("{guest:?}");
        assert!(debug.contains("web-server"));
        assert!(debug.contains("10.0.0.1"));
    }
}
