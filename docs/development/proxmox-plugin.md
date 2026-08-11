# Proxmox VE Plugin — Development Guide

This document covers the internals of the Proxmox VE infrastructure plugin.

See also: [End-user Guide](../end-user/proxmox.md),
[Plugin Guidelines](plugin-guidelines.md),
[Extensions](extensions.md).

## Crate Location

`crates/plugins/infrastructure/proxmox/` — crate name
`uptrakit-plugin-infrastructure-proxmox`.

## Architecture

`ProxmoxPlugin` is a **unified plugin struct** used on both the controller and
agent sides. On the controller (created via `ProxmoxPlugin::new(config, executor)`)
it holds a `ProxmoxConfig` and communicates with the Proxmox VE REST API. On
the agent (created via `ProxmoxPlugin::new_agent()`) it implements infrastructure
role traits (`HostLifecycle`, `HostReport`, `GuestExec`) behind
the `agent-infra` feature gate.

Feature-gated capabilities:

- **No features**: empty capabilities
- **`migrations`** (without `agent-infra`): `ControllerMigrations`
- **`agent-infra`** (implies `migrations`): `HostLifecycle`, `HostReport`,
  `GuestExec`, `ServiceMigrations`

```text
Controller
 ├── PluginCatalog
 │    └── InfrastructureProxmox (registered via declare_plugin! + all_descriptors())
 ├── SurfaceRegistry
 │    └── proxmox.hosts (Page), proxmox.host-info (Panel)
 └── Surface action dispatch
      └── proxmox::surfaces::handle_surface_action()
           ├── mappings  → DB query
           ├── discover  → ProxmoxClient → persist_discovered_guests()
           ├── test-connection → ProxmoxClient::test_connection()
           ├── match     → matching::manual_match()
           ├── unmatch   → matching::unmatch()
           └── info      → DB query
```

## Module Structure

| Module                     | Purpose                                                                                                                                                 |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `config.rs`                | `ProxmoxConfig` — API URL, token, TLS, node filter                                                                                                      |
| `error.rs`                 | `ProxmoxError` enum with `impl_report_conversion!`                                                                                                      |
| `client.rs`                | `ProxmoxClient` — HTTP client for Proxmox REST API                                                                                                      |
| `api_types.rs`             | Serde structs for PVE API JSON responses                                                                                                                |
| `plugin.rs`                | `ProxmoxPlugin` — unified `PluginMeta` + role trait impls (controller + agent)                                                                          |
| `agent/plugin.rs`          | Role trait impls (`HostLifecycle`, `HostReport`, `GuestExec`) on `ProxmoxPlugin`                                                                        |
| `agent/surface_actions.rs` | Dispatch shim over the plugin's `agent_interactions()` table (`discovered-guests`, `bootstrap-proxmox-guest`); implementation fns for both interactions |
| `agent/db_ops.rs`          | Agent-local DB operations (PVE host state, pending matches)                                                                                             |
| `agent/migration.rs`       | Agent-local DB migrations (`proxmox_host_state`, `proxmox_pending_matches`)                                                                             |
| `discovery.rs`             | `discover_guests()` — queries nodes for VMs/CTs                                                                                                         |
| `matching.rs`              | `manual_match()` / `unmatch()` — manual-only host matching                                                                                              |
| `surfaces.rs`              | Surface action definitions + handler dispatch                                                                                                           |
| `pve_setup.rs`             | PVE node detection and API credential creation (agent-side)                                                                                             |
| `guest_exec.rs`            | Guest command execution via `pct exec` / `qm guest exec` (agent-side)                                                                                   |

## Proxmox API Client

`ProxmoxClient` communicates with the Proxmox VE REST API using
`PVEAPIToken=USER@REALM!TOKENID=SECRET` authentication.

- Connect timeout: 10 seconds
- Request timeout: 60 seconds
- TLS verification: configurable (`verify_tls`, default `true`)
- DNS resolution: `SsrfSafeResolver::permissive()` because Proxmox is intentionally a
  self-hosted/private-network control plane
- All responses unwrap the `{"data": T}` wrapper

When `verify_tls = false`, the client emits a warning at construction time. This setting is
intended only for controlled bootstrap scenarios with self-signed certificates.

### Endpoints Used

| Method | Path                                                               | Purpose                         |
| ------ | ------------------------------------------------------------------ | ------------------------------- |
| GET    | `/api2/json/nodes`                                                 | List cluster nodes              |
| GET    | `/api2/json/nodes/{node}/qemu`                                     | List QEMU VMs                   |
| GET    | `/api2/json/nodes/{node}/lxc`                                      | List LXC containers             |
| GET    | `/api2/json/nodes/{node}/qemu/{vmid}/config`                       | QEMU VM config                  |
| GET    | `/api2/json/nodes/{node}/lxc/{vmid}/config`                        | LXC container config            |
| GET    | `/api2/json/nodes/{node}/qemu/{vmid}/agent/network-get-interfaces` | Guest agent network info        |
| GET    | `/api2/json/version`                                               | API version (connectivity test) |

## Database

### Table: `proxmox_host_mappings`

Stores discovered VMs/CTs and their optional link to an Uptrakit host.

| Column             | Type                    | Description                              |
| ------------------ | ----------------------- | ---------------------------------------- |
| `id`               | UUID PK                 | Row identifier                           |
| `tenant_id`        | UUID FK(tenants)        | Tenant scope                             |
| `plugin_config_id` | UUID FK(plugin_configs) | Source plugin configuration              |
| `host_id`          | UUID FK(hosts) NULL     | Matched Uptrakit host (NULL = unmatched) |
| `proxmox_node`     | TEXT                    | Proxmox node name                        |
| `proxmox_vmid`     | INTEGER                 | VM/CT identifier                         |
| `proxmox_type`     | TEXT                    | `"qemu"` or `"lxc"`                      |
| `proxmox_name`     | TEXT NULL               | Guest name                               |
| `proxmox_status`   | TEXT                    | Current status                           |
| `hostname`         | TEXT NULL               | Hostname from config                     |
| `ip_addresses`     | TEXT NULL               | JSON array of IPs                        |
| `match_method`     | TEXT NULL               | `"manual"` or NULL                       |
| `discovered_at`    | TIMESTAMP               | First discovery time                     |
| `updated_at`       | TIMESTAMP               | Last update time                         |

**Unique constraint**: `(plugin_config_id, proxmox_node, proxmox_vmid)`

The entity implements `TenantScoped` for automatic tenant isolation.

## Host Matching

Only **manual matching** is supported. Auto-matching by hostname or IP is not
implemented because no reliable stable identifier (such as `machine_id`) is
available through the Proxmox VE REST API.

Users manually link discovered Proxmox guests to Uptrakit hosts via the
`match` surface action or through the UI.

## Surface Actions

All actions are dispatched through the shared-surface runtime. No dedicated CLI
commands or REST routes exist.

| Surface             | Action             | Parameters                                  | Description                                                          |
| ------------------- | ------------------ | ------------------------------------------- | -------------------------------------------------------------------- |
| `proxmox.hosts`     | `mappings` (GET)   | `plugin_config_id`                          | List discovered guests with inline match suggestions                 |
| `proxmox.hosts`     | `discover`         | `plugin_config_id`                          | Trigger discovery                                                    |
| `proxmox.hosts`     | `test-connection`  | `plugin_config_id`                          | Test API connectivity                                                |
| `proxmox.hosts`     | `match`            | `mapping_id`, `host_id`                     | Manual match                                                         |
| `proxmox.hosts`     | `approve-match`    | `mapping_id`, `host_id`/`suggested_host_id` | Approve a suggested match (accepts `suggested_host_id` as fallback)  |
| `proxmox.hosts`     | `unmatch`          | `mapping_id`                                | Remove match (destructive, confirmation dialog shows `proxmox_name`) |
| `proxmox.hosts`     | `unmatched-guests` | (none)                                      | List unmatched guests sorted by name across all configs              |
| `proxmox.host-info` | `info` (GET)       | `host_id`                                   | Get Proxmox info for host                                            |

In the current shared-surface slice, `proxmox.hosts` is intentionally **not**
rendered as the old selector-driven data table. The page currently exposes only
the **Add Configuration** action and a boundary callout explaining that selector
semantics are not modeled yet in the shared renderer.

### Adding a configuration from the UI

The **Add Configuration** action uses the shared-surface controller-local path
to create the plugin configuration. The selector-driven host table is still
deferred until shared-surface selector semantics exist, so there is currently
no selector refresh/auto-select flow on this page.

### Row action visibility

Row actions use `row_visible_when` to conditionally show/hide buttons based on
row data:

- **Approve Match**: visible only when `suggested_host_id` is present (a match
  suggestion exists for the row).
- **Remove Match**: visible only when `matched_host` is present (the row is
  already matched to a host).
- **Manual Match**: always visible (no condition).

### Cross-config guest listing

The `unmatched-guests` action returns unmatched guests across **all** Proxmox
configurations for the tenant, sorted by name (case-insensitive) with VMID as a
tiebreaker. Unlike `list` (which requires a `plugin_config_id`), this action is
designed for service-initiated invocations where the calling service does not
know which Proxmox configs exist.

The response includes `hostname`, `plugin_config_id`, and other guest metadata fields.

The SSH agent uses this action via the controller-managed surface invocation path to populate the
`bootstrap-proxmox-guest` dropdown. If the Proxmox plugin is not installed, the
service-initiated request returns an error and the SSH agent returns an empty options
list.

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

| Function                                          | Description                                                                                 |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `detect_pve_node(executor)`                       | Runs `command -v pveversion` to detect a PVE node                                           |
| `check_pve_token_exists(executor, tenant_id)`     | Checks whether an Uptrakit PVE token already exists on the cluster and determines ownership |
| `create_pve_api_credentials(executor, tenant_id)` | Creates a PVE API user and token via `pveum` commands                                       |
| `pve_user_realm(tenant_id)`                       | Returns the tenant-specific PVE username: `uptrakit-{tenant_id}@pve`                        |

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

| Function                                             | Description                           |
| ---------------------------------------------------- | ------------------------------------- |
| `exec_in_guest(executor, vmid, guest_type, command)` | Execute a command inside a PVE guest  |
| `get_guest_ip(executor, vmid, guest_type)`           | Get the primary IP address of a guest |
| `list_guests(executor)`                              | List all guests on the cluster        |

**LXC** commands use `pct exec {vmid} -- bash -c '{command}'`.

**QEMU** commands use `qm guest exec {vmid} -- bash -c '{command}'` and parse
the JSON output for stdout/stderr/exit code.

| Type              | Description                                                    |
| ----------------- | -------------------------------------------------------------- |
| `PveGuestType`    | Enum: `Lxc`, `Qemu`                                            |
| `GuestExecResult` | Command output: `stdout`, `stderr`, `exit_code`                |
| `PveGuest`        | Guest metadata: `vmid`, `name`, `guest_type`, `status`, `node` |

### `RemoteExecutor` Integration

Both modules accept `&dyn RemoteExecutor` (defined in `uptrakit-command`), making
them testable with mock executors and usable with any SSH transport. The SSH agent
provides two implementations:

- `SshRemoteExecutor` — wraps `Arc<SshSession>` for direct SSH commands
- `PveGuestExecutor` — wraps SSH-to-PVE-node, routes commands through `exec_in_guest()`

These are defined in `crates/core/agent-ssh/src/remote_exec.rs`.

## Pre-Update Protection Artifacts

Before applying an update, the controller-side protection workflow
(`update_protection.rs`) can create a PVE snapshot or a vzdump backup for the
mapped guest, depending on the effective protection policy (`do_nothing`,
`snapshot`, `backup`). Both artifact kinds are labeled to be human-readable
in the PVE UI.

### Snapshot name scheme

Snapshot names follow `upk-<sanitized-software-name>-<hex8>`, built by
`snapshot_name_for_update_history()`:

- `upk-` — fixed 4-char prefix (also guarantees the first character satisfies
  PVE's snapname regex `[A-Za-z][A-Za-z0-9_\-]*`).
- `<sanitized-software-name>` — the software item's name, lowercased,
  restricted to ASCII alphanumerics with runs of any other character
  collapsed to a single `-` (leading/trailing separators stripped), capped at
  27 characters.
- `-<hex8>` — a literal `-` followed by the first 8 hex characters of the
  update-history row's UUID (simple/no-dashes form).

Total budget: `upk-` (4) + name (<=27) + `-` (1) + hex8 (8) = <=40 characters.

When the software name is unavailable (DB lookup failed or returned no row),
`None` was passed in, or it sanitizes to an empty string (e.g. a fully
non-ASCII name), the scheme falls back to `upk-<hex8>` — no name segment, no
double dash.

### Description and notes content

`snapshot_description()` builds a four-line PVE snapshot `description`;
`backup_notes_template()` builds a single-line vzdump `notes-template`. Both
include the software name, the version transition (`<from> -> <to>`, or just
`-> <to>` when only the target version is known, omitted entirely when
neither is known), the update-history UUID, and a "created automatically by
Uptrakit" framing so operators can identify and safely discard the artifact.

The software name and version strings are sourced from autodiscovery/plugin
detection and are therefore untrusted. Both fields are passed through
`sanitize_label()` before interpolation, which replaces any Unicode control
character (including `\n`, `\r`, `\t`) with a space — this prevents untrusted
metadata from forging an extra line (e.g. a fake `Update ID:` line) in text
an operator reads before deciding to delete the artifact. The vzdump
`notes-template` mechanism additionally treats a literal `{{` in its value as
the start of its own variable-substitution syntax (this is vzdump's own
templating, not ours), so `backup_notes_template()` further escapes each
interpolated value after sanitization: `\` -> `/`, `{` -> `(`, `}` -> `)`.

### Minimum PVE version for backup notes

The `notes-template` parameter on the vzdump backup endpoint requires
**Proxmox VE 7.2 or later**. There is no fallback or retry for older PVE
versions — if the cluster predates 7.2, `start_backup()` fails with an HTTP
400 "parameter verification failed" response mentioning `notes-template`.
This error propagates verbatim into the protection audit row's
`error_message` and into the streamed update output; it is not
uptrakit-specific error text, so operators troubleshooting a backup-mode
failure on an older cluster should recognize this signature.

### Timeout-retry edge case (snapshots)

If a snapshot creation call succeeds but waiting for task completion times
out, the snapshot may already exist on PVE even though the protection
workflow reports failure. On PVE, a duplicate `snapname` is rejected, so a
retry of the same update will fail to create a new snapshot until the
operator manually deletes the stale one.

`update_protection.rs`'s failure-path audit inserts distinguish the two
failure points: the "failed to start" branch (snapshot creation call itself
failed) records `artifact_ref: None`, while the wait-timeout failure branch
records `artifact_ref: Some(snapshot_name)` — the name was already sent to
PVE. Operators troubleshooting a stuck retry should check the protection
audit row's `artifact_ref` field for this specific case.

Unlike the previous opaque `upk-<uuid>` naming scheme, the snapshot name is
**not** recoverable purely from the update-history ID under the new scheme —
it also depends on the software name at generation time (which can be
unavailable, changing the fallback shape) plus a DB-availability fallback.
Always read the persisted `artifact_ref` rather than recomputing the name.

## Testing

Unit tests cover config validation, API type deserialization, client creation,
surface registration construction, UUID parameter parsing, snapshot name
sanitization, and protection artifact description/notes formatting.

Integration tests against a real Proxmox VE instance are not included in CI —
they require access to a PVE cluster with valid API credentials.
