# Host packages entity

Host packages represent system-level software tracked per-host by package manager plugins (APT, Homebrew, npm).
Unlike [software items](software-item-entity.md) which are tracked across hosts (one item, many hosts), host
packages are inherently per-host: "nginx" on host A and "nginx" on host B are independent records.

## Relationship to targeted software items

Uptrakit supports two complementary tracking systems:

| Aspect | Targeted (Software Items) | Host Packages |
| :--- | :--- | :--- |
| **Scope** | Cross-host: one item tracked on N hosts | Per-host: one record per host per package |
| **UI location** | Main Software list | Host detail → Packages tab |
| **Discovery state** | `pending` → requires approval | Created `enabled` immediately |
| **Plugin roles** | Separate `detect_version`, `fetch_releases`, `execute_update` | Single `plugin_config_id` covers all roles |
| **Use case** | Specific items you care about (Docker images, GitHub releases) | Aggregate "does this host have updates?" view |

Both systems coexist independently. The same package can exist in both systems simultaneously (e.g., nginx tracked
as a targeted item AND as a host package). No exclusion rules are needed — updates triggered through either system
work independently.

## `TrackingSystem` enum

The `TrackingSystem` enum (`crates/shared/types/src/tracking_system.rs`) routes discovered software to the correct
system:

- `TrackingSystem::Targeted` → existing `find_or_create_software_item()` path
- `TrackingSystem::HostManaged` → `find_or_create_host_package()` path

Each discovery plugin explicitly sets `tracking_system` on `DiscoveredSoftware`:

- **APT** (discover-all mode): `HostManaged`
- **Homebrew** (no pre-existing config): `HostManaged`
- **npm** (discover-all mode): `HostManaged`
- **Docker**: `Targeted`
- **Proxmox Helper Scripts**: `Targeted`

## Database tables

### `host_packages`

```text
host_packages
├── id (UUID PK)
├── tenant_id → tenants
├── host_id → hosts (CASCADE)
├── plugin_config_id → plugin_configs (RESTRICT)
├── package_identifier (TEXT)
├── name (TEXT)
├── installed_version (TEXT nullable)
├── installed_version_detected_at (TIMESTAMPTZ nullable)
├── latest_version (TEXT nullable)
├── latest_version_fetched_at (TIMESTAMPTZ nullable)
├── latest_release_metadata (JSONB nullable)
├── update_category (TEXT, default 'unknown')
├── enabled (BOOL, default TRUE)
├── last_checked_at (TIMESTAMPTZ nullable)
├── last_updated_at (TIMESTAMPTZ nullable)
├── created_at, updated_at (TIMESTAMPTZ)
├── deactivated_at (TIMESTAMPTZ nullable)
└── UNIQUE (host_id, plugin_config_id, package_identifier) WHERE deactivated_at IS NULL
```

### `host_package_ignores`

Per-host ignore rules preventing re-discovery of dismissed packages.

```text
host_package_ignores
├── id (UUID PK)
├── tenant_id → tenants
├── host_id → hosts (CASCADE)
├── plugin_config_id → plugin_configs (RESTRICT)
├── package_identifier (TEXT)
├── created_at (TIMESTAMPTZ)
└── UNIQUE (host_id, plugin_config_id, package_identifier)
```

### `host_package_update_history`

Separate update history for host package updates.

```text
host_package_update_history
├── id (UUID PK)
├── tenant_id → tenants
├── host_id → hosts
├── host_package_id → host_packages
├── from_version (TEXT nullable)
├── to_version (TEXT nullable)
├── status (TEXT, default 'pending')
├── output (TEXT nullable)
├── output_bytes (BIGINT, default 0)
├── actor_type (TEXT)
├── actor_id (TEXT)
├── update_category (TEXT, default 'unknown')
├── started_at (TIMESTAMPTZ nullable)
├── completed_at (TIMESTAMPTZ nullable)
├── created_at (TIMESTAMPTZ)
├── batch_id → update_batches (nullable)
└── INDEX (host_package_id, status) for in-progress checks
```

## Batch updates

Package managers execute a single bulk command (e.g., `apt-get upgrade`, `brew upgrade pkg1 pkg2`) instead of
per-package calls. The `Plugin` trait provides `execute_batch_update()` with a default sequential fallback.

### Wire protocol

- `ControllerMessage::ExecuteBatchHostPackageUpdate` — sends batch of packages grouped by plugin type
- `ServiceMessage::BatchHostPackageUpdateResult` — per-package outcomes

See [wire-protocol.md](../api/wire-protocol.md) for message details.

### APT batch update strategy

The APT plugin uses `apt_preferences` pin-priority for safe, targeted upgrades:

1. Write a temporary preferences file (user-writable `/tmp/` path) that blocks all upgrades (`Pin-Priority: -1`)
   except specific target packages (`Pin-Priority: 990`)
2. Run `sudo apt-get -o Dir::Etc::Preferences=<temp-path> upgrade --yes`
3. Delete the temporary file

This preserves `auto`/`manual` package marks and is crash-safe (the temp file is not in `/etc/apt/preferences.d/`).

## Host update summary

`HostResponse` includes aggregate update counts computed from `host_packages`:

```rust
pub struct HostUpdateSummary {
    pub available_updates_count: u32,  // host_packages where installed != latest
    pub security_updates_count: u32,   // subset where update_category = 'security'
}
```

These counts appear in the hosts list table and host detail page.

## Version check integration

Host packages are included in the version check cycle alongside targeted software items. The
`VersionCheckAssignment` struct has an optional `host_package_id` field to route results to the correct table.

## Permissions

Host packages reuse existing permissions:

- `ViewSoftware` → view host packages and update history
- `ManageSoftware` → enable/disable, trigger updates, manage ignore rules
- `ViewHosts` → see aggregate update counts on host list

## Related documentation

- [Host entity](host-entity.md) — parent entity
- [Software item entity](software-item-entity.md) — targeted tracking system
- [Plugin guidelines](../development/plugin-guidelines.md) — `execute_batch_update()` and `tracking_system`
- [HTTP API: host packages](../api/host-packages.md) — REST API reference
- [CLI: host-packages](../end-user/cli-usage.md) — CLI command reference
