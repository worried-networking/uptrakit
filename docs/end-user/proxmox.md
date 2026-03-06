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

All Proxmox operations are accessed through the Extensions framework.

### Testing the Connection

```bash
uptrakit extensions invoke proxmox.hosts test-connection \
  '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

### Discovering VMs and Containers

```bash
uptrakit extensions invoke proxmox.hosts discover \
  '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

This queries all online nodes (or filtered nodes) and lists their QEMU VMs
and LXC containers. For running QEMU VMs, it also attempts to query the
guest agent for IP address information.

### Listing Discovered Guests

```bash
uptrakit extensions invoke proxmox.hosts list \
  '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

### Matching to Uptrakit Hosts

Matching is manual — you explicitly link a discovered Proxmox guest to an
Uptrakit host:

```bash
uptrakit extensions invoke proxmox.hosts match \
  '{"mapping_id": "MAPPING_ID", "host_id": "HOST_ID"}'
```

To remove a match:

```bash
uptrakit extensions invoke proxmox.hosts unmatch \
  '{"mapping_id": "MAPPING_ID"}'
```

### Viewing Proxmox Info for a Host

```bash
uptrakit extensions invoke proxmox.host-info get-info \
  '{"host_id": "HOST_ID"}'
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

## Security Considerations

- The API token secret is stored encrypted at rest and masked in API responses
- HTTPS is required for the API URL — HTTP connections are rejected
- Private and loopback addresses are allowed since Proxmox is typically
  deployed on-premise
- TLS verification can be disabled for self-signed certificates common in PVE
  installations
