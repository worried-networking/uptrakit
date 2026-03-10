# Audit Logs API

The audit log API exposes two read-only list endpoints: one for tenant-scoped log entries and
one for system-level (global infrastructure) log entries. Both endpoints require authentication
and a specific permission.

## Endpoints

### `GET /api/v1/audit-logs`

Lists tenant-scoped audit log entries. Records HTTP requests made by authenticated users within
the tenant (host management, software updates, service operations, settings changes, etc.).

**Required permission:** `view_audit_logs` (included in the `settings_manager` role)

#### Query parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `page` | integer | Page number (1-based). Defaults to 1. |
| `per_page` | integer | Items per page. Defaults to 25. Maximum 200. |
| `actor_type` | string | Filter by actor type: `user`, `api_token`, or `oidc`. |
| `method` | string | Filter by HTTP method: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`. |
| `status` | integer | Filter by exact HTTP status code (e.g. `200`, `403`, `500`). |
| `from` | string | Lower bound timestamp (inclusive), RFC 3339 format. |
| `to` | string | Upper bound timestamp (inclusive), RFC 3339 format. |
| `actor_id` | UUID | Filter entries by a specific actor UUID. |

#### Response

```json
{
  "items": [
    {
      "id": "019585f4-...",
      "actor_id": "01958602-...",
      "actor_type": "user",
      "auth_method": "password",
      "http_method": "POST",
      "http_path": "/api/v1/hosts/abc123/discover",
      "route_pattern": "/api/v1/hosts/{id}/discover",
      "http_status": 200,
      "client_ip": "192.168.1.10",
      "user_agent": "uptrakit-cli/0.1.0",
      "duration_ms": 42,
      "occurred_at": "2026-03-03T12:00:00Z"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 25,
  "total_pages": 1
}
```

#### Example

```sh
# List all tenant audit log entries
curl -H "Authorization: Bearer $TOKEN" https://uptrakit.example.com/api/v1/audit-logs

# Filter by actor type and method
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?actor_type=api_token&method=DELETE"

# Filter by time range
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/audit-logs?from=2026-03-01T00:00:00Z&to=2026-03-03T23:59:59Z"
```

---

### `GET /api/v1/system-audit-logs`

Lists system-level audit log entries. Records HTTP requests to infrastructure management
endpoints (global settings changes, CA rotation, MQTT limit updates, system service management).

**Required permission:** `view_system_audit_logs` (included in the `system_administrator` role)

#### Query parameters

Identical to [`GET /api/v1/audit-logs`](#get-apiv1audit-logs).

#### Response

Same structure as `GET /api/v1/audit-logs`. The `http_path` values will be under
`/api/v1/global-settings/` or `/api/v1/system-services/`.

#### Example

```sh
# List system-level audit log entries
curl -H "Authorization: Bearer $TOKEN" https://uptrakit.example.com/api/v1/system-audit-logs

# Filter to see only failed requests
curl -H "Authorization: Bearer $TOKEN" \
  "https://uptrakit.example.com/api/v1/system-audit-logs?status=403"
```

---

## Response schema

### `AuditLogResponse` / `SystemAuditLogResponse`

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | UUIDv7 identifier of this audit log entry. |
| `actor_id` | UUID | UUID of the actor who made the request. |
| `actor_type` | string | `"user"`, `"api_token"`, or `"oidc"`. |
| `auth_method` | string | `"password"`, `"oidc"`, or `"api_token"`. |
| `http_method` | string | HTTP method (`"GET"`, `"POST"`, etc.). |
| `http_path` | string | Full request path. |
| `route_pattern` | string \| null | Matched router pattern (e.g. `/api/v1/hosts/{id}`). |
| `http_status` | integer | HTTP response status code. |
| `client_ip` | string \| null | Client IP address. |
| `user_agent` | string \| null | `User-Agent` header value. |
| `duration_ms` | integer | Request duration in milliseconds. |
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
uptrakit audit-logs list --actor-type user --method POST --status 200
uptrakit audit-logs list --from 2026-03-01T00:00:00Z --to 2026-03-03T23:59:59Z

# System audit log (owner only)
uptrakit audit-logs system list
uptrakit audit-logs system list --method PUT
```

---

## See also

- [Audit Logs End-User Guide](../end-user/audit-logs.md)
- [Audit Logs Security](../security/audit-logs.md)
- [Audit Logs Development](../development/audit-logs.md)
- [HTTP Web API Overview](http-web-api.md)
