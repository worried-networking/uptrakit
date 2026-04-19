# Audit Logs API

The audit log API exposes two read-only list endpoints over semantic audit entries:

- Tenant scope: `GET /api/v1/audit-logs`
- System scope: `GET /api/v1/system-audit-logs`

Both require authentication and explicit permission.

## Endpoints

### `GET /api/v1/audit-logs`

Lists tenant-scoped semantic audit entries.

**Required permission:** `view_audit_logs`

#### Query parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `page` | integer | Page number (1-based). Defaults to 1. |
| `per_page` | integer | Items per page. Defaults to 20. Maximum 1000. |
| `actor_type` | string | Filter by actor type: `user`, `api_token`, `oidc`, `service`, `system`. |
| `action_type` | string | Filter by semantic action (for example `plugin_config.create`). |
| `outcome` | string | Filter by action outcome (`success`, `denied`, `validation_failed`, `failed`, `partial`). |
| `target_type` | string | Filter by semantic target type. |
| `target_id` | string | Filter by semantic target identifier. |
| `from` | string | Lower bound timestamp (inclusive), RFC 3339 format. |
| `to` | string | Upper bound timestamp (inclusive), RFC 3339 format. |
| `actor_id` | UUID | Filter entries by a specific actor UUID. |

#### Response

```json
{
  "items": [
    {
      "id": "019585f4-...",
      "actor_type": "user",
      "actor_id": "01958602-...",
      "actor_display": "admin@example.com",
      "action_type": "plugin_config.create",
      "target_type": "plugin_config",
      "target_id": "0195ab10-...",
      "target_display": "APT Production",
      "outcome": "success",
      "details_json": {
        "plugin_type": "package_manager_apt"
      },
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
```

---

### `GET /api/v1/system-audit-logs`

Lists system-level semantic audit entries.

**Required permission:** `view_system_audit_logs`

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

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | UUIDv7 identifier of this audit log entry. |
| `actor_type` | string | `"user"`, `"api_token"`, `"oidc"`, `"service"`, or `"system"`. |
| `actor_id` | UUID \| null | Actor UUID when available. |
| `actor_display` | string \| null | Human-readable actor label. |
| `action_type` | string | Canonical semantic action. |
| `target_type` | string \| null | Semantic target type. |
| `target_id` | string \| null | Semantic target identifier. |
| `target_display` | string \| null | Human-readable target label. |
| `outcome` | string | Action outcome (`success`, `denied`, `validation_failed`, `failed`, `partial`). |
| `details_json` | object \| null | Optional structured details payload. |
| `request_id` | string \| null | Optional request correlation identifier. |
| `occurred_at` | string | RFC 3339 timestamp. |

---

## Pagination

Both endpoints use the standard paginated response format:

| Field | Type | Description |
| --- | --- | --- |
| `items` | array | Array of log entries for this page. |
| `total` | integer | Total number of matching entries. |
| `page` | integer | Current page number. |
| `per_page` | integer | Items per page used in this response. |
| `total_pages` | integer | Total number of pages. |

---

## Permissions

| Permission | Granted to | Endpoint |
| --- | --- | --- |
| `view_audit_logs` | `owner`, `admin` | `GET /api/v1/audit-logs` |
| `view_system_audit_logs` | `owner` only | `GET /api/v1/system-audit-logs` |

A user with neither permission receives `403 Forbidden`.

---

## CLI

```sh
# Tenant audit log
uptrakit audit-logs list
uptrakit audit-logs list --actor-type user --action-type host.update --outcome success
uptrakit audit-logs list --target-type host --target-id 0193c9c5-4b3e-7b11-8ab2-7860a9f2f1ad
uptrakit audit-logs list --from 2026-03-01T00:00:00Z --to 2026-03-03T23:59:59Z

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
