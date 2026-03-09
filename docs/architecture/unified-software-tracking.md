# Unified software tracking

Uptrakit uses a single `software_items` table to track all software across hosts. Each item
carries a `featured` flag that controls its visibility model:

- **`featured: true`** -- prominent individual entities in the main Software list and in MQTT/HA.
  Plugins like Docker and Proxmox Helper Scripts set this flag because users care about each
  discovered item individually.

- **`featured: false`** -- aggregated per-host summary view. Package manager plugins (APT,
  Homebrew, npm, Mac App Store) set this because they discover hundreds of system packages
  where an aggregate "N updates available" view is more practical than individual entities.

There is no approval workflow. All discovered software is tracked immediately upon discovery.

## Design rationale

The previous architecture maintained two parallel tracking systems: `software_items` (with a
pending/approval workflow) and `host_packages` (immediate tracking, per-host only). This caused
duplicated code paths, separate update history tables, separate ignore tables, and confusion
about which system tracked what.

The unified model eliminates the dual-system complexity:

- One table, one set of ignore rules, one update history table.
- The `featured` flag replaces the `TrackingSystem` enum routing.
- No `discovery_state` or pending/approval workflow -- discovery results create enabled items
  immediately.
- Batch updates work identically for featured and non-featured items.

## Database tables

### `software_items`

```text
software_items
+-- id (UUID PK)
+-- tenant_id -> tenants
+-- name (TEXT)
+-- featured (BOOL, default FALSE)
+-- enabled (BOOL, default TRUE)
+-- last_checked_at (TIMESTAMPTZ nullable)
+-- created_at, updated_at (TIMESTAMPTZ)
+-- deactivated_at (TIMESTAMPTZ nullable)
+-- UNIQUE (tenant_id, name) WHERE deactivated_at IS NULL
```

The `featured` field is immutable after creation -- it is set by the discovering plugin and
not changed by user actions. The `enabled` field defaults to `true` for all items (no pending
state).

### `host_software_items`

The junction table linking software items to hosts, with per-host version tracking.

```text
host_software_items
+-- host_id -> hosts (PK component)
+-- software_item_id -> software_items (PK component)
+-- plugin_config_id -> plugin_configs (nullable)
+-- package_identifier (TEXT nullable)
+-- installed_version (TEXT nullable)
+-- installed_version_detected_at (TIMESTAMPTZ nullable)
+-- latest_version (TEXT nullable)
+-- latest_version_fetched_at (TIMESTAMPTZ nullable)
+-- latest_release_metadata (JSONB nullable)
+-- update_category (TEXT, default 'unknown')
+-- last_updated_at (TIMESTAMPTZ nullable)
+-- linked_at (TIMESTAMPTZ)
+-- deactivated_at (TIMESTAMPTZ nullable)
```

The `plugin_config_id` and `package_identifier` columns support non-featured items that use a
single plugin config for all roles (package managers). Featured items continue to use
role-based plugin assignments in `host_software_item_plugins`.

### `software_ignores`

Replaces both the former `autodiscovery_ignores` and `host_package_ignores` tables. A single
table handles both tenant-wide and host-specific ignore rules.

```text
software_ignores
+-- id (UUID PK)
+-- tenant_id -> tenants
+-- host_id -> hosts (nullable; NULL = tenant-wide, non-NULL = host-specific)
+-- plugin_config_id -> plugin_configs (nullable)
+-- name (TEXT nullable)
+-- package_identifier (TEXT nullable)
+-- created_at (TIMESTAMPTZ)
```

- **Tenant-wide rule** (`host_id IS NULL`): suppresses discovery of the named software across
  all hosts and plugin configs.
- **Host-specific rule** (`host_id IS NOT NULL`): suppresses discovery on a specific host,
  scoped to a plugin config and package identifier.

### `update_history`

Gained two columns:

- `tenant_id` (UUID FK) -- direct tenant scoping (previously implicit via `host_id`).
- `host_software_item_id` (UUID FK, nullable) -- links to the specific host-software junction
  row. Nullable for backward compatibility with records created before the column existed.

The `to_version` and `started_at` columns are now nullable to support batch updates where the
target version is not known upfront.

### `update_batches`

Gained two columns for batch output streaming:

- `output` (TEXT nullable) -- accumulated batch output text.
- `output_bytes` (BIGINT, default 0) -- byte count of accumulated output.

## Discovery flow

1. An agent connects and registers a host (or a periodic `discover_software` task fires).
2. The agent runs all applicable discovery plugins and reports results.
3. For each discovered item, the controller:
   - Checks the `software_ignores` table -- skips if an ignore rule matches.
   - Finds or creates a `software_items` row with `featured` set by the plugin.
   - Creates or updates the `host_software_items` junction row with version data.
   - For featured items with discovery targets: creates role-based plugin assignments.
   - For non-featured items: stores `plugin_config_id` and `package_identifier` directly
     on the junction row.
4. All items are created with `enabled: true` -- no pending state, no approval step.

## Plugin `featured` assignment

| Plugin | `featured` | Rationale |
| :--- | :---: | :--- |
| Docker | `true` | Users care about individual container images |
| Proxmox Helper Scripts | `true` | Users care about individual PHS-managed apps |
| APT | `false` | Hundreds of system packages; aggregate view preferred |
| Homebrew | `false` | Dozens to hundreds of packages; aggregate view preferred |
| npm | `false` | Global npm packages; aggregate view preferred |
| Mac App Store | `false` | App Store apps; aggregate view preferred |

## Visibility model

### Featured items (`featured: true`)

- Appear in the main **Software** list in the Web UI.
- Each `(software_item, host)` pair creates an individual MQTT `update` entity in Home
  Assistant.
- Per-host version tracking with full role-based plugin assignments.
- Individual update triggers via REST, CLI, and MQTT.

### Non-featured items (`featured: false`)

- Appear in the **host detail** view as an aggregated packages summary.
- Two per-host MQTT entities summarize all non-featured items: "all packages" and
  "security updates".
- Version tracking via `plugin_config_id` and `package_identifier` on the junction row.
- Batch updates via `execute_batch_update` grouped by plugin type.

## Batch updates

Package managers execute a single bulk command (e.g., `apt-get upgrade`, `brew upgrade pkg1
pkg2`) instead of per-package calls. The `Plugin` trait provides `execute_batch_update()` with
a default sequential fallback.

### Wire protocol

- `ControllerMessage::ExecuteBatchUpdate` -- sends batch of packages grouped by plugin type.
- `ServiceMessage::BatchUpdateResult` -- per-package outcomes.

## Host update summary

`HostResponse` includes aggregate update counts computed from non-featured software items on
the host:

- `available_updates_count` -- items where installed version differs from latest version.
- `security_updates_count` -- subset where `update_category = 'security'`.

These counts appear in the hosts list table and host detail page.

## MQTT integration

The `software_states` wire message carries:

- `items` -- featured software items with per-host version data (individual MQTT entities).
- `host_summaries` -- per-host aggregates for non-featured items (summary MQTT entities).

## Permissions

All software items (featured and non-featured) use the same permissions:

- `ViewSoftware` -- view items, versions, and update history.
- `ManageSoftware` -- create, update, delete, trigger updates, manage ignore rules.
- `ViewHosts` -- see aggregate update counts on host list.

## Related documentation

- [Software item entity](software-item-entity.md) -- table schema and response types
- [Update history entity](update-history-entity.md) -- update tracking
- [Autodiscovery (end-user)](../end-user/autodiscovery.md) -- discovery workflow
- [Plugin guidelines](../development/plugin-guidelines.md) -- plugin discovery contract
- [Wire protocol](../api/wire-protocol.md) -- batch update messages
- [Home Assistant MQTT](../end-user/home-assistant-mqtt.md) -- MQTT entity model
