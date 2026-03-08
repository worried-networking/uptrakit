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
| `POST /api/v1/services/batch` | `approve`, `reject`, `deactivate` | `CanManageAgents` |
| `POST /api/v1/system-services/batch` | `approve`, `reject`, `deactivate` | `CanManageSystemServices` |
| `POST /api/v1/software-items/batch` | `approve`, `delete` | `CanManageSoftware` |
| `POST /api/v1/hosts/batch` | `deactivate` | `CanManageHosts` |
| `POST /api/v1/hosts/{host_id}/packages/batch` | `delete`, `enable`, `disable` | `CanManageSoftware` |
| `POST /api/v1/autodiscovery/ignores/batch` | `delete` | `CanManageSoftware` |
| `POST /api/v1/plugin-configs/batch` | `delete` | `CanManageSoftware` |

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

- **approve** -- Transitions discovered (pending) software items to approved status. Emits a
  `SoftwareItemUpdated` event.
- **delete** -- Removes software items. Emits a `SoftwareItemUpdated` event.

### Hosts (`/api/v1/hosts/batch`)

- **deactivate** -- Deactivates host records. Emits a `HostDeleted` event.

### Host Packages (`/api/v1/hosts/{host_id}/packages/batch`)

- **delete** -- Removes package records from the host.
- **enable** / **disable** -- Toggles the monitoring state of packages on the host.

All three actions emit a `HostPackagesChanged` event.

### Autodiscovery Ignores (`/api/v1/autodiscovery/ignores/batch`)

- **delete** -- Removes ignore rules from the autodiscovery configuration.

### Plugin Configs (`/api/v1/plugin-configs/batch`)

- **delete** -- Removes plugin configuration entries.

## Error Scenarios

| Status | Condition |
| --- | --- |
| `400 Bad Request` | Empty `action`, empty `ids`, more than 100 IDs, or unknown action string. |
| `401 Unauthorized` | Missing or invalid Bearer token. |
| `403 Forbidden` | Token lacks the required permission for the endpoint. |
| `200 OK` (partial success) | Request is valid but some items could not be processed. Items may fail because they are not found, are in the wrong state for the requested action, or violate a constraint. Each failure includes a per-item `error` message. |

## Extension Framework Batch Actions

Extensions can mark actions as batch-capable by calling `.batch()` on `ActionDef` (which sets
`batch_action: true` in the serialized definition). When multiple rows are selected in a
DataTable, batch-capable actions appear in the batch action bar.

The `ids` of all selected rows are passed in the action params. The extension receives the
full list and is responsible for processing each item. See
[Extensions API](extensions.md) for the full extension action model.

## Key Files

| File | Purpose |
| --- | --- |
| `crates/shared/web-api-types/src/batch_actions.rs` | `BatchActionRequest`, `BatchActionResponse`, and validation |
| `crates/ui/web-api/src/routes/services.rs` | `batch_services` handler |
| `crates/ui/web-api/src/routes/system_services.rs` | `batch_system_services` handler |
| `crates/ui/web-api/src/routes/software_items.rs` | `batch_software_items` handler |
| `crates/ui/web-api/src/routes/hosts.rs` | `batch_hosts` handler |
| `crates/ui/web-api/src/routes/host_packages.rs` | `batch_host_packages` handler |
| `crates/ui/web-api/src/routes/autodiscovery.rs` | `batch_autodiscovery_ignores` handler |
| `crates/ui/web-api/src/routes/plugin_configs.rs` | `batch_plugin_configs` handler |
| `crates/shared/extension-framework/src/lib.rs` | `ActionDef` with `batch_action` field |
