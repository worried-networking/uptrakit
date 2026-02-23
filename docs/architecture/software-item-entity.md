# Software item entity

A `SoftwareItem` defines what to track: a named piece of software linked to a `ProviderConfig`. Each item can be
assigned to multiple hosts via the `HostSoftwareItem` junction table, which stores per-host state (installed version,
detection timestamp).

## Database tables

- **`software_items`**: `id` (UUID PK), `name`, `provider_config_id` (FK → `provider_configs.id`, ON DELETE RESTRICT),
  `package_identifier` (default `""`), `config_override?` (JSON), `enabled` (default `true`), `discovery_state?`
  (TEXT — `null` for manual items, `'pending'` for discovered-not-yet-reviewed, `'approved'` for reviewed
  discovered items), `last_checked_at?`, `created_at`, `updated_at`, `deactivated_at?`
  - Partial unique index: `uq_software_items_active ON (tenant_id, provider_config_id, package_identifier) WHERE deactivated_at IS NULL` — prevents duplicate tracking of the same active package; deleted items can be re-discovered
  - Indexes: `idx_software_items_provider_config_id`, `idx_software_items_deactivated_at`
- **`autodiscovery_ignores`**: `id` (UUID PK), `tenant_id` (FK → `tenants.id`, ON DELETE CASCADE),
  `provider_config_id` (FK → `provider_configs.id`, ON DELETE CASCADE), `package_identifier` (TEXT), `created_at`
  - Unique constraint: `(tenant_id, provider_config_id, package_identifier)` — one rule per package per provider config
- **`host_software_items`**: junction table with composite PK `(host_id, software_item_id)`, `installed_version?`,
  `installed_version_detected_at?`, `last_updated_at?`, `linked_at`. FKs cascade on delete.
- **`available_versions`**: `id` (UUID PK), `software_item_id` (FK → `software_items.id`, ON DELETE CASCADE),
  `version?`, `release_date?`, `release_notes?` (text), `extra?` (JSON — provider-specific metadata such as tag,
  is_prerelease, release_url), `created_at`, `updated_at`
  - CHECK constraint: at least one of `version` or `release_date` must be non-null
  - Index: `idx_available_versions_software_item_id`

## Relationships

- `SoftwareItem` belongs_to `ProviderConfig` (many:1 — multiple items can share one config)
- `ProviderConfig` has_many `SoftwareItem`
- `SoftwareItem` has_many `AvailableVersion` (one:many — upstream release records per item)
- `SoftwareItem` ↔ `Host` via `HostSoftwareItem` junction (many:many)
- `package_identifier` distinguishes items within a shared config (e.g. different assets from the same GitHub repo)
- `config_override` extends/overrides the base ProviderConfig at resolution time (e.g. different `asset_patterns` or
  `tag_strip_prefix`)

## `discovery_state` field

| Value | `enabled` | Description |
| ----- | --------- | ----------- |
| `null` | any | Manually created item — always included in version checks (subject to `enabled` flag) |
| `"pending"` | `false` | Discovered but not yet reviewed — excluded from version checks and update flows |
| `"approved"` | `true` | Reviewed discovered item — included in version checks |

Pending items are created automatically by the autodiscovery subsystem. See
[docs/api/autodiscovery.md](../api/autodiscovery.md) for the discovery workflow and API endpoints.

## REST API

| Method | Path | Permission | Status | Description |
| :----- | :---------------------------------------------------- | :------------- | :----- | :------------------------------------------------------------------ |
| POST | `/api/v1/software-items` | ManageSoftware | 201 | Create a new software item |
| GET | `/api/v1/software-items` | ViewSoftware | 200 | List all active software items (with host count) |
| GET | `/api/v1/software-items/{id}` | ViewSoftware | 200 | Get software item with assigned hosts + installed versions |
| PUT | `/api/v1/software-items/{id}` | ManageSoftware | 200 | Update name, enabled, package_identifier, config_override |
| DELETE | `/api/v1/software-items/{id}` | ManageSoftware | 204 | Soft-delete; add `?ignore=true` to also create an ignore rule |
| POST | `/api/v1/software-items/{id}/approve` | ManageSoftware | 200 | Approve a pending discovered item (enables version tracking) |
| POST | `/api/v1/software-items/{id}/hosts` | ManageSoftware | 200 | Assign to additional host(s) |
| DELETE | `/api/v1/software-items/{id}/hosts/{host_id}` | ManageSoftware | 204 | Unassign from a host |

## Validation rules

- `name` must not be empty
- `provider_config_id` must reference an active (non-deactivated) provider config
- `(provider_config_id, package_identifier)` must be unique among active items
- `config_override`, if provided, is validated by merging with the base config and running provider-specific validation
- Host IDs in assignment requests must reference active (non-deactivated) hosts
- `provider_config_id` cannot be changed after creation
