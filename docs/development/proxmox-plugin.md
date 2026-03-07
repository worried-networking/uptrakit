# Proxmox VE Plugin — Development Guide

This document covers the internals of the Proxmox VE infrastructure plugin.

See also: [End-user Guide](../end-user/proxmox.md),
[Plugin Guidelines](plugin-guidelines.md),
[Extensions](extensions.md).

## Crate Location

`crates/plugins/infrastructure/proxmox/` — crate name
`uptrakit-plugin-infrastructure-proxmox`.

## Architecture

The Proxmox VE plugin is a **primarily controller-side** infrastructure plugin.
Most operations go through the Proxmox VE REST API.

The plugin also provides agent-side modules (`pve_setup`, `guest_exec`) used by
the SSH agent for PVE node detection and guest command execution during bootstrap.

```text
Controller
 ├── PluginRegistry
 │    └── InfrastructureProxmox (registered via register_plugins!)
 ├── ExtensionRegistry
 │    └── proxmox.hosts (Page), proxmox.host-info (Panel)
 └── Extension action dispatch
      └── proxmox::extensions::handle_action()
           ├── list      → DB query
           ├── discover  → ProxmoxClient → persist_discovered_guests()
           ├── test-connection → ProxmoxClient::test_connection()
           ├── match     → matching::manual_match()
           ├── unmatch   → matching::unmatch()
           └── get-info  → DB query
```

## Module Structure

| Module | Purpose |
| --- | --- |
| `config.rs` | `ProxmoxConfig` — API URL, token, TLS, node filter |
| `error.rs` | `ProxmoxError` enum with `impl_report_conversion!` |
| `client.rs` | `ProxmoxClient` — HTTP client for Proxmox REST API |
| `api_types.rs` | Serde structs for PVE API JSON responses |
| `plugin.rs` | `ProxmoxPlugin` — `Plugin` trait impl (empty capabilities) |
| `discovery.rs` | `discover_guests()` — queries nodes for VMs/CTs |
| `matching.rs` | `manual_match()` / `unmatch()` — manual-only host matching |
| `extensions.rs` | Extension manifests + action handler dispatch |
| `pve_setup.rs` | PVE node detection and API credential creation (agent-side) |
| `guest_exec.rs` | Guest command execution via `pct exec` / `qm guest exec` (agent-side) |

## Proxmox API Client

`ProxmoxClient` communicates with the Proxmox VE REST API using
`PVEAPIToken=USER@REALM!TOKENID=SECRET` authentication.

- Connect timeout: 10 seconds
- Request timeout: 60 seconds
- TLS verification: configurable (`verify_tls`, default `true`)
- All responses unwrap the `{"data": T}` wrapper

### Endpoints Used

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api2/json/nodes` | List cluster nodes |
| GET | `/api2/json/nodes/{node}/qemu` | List QEMU VMs |
| GET | `/api2/json/nodes/{node}/lxc` | List LXC containers |
| GET | `/api2/json/nodes/{node}/qemu/{vmid}/config` | QEMU VM config |
| GET | `/api2/json/nodes/{node}/lxc/{vmid}/config` | LXC container config |
| GET | `/api2/json/nodes/{node}/qemu/{vmid}/agent/network-get-interfaces` | Guest agent network info |
| GET | `/api2/json/version` | API version (connectivity test) |

## Database

### Table: `proxmox_host_mappings`

Stores discovered VMs/CTs and their optional link to an Uptrakit host.

| Column | Type | Description |
| --- | --- | --- |
| `id` | UUID PK | Row identifier |
| `tenant_id` | UUID FK(tenants) | Tenant scope |
| `plugin_config_id` | UUID FK(plugin_configs) | Source plugin configuration |
| `host_id` | UUID FK(hosts) NULL | Matched Uptrakit host (NULL = unmatched) |
| `proxmox_node` | TEXT | Proxmox node name |
| `proxmox_vmid` | INTEGER | VM/CT identifier |
| `proxmox_type` | TEXT | `"qemu"` or `"lxc"` |
| `proxmox_name` | TEXT NULL | Guest name |
| `proxmox_status` | TEXT | Current status |
| `hostname` | TEXT NULL | Hostname from config |
| `ip_addresses` | TEXT NULL | JSON array of IPs |
| `match_method` | TEXT NULL | `"manual"` or NULL |
| `discovered_at` | TIMESTAMP | First discovery time |
| `updated_at` | TIMESTAMP | Last update time |

**Unique constraint**: `(plugin_config_id, proxmox_node, proxmox_vmid)`

The entity implements `TenantScoped` for automatic tenant isolation.

## Host Matching

Only **manual matching** is supported. Auto-matching by hostname or IP is not
implemented because no reliable stable identifier (such as `machine_id`) is
available through the Proxmox VE REST API.

Users manually link discovered Proxmox guests to Uptrakit hosts via the
`match` extension action or through the UI.

## Extension Actions

All actions are dispatched through the Extensions framework. No dedicated CLI
commands or REST routes exist.

| Extension | Action | Parameters | Description |
| --- | --- | --- | --- |
| `proxmox.hosts` | `list` | `plugin_config_id` | List discovered guests |
| `proxmox.hosts` | `discover` | `plugin_config_id` | Trigger discovery |
| `proxmox.hosts` | `test-connection` | `plugin_config_id` | Test API connectivity |
| `proxmox.hosts` | `match` | `mapping_id`, `host_id` | Manual match |
| `proxmox.hosts` | `unmatch` | `mapping_id` | Remove match |
| `proxmox.host-info` | `get-info` | `host_id` | Get Proxmox info for host |

The `plugin_config_id` parameter is **not included in action forms**. Instead, the
`proxmox.hosts` data table uses a `context_selector` that asks the user to pick a
Proxmox VE configuration before any data loads. The selected ID is automatically
injected into all action invocations by the frontend.

### Adding a configuration from the UI

The `context_selector.add_action` uses `ApiSubmitDef` to route form submission
directly to `POST /api/v1/plugin-configs`, bypassing the extension proxy. No
extension-side handler for config creation is needed. After the REST call succeeds,
the frontend refreshes the selector options and auto-selects the new configuration
(identified via `response_id_field: "id"`).

## Configuration Validation

- `api_url` must be HTTPS with a host (private/loopback hosts allowed)
- `api_token` must match PVE format: `USER@REALM!TOKENID=SECRET`
- Secret masking: `api_token` is replaced with `***` in API responses

## Agent-Side Modules

The Proxmox plugin also provides modules used by the SSH agent for PVE node
detection and guest command execution. These modules have no dependency on the
Proxmox REST API client — they operate via SSH commands on the PVE node.

### `pve_setup.rs` — PVE Detection and Credential Creation

Used during SSH agent bootstrap to detect and configure PVE nodes.

| Function | Description |
| --- | --- |
| `detect_pve_node(executor)` | Runs `command -v pveversion` to detect a PVE node |
| `check_pve_token_exists(executor, tenant_id)` | Checks whether an Uptrakit PVE token already exists on the cluster and determines ownership |
| `create_pve_api_credentials(executor, tenant_id)` | Creates a PVE API user and token via `pveum` commands |
| `pve_user_realm(tenant_id)` | Returns the tenant-specific PVE username: `uptrakit-{tenant_id}@pve` |

#### PVE Cluster Deduplication

PVE clusters share a cluster-wide user and token database. If Uptrakit has
already been installed on another node in the same cluster, creating a second
token would fail. The agent performs a pre-flight check via
`check_pve_token_exists()` before attempting credential creation:

1. Lists all PVE users via `pveum user list --output-format json`
2. Looks for users matching the `uptrakit-*@pve` pattern
3. Returns a `PveTokenStatus`:
   - `NotFound` — no Uptrakit token exists; safe to create
   - `OwnedByTenant(username)` — token belongs to the current tenant; reuse existing config
   - `OwnedByOtherTenant(username)` — token belongs to a different tenant; bootstrap fails with error

When the token is owned by the current tenant, the agent looks up the existing
`pve_plugin_config_id` from a previously bootstrapped host and reuses it
instead of creating duplicate credentials.

#### Tenant-Scoped PVE Credentials

PVE API users are named per-tenant: `uptrakit-{tenant_id}@pve`. This ensures
that each tenant's credentials are isolated and identifiable. The tenant ID is
received via `ServiceSettingsPayload.tenant_id` from the controller.

`create_pve_api_credentials` performs three steps:

1. `pveum user add uptrakit-{tenant_id}@pve` — creates the API user (ignores "already exists")
2. `pveum user token add uptrakit-{tenant_id}@pve uptrakit --privsep=0 --output-format json` — creates a token
3. `pveum acl modify / --users uptrakit-{tenant_id}@pve --roles PVEAuditor` — grants read-only access

Returns `PveCredentials { api_url, api_token }` where `api_url` is derived from
the PVE node's hostname (`https://{hostname}:8006`). The `api_url` does **not**
include the `/api2/json` path — that prefix is added per-request by `ProxmoxClient`.

### `guest_exec.rs` — Guest Command Execution

Provides transport-agnostic guest command execution via PVE CLI tools.

| Function | Description |
| --- | --- |
| `exec_in_guest(executor, vmid, guest_type, command)` | Execute a command inside a PVE guest |
| `get_guest_ip(executor, vmid, guest_type)` | Get the primary IP address of a guest |
| `list_guests(executor)` | List all guests on the cluster |

**LXC** commands use `pct exec {vmid} -- bash -c '{command}'`.

**QEMU** commands use `qm guest exec {vmid} -- bash -c '{command}'` and parse
the JSON output for stdout/stderr/exit code.

| Type | Description |
| --- | --- |
| `PveGuestType` | Enum: `Lxc`, `Qemu` |
| `GuestExecResult` | Command output: `stdout`, `stderr`, `exit_code` |
| `PveGuest` | Guest metadata: `vmid`, `name`, `guest_type`, `status`, `node` |

### `RemoteExecutor` Integration

Both modules accept `&dyn RemoteExecutor` (defined in `uptrakit-command`), making
them testable with mock executors and usable with any SSH transport. The SSH agent
provides two implementations:

- `SshRemoteExecutor` — wraps `Arc<SshSession>` for direct SSH commands
- `PveGuestExecutor` — wraps SSH-to-PVE-node, routes commands through `exec_in_guest()`

These are defined in `crates/core/agent-ssh/src/remote_exec.rs`.

## Testing

Unit tests cover config validation, API type deserialization, client creation,
extension manifest construction, and UUID parameter parsing.

Integration tests against a real Proxmox VE instance are not included in CI —
they require access to a PVE cluster with valid API credentials.
