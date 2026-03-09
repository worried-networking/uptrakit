# Software item entity

A `SoftwareItem` is a named catalog entry that represents a piece of software to track. It carries no
plugin or package information of its own. Each assignment of a software item to a host is recorded in
the `HostSoftwareItem` junction table, which stores per-host version state (installed version,
latest version, detection timestamps). Plugin coupling is in the separate `HostSoftwareItemPlugin`
table, where each (host, software_item) pair can have separate plugins per role (`detect_version`,
`fetch_releases`, `execute_update`).

This model allows a single "1Password" software item to be tracked across hosts that install it via
different plugins (e.g. Homebrew on one host, APT via a different plugin config on another), without
creating multiple fragmented software item entries. The role-based plugin system further allows each
host to use different plugins for version detection, release fetching, and update execution.

## Database tables

- **`software_items`**: `id` (UUID PK), `tenant_id`, `name`, `enabled` (default `true`),
  `featured` (BOOLEAN, default `true` — controls visibility: featured items appear individually
  in the main Software list; non-featured items appear as aggregated per-host package summaries),
  `last_checked_at?`, `created_at`, `updated_at`, `deactivated_at?`
  - Partial unique index: `uq_software_items_active_name ON (tenant_id, name) WHERE deactivated_at IS NULL`
    — prevents two active items with the same name in a tenant
  - Index: `idx_software_items_deactivated_at`
- **`host_software_items`**: junction table with `id` (UUID PK), `host_id`, `software_item_id`,
  `plugin_config_id?` (UUID FK — for non-featured items, the plugin config used for version
  detection and updates), `package_identifier?` (TEXT — for non-featured items, the
  package-manager-specific identifier), `installed_version?`, `installed_version_detected_at?`,
  `latest_version?`, `latest_version_fetched_at?`, `latest_release_metadata?` (JSON),
  `last_updated_at?`, `update_category?` (TEXT), `linked_at`, `deactivated_at?` (TIMESTAMP —
  soft-delete for packages that disappear from discovery). FKs on `host_id` and
  `software_item_id` cascade on delete. Per-host version state lives here; for featured items,
  plugin coupling is in the separate `host_software_item_plugins` table; for non-featured items,
  `plugin_config_id` and `package_identifier` are stored directly on this row.
- **`host_software_item_plugins`**: role-based plugin assignments. Each (host, software_item) pair
  can have one plugin per role. Columns: `id` (UUID PK), `host_id`, `software_item_id`,
  `plugin_config_id` (FK → `plugin_configs.id`, ON DELETE RESTRICT), `role` (TEXT —
  `"detect_version"`, `"fetch_releases"`, or `"execute_update"`), `ordinal` (INTEGER, default `0` —
  reserved for future multi-instance roles), `package_identifier`, `config_override?` (JSON),
  `execution_site` (TEXT — `"auto"` | `"agent"` | `"controller"`, default `"auto"`), `created_at`,
  `updated_at`. Composite FK `(host_id, software_item_id)` references `host_software_items` with
  ON DELETE CASCADE.
  - Unique index: `uq_hsip_host_item_role_ordinal ON (host_id, software_item_id, role, ordinal)`
    — one plugin per role per (host, software_item) pair
- **`software_ignores`**: `id` (UUID PK), `tenant_id` (FK → `tenants.id`, ON DELETE CASCADE),
  `host_id?` (UUID FK — nullable; `null` for tenant-wide rules, non-null for host-specific rules),
  `plugin_config_id?` (UUID FK — nullable; scope for host-specific rules),
  `name?` (TEXT — software item name for tenant-wide rules),
  `package_identifier?` (TEXT — package identifier for host-specific rules),
  `created_at`
  - Unique constraint: `(tenant_id, name) WHERE host_id IS NULL` — one tenant-wide rule per name
  - Unique constraint: `(tenant_id, host_id, plugin_config_id, package_identifier) WHERE host_id IS NOT NULL` — one host-specific rule per package

## Relationships

- `SoftwareItem` ↔ `Host` via `HostSoftwareItem` junction (many:many)
- `HostSoftwareItem` has_many `HostSoftwareItemPlugin` (one:many — one plugin per role)
- `HostSoftwareItemPlugin` belongs_to `PluginConfig` (many:1 — multiple role assignments can share one config)
- `PluginConfig` has_many `HostSoftwareItemPlugin`
- `package_identifier` on each role assignment distinguishes packages within a plugin config (e.g. different
  formulae from the same Homebrew config)
- `config_override` on a role assignment extends/overrides the base PluginConfig at resolution time (e.g.
  different `asset_patterns` or `tag_strip_prefix` per host)
- `execution_site` controls where the plugin operation runs: `"auto"` (system decides based on plugin
  capabilities), `"agent"` (always run on the agent), or `"controller"` (only valid for `fetch_releases`)
- Latest version and release metadata are stored per-host on `host_software_items` (not in a separate table),
  populated by the `fetch_releases` role plugin

## `featured` field

The `featured` boolean flag controls how a software item is presented in the UI:

| Value | UI location | Typical plugins |
| ----- | ----------- | --------------- |
| `true` | Main Software list (individual entries) | Docker, Proxmox Helper Scripts, GitHub Releases |
| `false` | Host detail page (aggregated package view) | APT, Homebrew, Mac App Store, npm |

Plugins set the `featured` flag during discovery. Manually created items default to `featured = true`.
All discovered items are tracked immediately with `enabled: true` -- there is no pending state or
approval workflow. See [Autodiscovery](../end-user/autodiscovery.md) for the discovery workflow.

## Response type fields

### `SoftwareItemResponse` (list and mutation responses)

| Field | Type | Description |
| --- | --- | --- |
| `id` | `Uuid` | Item UUID |
| `name` | `String` | Display name |
| `plugins` | `Vec<String>` | Distinct plugin type identifiers across all active host assignments (for display in lists) |
| `enabled` | `bool` | Whether version checks are active |
| `featured` | `bool` | Whether the item appears individually in the Software list (`true`) or as part of aggregated host summaries (`false`) |
| `last_checked_at` | `Option<OffsetDateTime>` | When the last successful version check completed; updated in batch after `VersionCheckResults` is received |
| `host_count` | `u64` | Number of hosts currently assigned |
| `latest_version` | `Option<String>` | Latest known version derived as the maximum across all hosts' per-host `latest_version` values. `null` when no host has a known latest version yet. |
| `update_available` | `bool` | `true` if at least one assigned host has an `installed_version` that differs from its per-host `latest_version` (and both values are known). Uses string equality -- no semver parsing. |
| `created_at` | `OffsetDateTime` | |
| `updated_at` | `OffsetDateTime` | |

### `SoftwareItemDetailResponse` (detail and assignment responses)

Extends `SoftwareItemResponse` with:

| Field | Type | Description |
| --- | --- | --- |
| `hosts` | `Vec<SoftwareItemHostSummary>` | Per-host assignment records |

### `SoftwareItemHostSummary`

| Field | Type | Description |
| --- | --- | --- |
| `host_id` | `Uuid` | |
| `hostname` | `String` | |
| `friendly_name` | `String` | |
| `plugins` | `Vec<HostPluginRoleSummary>` | Role-specific plugin assignments for this host-software pair (see below) |
| `installed_version` | `Option<String>` | Version detected on this host |
| `installed_version_detected_at` | `Option<OffsetDateTime>` | |
| `latest_version` | `Option<String>` | Per-host latest known version from the `fetch_releases` role plugin. `null` when no upstream version has been resolved yet for this host. |
| `latest_release_metadata` | `Option<Value>` | Rich release metadata (notes, date, assets) from the latest fetch |
| `update_available` | `bool` | `true` when `installed_version` and `latest_version` are both `Some` and differ |
| `last_updated_at` | `Option<OffsetDateTime>` | |
| `linked_at` | `OffsetDateTime` | |

### `HostPluginRoleSummary`

| Field | Type | Description |
| --- | --- | --- |
| `role` | `PluginRole` | `detect_version`, `fetch_releases`, or `execute_update` |
| `plugin_config_id` | `Uuid` | Referenced plugin config |
| `plugin_config_name` | `String` | Display name of the plugin config |
| `plugin_type` | `String` | Plugin type identifier (e.g. `"package_manager_homebrew"`, `"releases_github"`) |
| `package_identifier` | `String` | Plugin-specific package identifier |
| `config_override` | `Option<Value>` | Per-role overrides merged onto the base config |
| `execution_site` | `String` | `"auto"`, `"agent"`, or `"controller"` |

> **Note:** `update_available` uses string equality only. Because version format is
> plugin-specific (e.g. Homebrew may return `"1.2.3"`, GitHub Releases may return `"v1.2.3"`), no
> semver normalization is applied. For agent-side `fetch_releases` plugins (Homebrew, APT),
> `latest_version` is populated by the agent during the `VersionCheckResults` handler. For
> controller-side `fetch_releases` plugins (GitHub Releases, Docker Registry), the value comes
> from the controller scheduler and may be `null` until that task has run.

## REST API

| Method | Path | Permission | Status | Description |
| :----- | :---------------------------------------------------- | :------------- | :----- | :------------------------------------------------------------------ |
| POST | `/api/v1/software-items` | ManageSoftware | 201 | Create a new software item (name + enabled only) |
| GET | `/api/v1/software-items` | ViewSoftware | 200 | List active software items; supports `?featured=true\|false` filter |
| GET | `/api/v1/software-items/{id}` | ViewSoftware | 200 | Get software item with assigned hosts + per-host plugin info |
| PUT | `/api/v1/software-items/{id}` | ManageSoftware | 200 | Update name and/or enabled flag |
| DELETE | `/api/v1/software-items/{id}` | ManageSoftware | 204 | Soft-delete the software item |
| POST | `/api/v1/software-items/{id}/hosts` | ManageSoftware | 200 | Assign to additional host(s); each assignment carries a list of role-specific plugin assignments |
| PUT | `/api/v1/software-items/{id}/hosts/{host_id}` | ManageSoftware | 200 | Update a specific role assignment (plugin config, package identifier, config override, or execution site) for a host |
| DELETE | `/api/v1/software-items/{id}/hosts/{host_id}` | ManageSoftware | 204 | Unassign from a host; add `?ignore=true` to also create an ignore rule |
| POST | `/api/v1/software-items/{id}/hosts/{host_id}/update` | ManageSoftware | 200 | Trigger a software update on a specific host; returns `TriggerUpdateResponse` |

## Validation rules

- `name` must not be empty
- `(tenant_id, name)` must be unique among active items
- Each role assignment must reference an active (non-deactivated) plugin config
- `package_identifier` is validated per plugin type (e.g. Homebrew naming rules)
- `config_override`, if provided, is validated by merging with the base config and running plugin-specific validation
- `(host_id, software_item_id, role, ordinal)` must be unique — one plugin per role per (host, software_item) pair
- `execution_site` must be `"auto"`, `"agent"`, or `"controller"`. `"controller"` is only valid for the
  `fetch_releases` role.
- Host IDs in assignment requests must reference active (non-deactivated) hosts
