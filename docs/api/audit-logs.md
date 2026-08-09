# Audit Logs API

The audit log API exposes two read-only list endpoints over semantic audit entries:

- Tenant scope: `GET /api/v1/audit-logs`
- System scope: `GET /api/v1/system-audit-logs`

Both require authentication and an explicit action grant.

## Endpoints

### `GET /api/v1/audit-logs`

Lists tenant-scoped semantic audit entries.

**Required action:** `audit:read`

#### Query parameters

| Parameter        | Type    | Description                                                                               |
| ---------------- | ------- | ----------------------------------------------------------------------------------------- |
| `page`           | integer | Page number (1-based). Defaults to 1.                                                     |
| `per_page`       | integer | Items per page. Defaults to 20. Maximum 1000.                                             |
| `actor_type`     | string  | Filter by actor type: `user`, `api_token`, `oidc`, `service`, `system`.                   |
| `action_type`    | string  | Filter by semantic action (for example `plugin_config.create`).                           |
| `action_kind`    | string  | Filter by action kind: `stateful` or `event`.                                             |
| `outcome`        | string  | Filter by action outcome (`success`, `denied`, `validation_failed`, `failed`, `partial`). |
| `target_type`    | string  | Filter by semantic target type.                                                           |
| `target_id`      | string  | Filter by semantic target identifier.                                                     |
| `from`           | string  | Lower bound timestamp (inclusive), RFC 3339 format.                                       |
| `to`             | string  | Upper bound timestamp (inclusive), RFC 3339 format.                                       |
| `actor_id`       | UUID    | Filter entries by a specific actor UUID.                                                  |
| `correlation_id` | UUID    | Filter entries by correlation ID (exact match).                                           |

#### Response

```json
{
  "items": [
    {
      "id": "019585f4-...",
      "actor_type": "user",
      "actor_id": "01958602-...",
      "actor_display": "admin@example.com",
      "action_type": "plugin_config.update",
      "action_kind": "stateful",
      "target_type": "plugin_config",
      "target_id": "0195ab10-...",
      "target_display": "APT Production",
      "outcome": "success",
      "before_snapshot": {
        "name": "APT Production",
        "enabled": false,
        "api_endpoint": "https://apt.example.com"
      },
      "after_snapshot": {
        "name": "APT Production",
        "enabled": true,
        "api_endpoint": "https://apt.example.com"
      },
      "correlation_id": null,
      "details_json": { "plugin_type": "package-manager.apt" },
      "request_id": "req-01J9...",
      "occurred_at": "2026-03-03T12:00:00Z"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

#### Example

```sh
# List all tenant audit log entries
curl -H "Authorization: Bearer $TOKEN" https://uptrakit.example.com/api/v1/audit-logs

# Filter by actor type and action
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?actor_type=api_token&action_type=plugin_config.delete"

# Filter by outcome and target
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?outcome=denied&target_type=host"

# Filter by time range (inclusive)
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?from=2026-03-01T00:00:00Z&to=2026-03-03T23:59:59Z"

# Fetch stateful rows to inspect before/after snapshots
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?action_kind=stateful&per_page=1"

# Filter by correlation ID
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?correlation_id=01958602-1234-7abc-8def-000000000000"

# Filter to stateful entries only
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?action_kind=stateful"

# Filter to event entries only
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?action_kind=event"
```

---

### `GET /api/v1/system-audit-logs`

Lists system-level semantic audit entries.

**Required action:** `system.audit:read`

#### Query parameters

Identical to [`GET /api/v1/audit-logs`](#get-apiv1audit-logs).

#### Response

Same structure as `GET /api/v1/audit-logs`.

#### Example

```sh
# List system-level audit log entries
curl -H "Authorization: Bearer $TOKEN" https://uptrakit.example.com/api/v1/system-audit-logs

# Filter to show update-freeze events
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/system-audit-logs?action_type=system.service.update_freeze.apply"
```

---

## Response schema

### `AuditLogResponse` / `SystemAuditLogResponse`

| Field             | Type           | Description                                                                              |
| ----------------- | -------------- | ---------------------------------------------------------------------------------------- |
| `id`              | UUID           | UUIDv7 identifier of this audit log entry.                                               |
| `actor_type`      | string         | `"user"`, `"api_token"`, `"oidc"`, `"service"`, or `"system"`.                           |
| `actor_id`        | UUID \| null   | Actor UUID when available.                                                               |
| `actor_display`   | string \| null | Human-readable actor label.                                                              |
| `action_type`     | string         | Canonical semantic action.                                                               |
| `action_kind`     | string         | `"stateful"` or `"event"`. Always present.                                               |
| `target_type`     | string \| null | Semantic target type.                                                                    |
| `target_id`       | string \| null | Semantic target identifier.                                                              |
| `target_display`  | string \| null | Human-readable target label.                                                             |
| `outcome`         | string         | Action outcome (`success`, `denied`, `validation_failed`, `failed`, `partial`).          |
| `before_snapshot` | object \| null | Entity state before the mutation. Present only when `action_kind` is `"stateful"`.       |
| `after_snapshot`  | object \| null | Entity state after the mutation. Present only when `action_kind` is `"stateful"`.        |
| `correlation_id`  | UUID \| null   | Shared identifier for all events in a multi-step workflow. Null for single-step actions. |
| `details_json`    | object \| null | Optional structured details payload.                                                     |
| `request_id`      | string \| null | Optional request correlation identifier.                                                 |
| `occurred_at`     | string         | RFC 3339 timestamp.                                                                      |

### `action_kind` semantics

- `"stateful"` — the action mutated a persisted entity. `before_snapshot` and `after_snapshot` are
  both present and non-null.
- `"event"` — the action is a discrete workflow fact. `before_snapshot` and `after_snapshot` are
  both null.

### System audit log schema difference

`SystemAuditLogResponse` uses the same shape as `AuditLogResponse` minus `tenant_id`. System-scoped
rows have no tenant context by design (they describe controller-level events). The `tenant_id` field
is absent from system audit log responses.

---

## Pagination

Both endpoints use the standard paginated response format:

| Field         | Type    | Description                           |
| ------------- | ------- | ------------------------------------- |
| `items`       | array   | Array of log entries for this page.   |
| `total`       | integer | Total number of matching entries.     |
| `page`        | integer | Current page number.                  |
| `per_page`    | integer | Items per page used in this response. |
| `total_pages` | integer | Total number of pages.                |

---

## Required Actions

| Action              | Granted to       | Endpoint                        |
| ------------------- | ---------------- | ------------------------------- |
| `audit:read`        | `owner`, `admin` | `GET /api/v1/audit-logs`        |
| `system.audit:read` | `owner` only     | `GET /api/v1/system-audit-logs` |

A user without either grant receives `403 Forbidden`.

---

## CLI

```sh
# Tenant audit log
uptrakit audit-logs list
uptrakit audit-logs list --actor-type user --action-type host.update --outcome success
uptrakit audit-logs list --target-type host --target-id 0193c9c5-4b3e-7b11-8ab2-7860a9f2f1ad
uptrakit audit-logs list --from 2026-03-01T00:00:00Z --to 2026-03-03T23:59:59Z

# Filter by correlation ID
uptrakit audit-logs list --correlation-id <uuid>

# Filter by action kind
uptrakit audit-logs list --action-kind stateful
uptrakit audit-logs list --action-kind event

# System audit log (owner only)
uptrakit audit-logs system list
uptrakit audit-logs system list --action-type system.scheduler.audit_log_cleanup
```

---

## See also

- [Audit Logs End-User Guide](../end-user/audit-logs.md)
- [Audit Logs Security](../security/audit-logs.md)
- [Audit Logs Development](../development/audit-logs.md)
- [HTTP Web API Overview](http-web-api.md)
