---
title: Proxmox VE Integration
weight: 190
description: Uptrakit can discover virtual machines and containers from a Proxmox VE cluster and link them to managed hosts.
---

# Proxmox VE Integration

Uptrakit can discover virtual machines and containers from a Proxmox VE
cluster and link them to managed hosts.

See also: [Plugin Configurations](plugin-configs.md),
[Proxmox Plugin Development](../development/proxmox-plugin.md).

## Overview

The Proxmox VE plugin connects to your Proxmox cluster's REST API to discover
QEMU VMs and LXC containers. Once discovered, you can manually link them to
existing Uptrakit-managed hosts to see Proxmox metadata alongside update
information.

This is Phase 1 — discovery and manual matching. Future phases will add
pre-update snapshot creation and rollback capabilities.

## Setup

### Prerequisites

- A Proxmox VE cluster (version 7.x or 8.x)
- An API token with at least read access to nodes and VMs/CTs

### Creating a Proxmox API Token

1. In the Proxmox web UI, go to **Datacenter > Permissions > API Tokens**
1. Click **Add** and create a token for an existing user
1. Note the token ID and secret — you will need the full string in the format
   `USER@REALM!TOKENID=SECRET`

> **Tip**: For discovery-only access, the token needs `PVEAuditor` role
> privileges on `/` (root path). This grants read-only access to all nodes,
> VMs, and containers without allowing any modifications.

### Adding the Plugin Configuration

Create a plugin configuration via the API or CLI:

```bash
uptrakit plugin-configs create \
  --name "My PVE Cluster" \
  --plugin-type infrastructure_proxmox \
  --config '{
    "api_url": "https://pve.local:8006",
    "api_token": "root@pam!uptrakit=your-secret-here",
    "verify_tls": false
  }'
```

#### Configuration Fields

| Field | Required | Default | Description |
| --- | --- | --- | --- |
| `api_url` | Yes | — | Proxmox VE API URL (must be HTTPS) |
| `api_token` | Yes | — | API token in `USER@REALM!TOKENID=SECRET` format |
| `verify_tls` | No | `true` | Set to `false` for self-signed certificates |
| `node_filter` | No | `[]` | Restrict discovery to specific node names |

## Usage

All Proxmox operations are accessed through shared surfaces.

### Testing the Connection

```bash
uptrakit surfaces invoke proxmox.hosts test-connection \
  --params '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

### Discovering VMs and Containers

```bash
uptrakit surfaces invoke proxmox.hosts discover \
  --params '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

This queries all online nodes (or filtered nodes) and lists their QEMU VMs
and LXC containers. For running QEMU VMs, it also attempts to query the
guest agent for IP address information.

### Listing Discovered Guests

```bash
uptrakit surfaces invoke proxmox.hosts list \
  --params '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

### Matching to Uptrakit Hosts

Matching is manual — you explicitly link a discovered Proxmox guest to an
Uptrakit host:

```bash
uptrakit surfaces invoke proxmox.hosts match \
  --params '{"mapping_id": "MAPPING_ID", "host_id": "HOST_ID"}'
```

To remove a match:

```bash
uptrakit surfaces invoke proxmox.hosts unmatch \
  --params '{"mapping_id": "MAPPING_ID"}'
```

### Viewing Proxmox Info for a Host

```bash
uptrakit surfaces invoke proxmox.host-info get-info \
  --params '{"host_id": "HOST_ID"}'
```

## Node Filtering

To restrict discovery to specific Proxmox nodes, set the `node_filter` field
in your plugin configuration:

```json
{
  "api_url": "https://pve.local:8006",
  "api_token": "root@pam!uptrakit=secret",
  "node_filter": ["pve1", "pve3"]
}
```

Only nodes listed in the filter will be queried. An empty array (the default)
means all online nodes are included.

## Bootstrapping Guests via SSH Agent

When the SSH agent bootstraps a Proxmox VE node (via regular SSH bootstrap), it
automatically detects PVE and creates tenant-scoped API credentials
(`uptrakit-{tenant_id}@pve`). If the cluster already has credentials for the
same tenant (from bootstrapping another node in the same cluster), the existing
configuration is reused automatically. This enables a second bootstrap mode:
bootstrapping guests (LXC containers and QEMU VMs) directly through the PVE
node without needing SSH access to the guest.

### How It Works

1. **Bootstrap the PVE node** — Use the regular SSH bootstrap to set up the PVE
   host. The agent detects PVE automatically and creates an API user/token.
2. **Bootstrap guests** — Use the "Bootstrap via Proxmox" action in the UI or
   CLI. The agent SSHs to the PVE node and runs commands inside the guest via
   `pct exec` (LXC) or `qm guest exec` (QEMU).

### Bootstrap via Proxmox Action

Available in the SSH Hosts surface page when at least one PVE node has been
bootstrapped.

| Field | Required | Default | Description |
| --- | --- | --- | --- |
| PVE Host | Yes | — | PVE node to use as gateway (select from bootstrapped nodes) |
| Guest VMID | Yes | — | VMID of the target container or VM |
| Guest Type | Yes | `lxc` | LXC Container or QEMU VM |
| Host Name | Yes | — | Friendly name for identification |
| Target Username | No | `uptrakit` | User to create/use in the guest |
| Allow All | No | `false` | Use NOPASSWD: ALL in sudoers |

### What Happens During Guest Bootstrap

1. Connects to the PVE node via SSH
2. Creates the target user inside the guest
3. Deploys an SSH key to the guest's `authorized_keys`
4. Configures sudoers for the target user
5. Retrieves the guest's IP address
6. Verifies direct SSH connectivity to the guest
7. Saves the host entry to the local database

After bootstrap, the guest is managed like any other SSH host — the agent
connects directly to it for version checks and updates.

### Bootstrap via Discovered Guest

The "Bootstrap via Discovered Guest" action matches guests discovered by the
Proxmox plugin to PVE hosts using the stored **PVE node name** and
**plugin config ID**. This requires that PVE hosts have been synced (via the
**Sync Host** action or during initial bootstrap) so the short node name (e.g.
`optiplex2`) is stored in the local database. Without a stored node name,
matching will fail.

If matching fails, use the **Sync Host** row action in the web UI (or run
`uptrakit surfaces ssh-agent.hosts --target-provider-id <PROVIDER_ID> sync-host <host-id>`)
to populate the node name, then retry.

## Security Considerations

- The API token secret is stored encrypted at rest and masked in API responses
- HTTPS is required for the API URL — HTTP connections are rejected
- Private and loopback addresses are allowed since Proxmox is typically
  deployed on-premise
- TLS verification can be disabled for self-signed certificates common in PVE
  installations
