# Software item entity

A `SoftwareItem` is a named catalog entry that represents a piece of software to track. It carries no
provider or package information of its own. Each assignment of a software item to a host is recorded in
the `HostSoftwareItem` junction table, which stores the provider config, package identifier, config
override, and per-host state (installed version, detection timestamp).

This model allows a single "1Password" software item to be tracked across hosts that install it via
different providers (e.g. Homebrew on one host, APT via a different provider config on another), without
creating multiple fragmented software item entries.

## Database tables

- **`software_items`**: `id` (UUID PK), `tenant_id`, `name`, `enabled` (default `true`), `discovery_state?`
  (TEXT — `null` for manual items, `'pending'` for discovered-not-yet-reviewed, `'approved'` for reviewed
  discovered items), `last_checked_at?`, `created_at`, `updated_at`, `deactivated_at?`
  - Partial unique index: `uq_software_items_active_name ON (tenant_id, name) WHERE deactivated_at IS NULL`
    — prevents two active items with the same name in a tenant
  - Index: `idx_software_items_deactivated_at`
- **`host_software_items`**: junction table with composite PK `(host_id, software_item_id)`,
  `provider_config_id` (FK → `provider_configs.id`, NOT NULL), `package_identifier` (default `""`),
  `config_override?` (JSON), `installed_version?`, `installed_version_detected_at?`, `last_updated_at?`,
  `linked_at`. FKs on `host_id` and `software_item_id` cascade on delete; `provider_config_id` uses
  ON DELETE RESTRICT.
  - Unique index: `uq_host_software_items_active ON (host_id, provider_config_id, package_identifier)`
    — prevents the same (provider, package) from being tracked twice on one host
  - Index: `idx_host_software_items_provider_config_id`
- **`autodiscovery_ignores`**: `id` (UUID PK), `tenant_id` (FK → `tenants.id`, ON DELETE CASCADE),
  `provider_config_id` (FK → `provider_configs.id`, ON DELETE CASCADE), `package_identifier` (TEXT), `created_at`
  - Unique constraint: `(tenant_id, provider_config_id, package_identifier)` — one rule per package per provider config
- **`available_versions`**: `id` (UUID PK), `software_item_id` (FK → `software_items.id`, ON DELETE CASCADE),
  `version?`, `release_date?`, `release_notes?` (text), `extra?` (JSON — provider-specific metadata such as tag,
  is_prerelease, release_url), `created_at`, `updated_at`
  - CHECK constraint: at least one of `version` or `release_date` must be non-null
  - Index: `idx_available_versions_software_item_id`

## Relationships

- `SoftwareItem` ↔ `Host` via `HostSoftwareItem` junction (many:many)
- `HostSoftwareItem` belongs_to `ProviderConfig` (many:1 — multiple host assignments can share one config)
- `ProviderConfig` has_many `HostSoftwareItem`
- `SoftwareItem` has_many `AvailableVersion` (one:many — upstream release records per item)
- `package_identifier` distinguishes packages within a provider config (e.g. different formulae from the same
  Homebrew config)
- `config_override` on the host assignment extends/overrides the base ProviderConfig at resolution time (e.g.
  different `asset_patterns` or `tag_strip_prefix` per host)

## `discovery_state` field

| Value | `enabled` | Description |
| ----- | --------- | ----------- |
| `null` | any | Manually created item — always included in version checks (subject to `enabled` flag) |
| `"pending"` | `false` | Discovered but not yet reviewed — excluded from version checks and update flows |
| `"approved"` | `true` | Reviewed discovered item — included in version checks |

Pending items are created automatically by the autodiscovery subsystem. See
[docs/api/autodiscovery.md](../api/autodiscovery.md) for the discovery workflow and API endpoints.

## Response type fields

### `SoftwareItemResponse` (list and mutation responses)

| Field | Type | Description |
| --- | --- | --- |
| `id` | `Uuid` | Item UUID |
| `name` | `String` | Display name |
| `provider_types` | `Vec<String>` | Distinct provider types across all host assignments |
| `enabled` | `bool` | Whether version checks are active |
| `discovery_state` | `Option<String>` | `null`, `"pending"`, or `"approved"` |
| `last_checked_at` | `Option<OffsetDateTime>` | When the last successful version check completed; updated in batch after `VersionCheckResults` is received |
| `host_count` | `u64` | Number of hosts currently assigned |
| `latest_version` | `Option<String>` | Latest upstream version from `available_versions`, if known. Populated for agent-side providers (Homebrew, PHS). `null` when not yet resolved. |
| `update_available` | `bool` | `true` if at least one assigned host has a different `installed_version` than `latest_version` (string equality; no semver parsing) |
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
| `provider_config_id` | `Uuid` | |
| `provider_config_name` | `String` | |
| `provider_type` | `String` | |
| `package_identifier` | `String` | |
| `config_override` | `Option<Value>` | |
| `installed_version` | `Option<String>` | Version detected on this host |
| `installed_version_detected_at` | `Option<OffsetDateTime>` | |
| `last_updated_at` | `Option<OffsetDateTime>` | |
| `linked_at` | `OffsetDateTime` | |
| `latest_version` | `Option<String>` | Same as the item-level `latest_version` (denormalized for convenience) |
| `update_available` | `bool` | `true` when `installed_version` and `latest_version` are both `Some` and differ |

> **Note:** `update_available` uses string equality only. Because version format is
> provider-specific (e.g. Homebrew may return `"1.2.3"`, PHS may return `"v1.2.3"`), no
> semver normalization is applied. For agent-side providers (Homebrew, PHS), `latest_version`
> is populated by the agent during the `VersionCheckResults` handler. For controller-side
> providers (GitHub Releases, Docker Registry), the value comes from the scheduled upstream
> resolver and may be `null` until that task has run.

## REST API

| Method | Path | Permission | Status | Description |
| :----- | :---------------------------------------------------- | :------------- | :----- | :------------------------------------------------------------------ |
| POST | `/api/v1/software-items` | ManageSoftware | 201 | Create a new software item (name + enabled only) |
| GET | `/api/v1/software-items` | ViewSoftware | 200 | List active software items; supports `?discovery_state=pending\|approved` filter |
| GET | `/api/v1/software-items/{id}` | ViewSoftware | 200 | Get software item with assigned hosts + per-host provider info |
| PUT | `/api/v1/software-items/{id}` | ManageSoftware | 200 | Update name and/or enabled flag |
| DELETE | `/api/v1/software-items/{id}` | ManageSoftware | 204 | Soft-delete the software item |
| POST | `/api/v1/software-items/{id}/approve` | ManageSoftware | 200 | Approve a pending discovered item (enables version tracking) |
| POST | `/api/v1/software-items/{id}/hosts` | ManageSoftware | 200 | Assign to additional host(s); each assignment carries its own provider config and package identifier |
| PUT | `/api/v1/software-items/{id}/hosts/{host_id}` | ManageSoftware | 200 | Update the provider config, package identifier, or config override for a specific host assignment |
| DELETE | `/api/v1/software-items/{id}/hosts/{host_id}` | ManageSoftware | 204 | Unassign from a host; add `?ignore=true` to also create an ignore rule |
| POST | `/api/v1/software-items/{id}/hosts/{host_id}/update` | ManageSoftware | 200 | Trigger a software update on a specific host; returns `TriggerUpdateResponse` |

## Validation rules

- `name` must not be empty
- `(tenant_id, name)` must be unique among active items
- Each host assignment must reference an active (non-deactivated) provider config
- `package_identifier` is validated per provider type (e.g. Homebrew naming rules)
- `config_override`, if provided, is validated by merging with the base config and running provider-specific validation
- `(host_id, provider_config_id, package_identifier)` must be unique across all host assignments — the same package
  cannot be tracked twice on one host via the same provider
- Host IDs in assignment requests must reference active (non-deactivated) hosts
