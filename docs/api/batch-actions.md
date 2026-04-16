# Batch Actions API

All batch endpoints follow a uniform pattern: `POST /api/v1/{resource}/batch` with a JSON body
containing an `action` string and an `ids` UUID array. Responses use partial-success semantics
-- each item independently succeeds or fails.

## Request Format

```json
{
  "action": "approve",
  "ids": ["550e8400-e29b-41d4-a716-446655440000", "6ba7b810-9dad-11d1-80b4-00c04fd430c8"]
}
```

The request body maps to `BatchActionRequest` defined in
`crates/shared/web-api-types/src/batch_actions.rs`.

Validation rules (enforced by the `Validate` trait):

- `action` must not be empty.
- `ids` must not be empty.
- `ids` must contain at most 100 entries (`MAX_BATCH_SIZE`).

## Response Format

Every batch endpoint returns `200 OK` with a `BatchActionResponse` body, even when some items
fail. Callers must inspect both arrays to determine the outcome of each item.

```json
{
  "succeeded": [
    { "id": "550e8400-e29b-41d4-a716-446655440000" }
  ],
  "failed": [
    { "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8", "error": "service is not in Pending state" }
  ]
}
```

## Endpoints

| Endpoint | Actions | Permission |
| --- | --- | --- |
| `POST /api/v1/services/batch` | `approve`, `reject`, `deactivate` | Per-action (`CanApproveServices`, `CanRejectServices`, `CanRemoveServices`) |
| `POST /api/v1/system-services/batch` | `approve`, `reject`, `deactivate` | Per-action (`CanApproveSystemServices`, `CanRejectSystemServices`, `CanRemoveSystemServices`) |
| `POST /api/v1/software-items/batch` | `approve`, `delete` | `CanDeleteSoftware` |
| `POST /api/v1/hosts/batch` | `deactivate` | `CanDeactivateHosts` |
| `POST /api/v1/autodiscovery/ignores/batch` | `delete` | `CanManageIgnores` |
| `POST /api/v1/plugin-configs/batch` | `delete` | `CanManageCommands` |
| `POST /api/v1/host-tags/batch` | `delete` | `CanUpdateHosts` |

All endpoints require a valid Bearer token. Permission extractors are declared on each route
handler and reflected in the OpenAPI spec via `x-required-permission`.

## Side Effects

### Services (`/api/v1/services/batch`)

- **approve** -- Transitions services from `Pending` to `Approved`. Sends a WebSocket
  notification to connected admin sessions and broadcasts an admin event.
- **reject** -- Transitions services from `Pending` to `Rejected`. Sends a WebSocket
  notification and broadcasts an admin event.
- **deactivate** -- Soft-deletes the service, revokes all associated certificates, and bumps
  the CRL revocation version. Wrapped in a database transaction (all-or-nothing per item).
  Sends a WebSocket notification and broadcasts an admin event.

### System Services (`/api/v1/system-services/batch`)

Same lifecycle semantics as tenant services. Approve, reject, and deactivate follow the same
transactional patterns and trigger the same WebSocket and admin event broadcasts.

### Software Items (`/api/v1/software-items/batch`)

- **approve** -- Sets `featured = true` for each matched software item. Already-featured items
  are treated as idempotent success. Emits a `SoftwareItemUpdated` event.
- **delete** -- Removes software items. Emits a `SoftwareItemUpdated` event.

### Hosts (`/api/v1/hosts/batch`)

- **deactivate** -- Deactivates host records. Emits a `HostDeleted` event.

### Autodiscovery Ignores (`/api/v1/autodiscovery/ignores/batch`)

- **delete** -- Removes ignore rules from the autodiscovery ignore list.

### Plugin Configs (`/api/v1/plugin-configs/batch`)

- **delete** -- Removes plugin configuration entries.

### Host Tags (`/api/v1/host-tags/batch`)

- **delete** -- Soft-deletes tags and hard-deletes all host assignments within a transaction.
  Emits `HostTagDeleted` admin event per succeeded item.

## Error Scenarios

| Status | Condition |
| --- | --- |
| `400 Bad Request` | Empty `action`, empty `ids`, more than 100 IDs, or unknown action string. |
| `401 Unauthorized` | Missing or invalid Bearer token. |
| `403 Forbidden` | Token lacks the required permission for the endpoint. |
| `200 OK` (partial success) | Request is valid but some items could not be processed. Items may fail because they are not found, are in the wrong state for the requested action, or violate a constraint. Each failure includes a per-item `error` message. |

## Shared Surface Batch Actions

Surface actions can be marked as batch-capable by calling `.batch()` on
`SurfaceActionDescriptor` (which sets `batch_action: true` in the serialized definition).
When multiple rows are selected in a DataTable, batch-capable actions appear in the batch
action bar.

The `ids` of all selected rows are passed in the action params. The surface action handler
receives the full list and is responsible for processing each item. See
[Extensions API](extensions.md) for the full surface action model.

## Frontend Integration

The frontend provides a multi-select UI for batch actions on all supported list pages. See
[End-user batch actions](../end-user/batch-actions.md) for the user-facing documentation.

### Shared components

| Component | Purpose |
| --- | --- |
| `BatchActionBar.svelte` | Fixed-position toolbar at the viewport bottom. Shows selected count, action buttons (styled per `destructive` flag), and a deselect-all button. |
| `BatchResultDialog.svelte` | Modal that displays partial-success results with per-item error messages. Only shown when failures occur; pure success uses a toast instead. |

### Page integration pattern

Each page follows the same pattern:

1. A `SvelteSet<string>` tracks selected IDs (using `SvelteSet` for Svelte 5 reactivity).
2. A select-all checkbox in `<thead>` supports checked, indeterminate, and unchecked states.
3. Per-row checkboxes are only visible when the user has the required manage permission.
4. `BatchActionBar` renders when `selectedIds.size > 0`.
5. Destructive actions show a `ConfirmDialog` before executing.
6. On success, a toast is shown and the page reloads. On partial failure, `BatchResultDialog`
   displays the results.

### Shared surface tables

When surface-backed actions declare `batch_action: true`, `SurfaceTable.svelte` automatically
adds a checkbox column and renders `BatchActionBar` with the batch-capable actions. The
action is invoked with an `ids` array in the params.

## Key Files

| File | Purpose |
| --- | --- |
| `crates/shared/web-api-types/src/batch_actions.rs` | `BatchActionRequest`, `BatchActionResponse`, and validation |
| `crates/ui/web-api/src/routes/services.rs` | `batch_services` handler |
| `crates/ui/web-api/src/routes/system_services.rs` | `batch_system_services` handler |
| `crates/ui/web-api/src/routes/software_items/mod.rs` | `batch_software_items` handler |
| `crates/ui/web-api/src/routes/hosts.rs` | `batch_hosts` handler |
| `crates/ui/web-api/src/routes/autodiscovery.rs` | `batch_autodiscovery_ignores` handler |
| `crates/ui/web-api/src/routes/plugin_configs.rs` | `batch_plugin_configs` handler |
| `crates/ui/web-api/src/routes/host_tags.rs` | `batch_host_tags` handler |
| `crates/plugins/infrastructure/core/src/legacy_extension.rs` | Legacy compatibility schema carrying the `batch_action` field |
| `frontend/src/lib/types.ts` | `BatchActionRequest`, `BatchActionResponse` TypeScript types |
| `frontend/src/lib/api.ts` | `batchServices`, `batchHosts`, etc. API client functions |
| `frontend/src/lib/components/BatchActionBar.svelte` | Shared batch action toolbar |
| `frontend/src/lib/components/BatchResultDialog.svelte` | Shared partial-success results dialog |
| `frontend/src/lib/components/surfaces/SurfaceTable.svelte` | Shared surface table with batch support |
