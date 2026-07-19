# Batch Actions API

All batch endpoints follow a uniform pattern: `POST /api/v1/{resource}/batch` with a JSON body
containing an `action` string and an `ids` UUID array. Responses use partial-success semantics
-- each item independently succeeds or fails.

## Request Format

```json
{
  "action": "approve",
  "ids": [
    "550e8400-e29b-41d4-a716-446655440000",
    "6ba7b810-9dad-11d1-80b4-00c04fd430c8"
  ]
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
  "succeeded": [{ "id": "550e8400-e29b-41d4-a716-446655440000" }],
  "failed": [
    {
      "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
      "error": "service is not in Pending state"
    }
  ]
}
```

## Endpoints

| Endpoint                                   | Actions                           | Permission                                                                                    |
| ------------------------------------------ | --------------------------------- | --------------------------------------------------------------------------------------------- |
| `POST /api/v1/services/batch`              | `approve`, `reject`, `deactivate` | Per-action (`CanApproveServices`, `CanRejectServices`, `CanRemoveServices`)                   |
| `POST /api/v1/system-services/batch`       | `approve`, `reject`, `deactivate` | Per-action (`CanApproveSystemServices`, `CanRejectSystemServices`, `CanRemoveSystemServices`) |
| `POST /api/v1/software-items/batch`        | `approve`, `delete`               | `CanDeleteSoftware`                                                                           |
| `POST /api/v1/hosts/batch`                 | `deactivate`                      | `CanDeactivateHosts`                                                                          |
| `POST /api/v1/autodiscovery/ignores/batch` | `delete`                          | `CanManageIgnores`                                                                            |
| `POST /api/v1/plugin-configs/batch`        | `delete`                          | `CanManageCommands`                                                                           |
| `POST /api/v1/host-tags/batch`             | `delete`                          | `CanUpdateHosts`                                                                              |

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

| Status                     | Condition                                                                                                                                                                                                                      |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `400 Bad Request`          | Empty `action`, empty `ids`, more than 100 IDs, or unknown action string.                                                                                                                                                      |
| `401 Unauthorized`         | Missing or invalid Bearer token.                                                                                                                                                                                               |
| `403 Forbidden`            | Token lacks the required permission for the endpoint.                                                                                                                                                                          |
| `200 OK` (partial success) | Request is valid but some items could not be processed. Items may fail because they are not found, are in the wrong state for the requested action, or violate a constraint. Each failure includes a per-item `error` message. |

## Shared Surface Batch Actions

Surface providers can expose selection-driven interactions through the same shared surface
contracts used for single-item actions. Keep batch-style UX aligned with the shared renderer
patterns and explicit permission gates below rather than building bespoke selection UI per
provider.

There is no `batch_action: true` flag on interaction descriptors -- it existed only on the
deleted legacy `SurfaceActionDescriptor` type and was dead data on the controller (no consumer
ever mapped it onto the wire contract). Selection-driven, multi-row invocation for
provider-backed interactions is not currently modeled; today's batch selection UX is limited to
the built-in surface tables described below. A future interaction-level batch contract is
tracked separately, not part of the single-source registration model in ADR-0028.

## Frontend Integration

The frontend adds multi-select checkboxes to all list pages backed by the endpoints above
(services, system-services, software, hosts, plugin-configs, software ignores) as well as
provider-backed surface tables. Selection uses `SvelteSet<string>` (required by the
`svelte/prefer-svelte-reactivity` ESLint rule); a shared `BatchActionBar` appears once items
are selected and `BatchResultDialog` shows partial-success results. See
[Batch action components](../development/frontend-components.md#batch-action-components) for
component props and the page integration pattern, and
[End-user batch actions](../end-user/batch-actions.md) for the user-facing documentation.

### Shared surface tables

Surface-backed batch selection is currently expressed only by the built-in surface tables listed
above (services, hosts, etc.) -- `SurfaceTable.svelte` does not add a checkbox column or
`BatchActionBar` on its own. Neither `RegisteredInteraction`/`InteractionDescriptor` (controller-side)
nor `AgentInteraction` (agent-side authoring) carries a batch flag; it was dropped as dead data --
no consumer ever mapped it onto the wire contract (ADR-0028).

## Key Files

| File                                                       | Purpose                                                      |
| ---------------------------------------------------------- | ------------------------------------------------------------ |
| `crates/shared/web-api-types/src/batch_actions.rs`         | `BatchActionRequest`, `BatchActionResponse`, and validation  |
| `crates/ui/web-api/src/routes/services.rs`                 | `batch_services` handler                                     |
| `crates/ui/web-api/src/routes/system_services.rs`          | `batch_system_services` handler                              |
| `crates/ui/web-api/src/routes/software_items/mod.rs`       | `batch_software_items` handler                               |
| `crates/ui/web-api/src/routes/hosts.rs`                    | `batch_hosts` handler                                        |
| `crates/ui/web-api/src/routes/autodiscovery.rs`            | `batch_autodiscovery_ignores` handler                        |
| `crates/ui/web-api/src/routes/plugin_configs.rs`           | `batch_plugin_configs` handler                               |
| `crates/ui/web-api/src/routes/host_tags.rs`                | `batch_host_tags` handler                                    |
| `frontend/src/lib/types.ts`                                | `BatchActionRequest`, `BatchActionResponse` TypeScript types |
| `frontend/src/lib/api.ts`                                  | `batchServices`, `batchHosts`, etc. API client functions     |
| `frontend/src/lib/components/BatchActionBar.svelte`        | Shared batch action toolbar                                  |
| `frontend/src/lib/components/BatchResultDialog.svelte`     | Shared partial-success results dialog                        |
| `frontend/src/lib/components/surfaces/SurfaceTable.svelte` | Shared surface table with batch support                      |
