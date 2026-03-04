# Update history entity

An `UpdateHistory` record tracks a single software update operation for a specific software item on a specific host.
Records are immutable — once created they are not modified or soft-deleted.

## Database tables

### `update_history`

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PK | UUIDv7 |
| `host_id` | UUID FK → `hosts.id` | ON DELETE CASCADE |
| `software_item_id` | UUID FK → `software_items.id` | ON DELETE CASCADE |
| `from_version` | TEXT | Nullable; version before update |
| `to_version` | TEXT | NOT NULL; target version |
| `status` | TEXT | String-backed enum: `pending`, `in_progress`, `completed`, `failed` |
| `output` | TEXT | NOT NULL; full command output |
| `actor_type` | TEXT | NOT NULL; `"user"`, `"mqtt"`, `"scheduler"`, or `"legacy"` |
| `actor_id` | TEXT | NOT NULL; user UUID, MQTT client UUID, or empty string |
| `update_category` | TEXT | Nullable; update category (e.g. `security`, `bugfix`, `feature`, `unknown`) |
| `batch_id` | UUID FK → `update_batches.id` | Nullable; ON DELETE SET NULL |
| `started_at` | TIMESTAMP | |
| `completed_at` | TIMESTAMP | Nullable |
| `created_at` | TIMESTAMP | |

Indexes: `idx_update_history_host_id`, `idx_update_history_software_item_id`,
`idx_update_history_status`, `idx_update_history_host_software_item` (composite),
`idx_uh_batch_id`.

### `update_batches`

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PK | UUIDv7 |
| `tenant_id` | UUID FK → `tenants.id` | NOT NULL; ON DELETE RESTRICT |
| `batch_type` | TEXT | `"host_update"` or `"item_rollout"` |
| `status` | TEXT | `"in_progress"`, `"completed"`, `"partially_completed"` |
| `total_count` | INTEGER | Set at creation time |
| `actor_type` | TEXT | `"user"` or `"mqtt"` |
| `actor_id` | TEXT | User UUID or MQTT client ID |
| `created_at` | TIMESTAMP | |
| `completed_at` | TIMESTAMP | Nullable |

Indexes: `idx_ub_tenant_status` on `(tenant_id, status)`.

Batch status is materialized — updated in `handle_update_result` when the last child
transitions to a terminal state. This avoids expensive subqueries in the list endpoint.

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
- `UpdateHistory` belongs_to `UpdateBatch` (many:1, optional)
- `UpdateBatch` has_many `UpdateHistory`
- `UpdateBatch` belongs_to `Tenant` (many:1)
- `Host` has_many `UpdateHistory`
- `SoftwareItem` has_many `UpdateHistory`

## REST API

| Method | Path | Permission | Description |
| :----- | :----------------------------------------------- | :------------- | :--------------------------------------------------------------- |
| GET | `/api/v1/update-history` | ViewSoftware | List records (filterable by host_id, software_item_id, status) |
| GET | `/api/v1/update-history/{id}` | ViewSoftware | Get single record |
| POST | `/api/v1/hosts/{host_id}/batch-update` | ManageSoftware | Trigger host-wide batch update |
| POST | `/api/v1/software-items/{id}/batch-update` | ManageSoftware | Trigger item-wide batch update |
| GET | `/api/v1/update-batches` | ViewSoftware | List batches (filterable by status) |
| GET | `/api/v1/update-batches/{id}` | ViewSoftware | Get batch with per-item details |
| GET | `/api/v1/update-batches/{id}/stream` | ViewSoftware | SSE stream for batch progress |

Responses include denormalized `host_name` and `software_item_name` fields.

See [Batch Update Endpoints](../api/http-web-api.md#batch-update-endpoints) for request/response details.

## Key files

| File | Purpose |
| :----------------------------------------------------------------- | :----------------------------------------------- |
| `crates/shared/db/src/entity/update_history.rs` | SeaORM entity with `UpdateStatus` enum |
| `crates/shared/db/src/entity/update_batch.rs` | SeaORM entity with `BatchStatus` enum |
| `crates/shared/db/src/migration/m20260209_000001_initial.rs` | DB migration (initial) |
| `crates/shared/db/src/migration/m20260301_000001_update_category.rs` | Migration: update_category column |
| `crates/shared/db/src/migration/m20260301_000002_update_batches.rs` | Migration: update_batches table, batch_id FK |
| `crates/shared/web-api-types/src/update_history.rs` | API types (response, query, status enum) |
| `crates/shared/web-api-types/src/update_batches.rs` | Batch API types (requests, responses) |
| `crates/ui/web-api/src/routes/update_history.rs` | Route handlers + unit tests |
| `crates/ui/web-api/src/routes/update_batches.rs` | Batch route handlers + SSE endpoint |
| `crates/ui/web-api-queries/src/queries/update_batches.rs` | Batch query logic |
| `crates/ui/web-api/src/batch_progress_broadcaster.rs` | In-process SSE broadcast registry |
