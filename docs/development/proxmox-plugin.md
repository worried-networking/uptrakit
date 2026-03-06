# Proxmox VE Plugin — Development Guide

This document covers the internals of the Proxmox VE infrastructure plugin.

See also: [End-user Guide](../end-user/proxmox.md),
[Plugin Guidelines](plugin-guidelines.md),
[Extensions](extensions.md).

## Crate Location

`crates/plugins/infrastructure/proxmox/` — crate name
`uptrakit-plugin-infrastructure-proxmox`.

## Architecture

The Proxmox VE plugin is a **controller-side** infrastructure plugin. It has no
agent-side capabilities — all operations go through the Proxmox VE REST API.

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

## Testing

Unit tests cover config validation, API type deserialization, client creation,
extension manifest construction, and UUID parameter parsing.

Integration tests against a real Proxmox VE instance are not included in CI —
they require access to a PVE cluster with valid API credentials.
