# Audit Logs

Uptrakit records every authenticated HTTP request in audit logs. This gives operators full
visibility into who did what, when, and from where.

## What the logs contain

Each entry records:

- **When** the request occurred
- **Who** made the request (actor ID and type)
- **How** they authenticated (password, OIDC, API token)
- **What** they did (HTTP method and path)
- **Result** (HTTP status code)
- **How long** the request took (milliseconds)
- **Where** from (client IP address and browser/tool user-agent)

The logs never contain passwords, request bodies, response bodies, or authentication tokens.

## Two log tables

| Log | Contents | Who can view |
| --- | --- | --- |
| Tenant Logs | All regular API operations (hosts, software, services, settings) | `owner`, `admin` |
| System Logs | Global infrastructure operations (global settings, CA rotation, MQTT limits, system services) | `owner` only |

## Viewing audit logs in the UI

Navigate to **Audit Logs** in the sidebar. The link is visible to users with the
`view_audit_logs` or `view_system_audit_logs` permission.

### Tab bar

Users with access to both logs see a tab bar at the top:

- **Tenant Logs** — regular API operations
- **System Logs** — infrastructure-scoped operations (global settings, CA rotation, etc.)

Users with access to only one log see that log directly with no tab bar.

### Filters

Apply filters to narrow results:

| Filter | Description |
| --- | --- |
| Actor Type | Filter by `user`, `api_token`, or `oidc`. |
| HTTP Method | Filter by `GET`, `POST`, `PUT`, `PATCH`, `DELETE`. |
| Status Code | Filter to an exact HTTP status code (e.g. `403` for access-denied events). |
| From | Lower time bound (inclusive). Use the date-time picker. |
| To | Upper time bound (inclusive). Use the date-time picker. |

Click **Apply** to load results with the current filters. Click **Clear** to reset all filters.

### Table columns

| Column | Description |
| --- | --- |
| Occurred At | When the request was processed (local time). |
| Method | HTTP method (`GET`, `POST`, etc.). |
| Path | Full request path. |
| Status | HTTP status code, colour-coded (green = 2xx, yellow = 4xx, red = 5xx). |
| Actor Type | `user`, `api_token`, or `oidc`. |
| Auth | How the actor authenticated. |
| Duration (ms) | How long the request took to process. |
| IP | Client IP address (`—` if not available). |

Results are shown newest first and support pagination.

## Viewing audit logs via the CLI

```sh
# List tenant audit log entries
uptrakit audit-logs list

# Filter by actor type
uptrakit audit-logs list --actor-type api_token

# Filter by HTTP method and status
uptrakit audit-logs list --method DELETE --status 403

# Filter by time range (RFC 3339 format)
uptrakit audit-logs list --from 2026-03-01T00:00:00Z --to 2026-03-03T23:59:59Z

# List system audit log entries (owner only)
uptrakit audit-logs system list
uptrakit audit-logs system list --method PUT
```

Use `--output json` to get machine-readable output:

```sh
uptrakit --output json audit-logs list --per-page 50
```

## Permissions required

| Permission | Role | Endpoint |
| --- | --- | --- |
| `view_audit_logs` | `owner`, `admin` | Tenant log |
| `view_system_audit_logs` | `owner` only | System log |

A user without either permission will see a "You do not have permission" message and the
Audit Logs nav link will not appear in the sidebar.

## See also

- [Audit Logs API Reference](../api/audit-logs.md)
- [Audit Logs Security](../security/audit-logs.md) — what is and is not logged, retention
