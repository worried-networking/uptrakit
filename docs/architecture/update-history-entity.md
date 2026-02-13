# Update history entity

An `UpdateHistory` record tracks a single software update operation for a specific software item on a specific host.
Records are immutable — once created they are not modified or soft-deleted.

## Database table

- **`update_history`**:
  - `id` (UUID PK)
  - `host_id` (FK → `hosts.id`, ON DELETE CASCADE)
  - `software_item_id` (FK → `software_items.id`, ON DELETE CASCADE)
  - `from_version?` (version before update, null if unknown)
  - `to_version` (target version)
  - `status` (string-backed enum: pending, in_progress, completed, failed)
  - `output` (text, NOT NULL — full command output for success or failure)
  - `initiated_by` (string, NOT NULL — user UUID, "scheduler", or "mqtt")
  - `started_at`
  - `completed_at?`
  - `created_at`
  - Indexes:
    - `idx_update_history_host_id`
    - `idx_update_history_software_item_id`
    - `idx_update_history_status`
    - `idx_update_history_host_software_item` (composite)

## Status enum

The `UpdateStatus` enum is defined in two places:

- **Entity level** (`crates/shared/db/src/entity/update_history.rs`): `DeriveActiveEnum` with
  `sea_orm(rs_type = "String")`. Variants: `Pending`, `InProgress`, `Completed`, `Failed`.
- **API level** (`crates/shared/web-api-types/src/update_history.rs`): `serde(rename_all = "snake_case")` with
  `as_str()` / `from_str()` methods. Conversion between DB and API enums happens in the route handler's
  `db_status_to_api` helper.

## Tenant scoping

No direct `tenant_id` column. Tenant scoping is implicit via `host_id` FK — the host table has `tenant_id`. The list
endpoint loads all tenant host IDs and filters with `is_in()`. The get endpoint verifies the record's host belongs to
the requesting tenant.

## Relationships

- `UpdateHistory` belongs_to `Host` (many:1)
- `UpdateHistory` belongs_to `SoftwareItem` (many:1)
- `Host` has_many `UpdateHistory`
- `SoftwareItem` has_many `UpdateHistory`

## REST API

| Method | Path | Permission | Description |
| :----- | :---------------------------- | :----------- | :------------------------------------------------------------- |
| GET | `/api/v1/update-history` | ViewSettings | List records (filterable by host_id, software_item_id, status) |
| GET | `/api/v1/update-history/{id}` | ViewSettings | Get single record |

Responses include denormalized `host_name` and `software_item_name` fields.

## Key files

| File | Purpose |
| :----------------------------------------------------------------- | :--------------------------------------- |
| `crates/shared/db/src/entity/update_history.rs` | SeaORM entity with `UpdateStatus` enum |
| `crates/core/controller/src/migration/m20260209_000001_initial.rs` | DB migration |
| `crates/shared/web-api-types/src/update_history.rs` | API types (response, query, status enum) |
| `crates/ui/web-api/src/routes/update_history.rs` | Route handlers + unit tests |
