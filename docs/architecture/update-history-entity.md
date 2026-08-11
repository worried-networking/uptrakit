# Update history entity

An `UpdateHistory` record tracks a single software update operation for a specific software item on a specific host.
Records are immutable — once created they are not modified or soft-deleted.

## Database tables

### `update_history`

| Column                  | Type                               | Notes                                                                          |
| ----------------------- | ---------------------------------- | ------------------------------------------------------------------------------ |
| `id`                    | UUID PK                            | UUIDv7                                                                         |
| `tenant_id`             | UUID FK → `tenants.id`             | NOT NULL; ON DELETE RESTRICT                                                   |
| `host_id`               | UUID FK → `hosts.id`               | ON DELETE CASCADE                                                              |
| `software_item_id`      | UUID FK → `software_items.id`      | ON DELETE CASCADE                                                              |
| `host_software_item_id` | UUID FK → `host_software_items.id` | Nullable; ON DELETE SET NULL                                                   |
| `from_version`          | TEXT                               | Nullable; version before update                                                |
| `to_version`            | TEXT                               | Nullable; target version (null for batch updates where the target is implicit) |
| `status`                | TEXT                               | String-backed enum: `queued`, `pending`, `in_progress`, `completed`, `failed`  |
| `output`                | TEXT                               | NOT NULL; full command output                                                  |
| `actor_type`            | TEXT                               | NOT NULL; `"user"`, `"mqtt"`, `"scheduler"`, or `"legacy"`                     |
| `actor_id`              | TEXT                               | NOT NULL; user UUID, MQTT client UUID, or empty string                         |
| `update_category`       | TEXT                               | Nullable; update category (e.g. `security`, `bugfix`, `feature`, `unknown`)    |
| `batch_id`              | UUID FK → `update_batches.id`      | Nullable; ON DELETE SET NULL                                                   |
| `started_at`            | TIMESTAMP                          | Nullable                                                                       |
| `completed_at`          | TIMESTAMP                          | Nullable                                                                       |
| `created_at`            | TIMESTAMP                          |                                                                                |

Indexes: `idx_update_history_host_id`, `idx_update_history_software_item_id`,
`idx_update_history_status`, `idx_update_history_host_software_item` (composite),
`idx_uh_batch_id`, `uix_update_history_host_active` (unique partial on `host_id WHERE status IN ('pending','in_progress')`),
`idx_update_history_host_queued` (partial on `(host_id, id) WHERE status = 'queued'` — supports FIFO dispatch query).

### `update_batches`

| Column         | Type                   | Notes                                                   |
| -------------- | ---------------------- | ------------------------------------------------------- |
| `id`           | UUID PK                | UUIDv7                                                  |
| `tenant_id`    | UUID FK → `tenants.id` | NOT NULL; ON DELETE RESTRICT                            |
| `batch_type`   | TEXT                   | `"host_update"` or `"item_rollout"`                     |
| `status`       | TEXT                   | `"in_progress"`, `"completed"`, `"partially_completed"` |
| `total_count`  | INTEGER                | Set at creation time                                    |
| `actor_type`   | TEXT                   | `"user"` or `"mqtt"`                                    |
| `actor_id`     | TEXT                   | User UUID or MQTT client ID                             |
| `output`       | TEXT                   | Nullable; aggregated batch output                       |
| `output_bytes` | INTEGER                | Nullable; byte count for streaming                      |
| `created_at`   | TIMESTAMP              |                                                         |
| `completed_at` | TIMESTAMP              | Nullable                                                |

Indexes: `idx_ub_tenant_status` on `(tenant_id, status)`.

Batch status is materialized — updated in `handle_update_result` when the last child
transitions to a terminal state. This avoids expensive subqueries in the list endpoint.

## Status enum

There is one canonical `UpdateStatus` enum, defined in `crates/shared/types/src/update_status.rs`. It carries
its `sea_orm::DeriveActiveEnum` mapping under the `sea-orm` feature and its `utoipa::ToSchema` under the
`openapi` feature. Both the DB entity (`crates/shared/db/src/entity/update_history.rs`) and
`uptrakit-web-api-types` (`crates/shared/web-api-types/src/update_history.rs`, which re-exports the type) use
this one definition, so DB and API statuses cannot drift apart — there is no DB↔API conversion step.

| Variant           | String             | Meaning                                                                             | Terminal? | Active lock? |
| :---------------- | :----------------- | :---------------------------------------------------------------------------------- | :-------: | :----------: |
| `Queued`          | `queued`           | Waiting for host to become free (batch or single update)                            |    No     |      No      |
| `Pending`         | `pending`          | Dispatched; agent not yet started                                                   |    No     |   **Yes**    |
| `InProgress`      | `in_progress`      | Agent executing the update                                                          |    No     |   **Yes**    |
| `AwaitingRestart` | `awaiting_restart` | Update applied; a system restart is required before it takes effect                 |    No     |   **Yes**    |
| `Completed`       | `completed`        | Update succeeded                                                                    |    Yes    |      No      |
| `Failed`          | `failed`           | Update failed                                                                       |    Yes    |      No      |
| `Interrupted`     | `interrupted`      | Outcome unknown — connection lost or time budget exceeded before reporting a result |    Yes    |      No      |

**Active lock** means the row counts toward the per-host lock (i.e. no further update may be
triggered for that host while such a row exists). The partial unique index
`uix_update_history_host_active` covers three statuses:
`WHERE status IN ('pending', 'in_progress', 'awaiting_restart')`.

## Output lifecycle

### Authoritative `output` column

`update_history.output` is the single authoritative source of truth for the captured output of every
**terminal** record (status `Failed` or `Completed`). Once a record reaches a terminal state, `output`
holds the full, byte-capped text of the update run and `output_bytes` / `output_truncated` are set
accordingly. No further writes to these columns occur after that point.

### In-progress streaming window

While an update is in progress (`InProgress`) the agent streams output lines in real time via the
`update_output_lines` side table rather than writing to `output` directly. During this window `output`
is empty. The read path (`get_update_history`, `list_update_history`) detects this state via an
`output.is_empty()` check and falls back to loading and concatenating the rows from
`update_output_lines`. This fallback is intentionally limited to the in-progress streaming window; any
terminal record is served from `output` directly.

### Pre-dispatch protection failures

When the controller's pre-update protection phase (e.g. Proxmox snapshot or backup) fails before the
update is dispatched to the agent, the orchestrator (`run_protection_and_dispatch`) consolidates the
streamed protection output into the authoritative `output` column via `consolidate_protection_output`
(`crates/ui/web-api-queries/src/queries/update_dispatch.rs`). This call happens **after** the output
forwarder task is joined, guaranteeing that every streamed line has been flushed to `update_output_lines`
before the consolidation read. The result is that a protection-failure record is byte-identical in
structure to any other terminal record — `output` holds the real plugin log, not a generic placeholder.

`fail_before_agent_dispatch` marks the record `Failed` and sets the protection status/summary fields,
but intentionally leaves `output` untouched. It is always followed by `consolidate_protection_output`
at the call sites that have joined the forwarder.

### Agent-disconnected path

When the agent disconnects mid-dispatch (`dispatch_update_to_agent` returns `Ok(false)`), the record
is intentionally left in `InProgress` — it is **not** consolidated and not marked terminal. This
preserves the reconnect-recovery path: when the agent reconnects, `select_best_output` reads all
`update_output_lines` for the record (including any protection-phase lines written before dispatch)
and finalizes the record with the combined output. Calling `consolidate_protection_output` on this
path would wrongly finalize a live record.

## Per-host update queue

At most one active (`Pending` or `InProgress`) update may run on a host at any time.
When a second update is triggered while the host is busy, it is **queued** instead of rejected.
All update types (individual software item updates and batch updates) share the same
`update_history` table and the same per-host queue.

### Single (non-batch) update queueing

When `trigger_update_for_host` is called and the host already has an active update:

1. `has_active_update_for_host` detects the busy state (counts `Pending`/`InProgress` rows).
2. The new record is inserted as `Queued` and the caller receives `initial_status: Queued`.
3. No dispatch is sent to the agent.
4. When the active update completes, `handle_update_result` calls `dispatch_next_queued_update`,
   which invokes `dispatch_next_queued_for_host` to promote the oldest `Queued` row to `Pending`
   using a CAS UPDATE: `WHERE id = ? AND status = 'queued'`.

**Race condition safety:** If two controllers simultaneously observe a free host and both attempt
to insert as `Pending`, the partial unique index `uix_update_history_host_active` will reject one
INSERT. The losing controller detects the constraint violation and re-inserts as `Queued`.

### Batch sequential dispatch

When a batch contains multiple items for the same host:

- The **first item** per host is inserted as `Pending` and dispatched immediately — unless the
  host already had an active update from outside the batch, in which case all items start `Queued`.
- **Subsequent items** on a free host are inserted as `Queued` (excluded from the unique index).
- When an item completes, `dispatch_next_in_batch` calls `dispatch_next_queued_for_host` to
  promote the oldest `Queued` row for that host — whether it belongs to this batch, another
  batch, or is a non-batch update. This ensures global FIFO ordering per host.
- The CAS promotes `Queued → Pending`. If another controller already promoted the row,
  `rows_affected == 0` and dispatch is skipped — preventing double-dispatch.

## Tenant scoping

The `tenant_id` column provides direct tenant scoping. The list endpoint filters by `tenant_id`
directly. The get endpoint verifies the record's tenant matches the requesting tenant.

## Relationships

- `UpdateHistory` belongs_to `Tenant` (many:1)
- `UpdateHistory` belongs_to `Host` (many:1)
- `UpdateHistory` belongs_to `SoftwareItem` (many:1)
- `UpdateHistory` belongs_to `HostSoftwareItem` (many:1, optional)
- `UpdateHistory` belongs_to `UpdateBatch` (many:1, optional)
- `UpdateBatch` has_many `UpdateHistory`
- `UpdateBatch` belongs_to `Tenant` (many:1)
- `Host` has_many `UpdateHistory`
- `SoftwareItem` has_many `UpdateHistory`

## REST API

| Method | Path                                       | Action            | Description                                                    |
| :----- | :----------------------------------------- | :---------------- | :------------------------------------------------------------- |
| GET    | `/api/v1/update-history`                   | `software:read`   | List records (filterable by host_id, software_item_id, status) |
| GET    | `/api/v1/update-history/{id}`              | `software:read`   | Get single record                                              |
| POST   | `/api/v1/hosts/{host_id}/batch-update`     | `updates:trigger` | Trigger host-wide batch update                                 |
| POST   | `/api/v1/software-items/{id}/batch-update` | `updates:trigger` | Trigger item-wide batch update                                 |
| GET    | `/api/v1/update-batches`                   | `software:read`   | List batches (filterable by status)                            |
| GET    | `/api/v1/update-batches/{id}`              | `software:read`   | Get batch with per-item details                                |
| GET    | `/api/v1/update-batches/{id}/stream`       | `software:read`   | SSE stream for batch progress                                  |

Responses include denormalized `host_name` and `software_item_name` fields.

See [Batch Update Endpoints](../api/http-web-api.md#batch-update-endpoints) for request/response details.

## Key files

| File                                                                         | Purpose                                                          |
| :--------------------------------------------------------------------------- | :--------------------------------------------------------------- |
| `crates/shared/db/src/entity/update_history.rs`                              | SeaORM entity with `UpdateStatus` enum                           |
| `crates/shared/db/src/entity/update_batch.rs`                                | SeaORM entity with `BatchStatus` enum                            |
| `crates/shared/db/src/migration/m20260209_000001_initial.rs`                 | DB migration (initial)                                           |
| `crates/shared/db/src/migration/m20260301_000001_update_category.rs`         | Migration: update_category column                                |
| `crates/shared/db/src/migration/m20260301_000002_update_batches.rs`          | Migration: update_batches table, batch_id FK                     |
| `crates/shared/db/src/migration/m20260313_000001_per_host_update_locking.rs` | Migration: partial unique index `uix_update_history_host_active` |
| `crates/shared/web-api-types/src/update_history.rs`                          | API types (response, query, status enum)                         |
| `crates/shared/web-api-types/src/update_batches.rs`                          | Batch API types (requests, responses)                            |
| `crates/ui/web-api/src/routes/update_history.rs`                             | Route handlers + unit tests                                      |
| `crates/ui/web-api/src/routes/update_batches.rs`                             | Batch route handlers + SSE endpoint                              |
| `crates/ui/web-api-queries/src/queries/update_batches.rs`                    | Batch query logic                                                |
| `crates/ui/web-api/src/batch_progress_broadcaster.rs`                        | In-process SSE broadcast registry                                |
