---
title: Audit Logs
weight: 130
description: Uptrakit audit logs record semantic actions and outcomes rather than raw HTTP request lines, providing a structured history of who did what and whether it succeeded.
---

# Audit Logs

Uptrakit audit logs show semantic actions and outcomes (for example
`plugin_config.update`, `host.deactivate`, `service_config.store`), not raw HTTP request lines.

## What the logs contain

Each entry records:

- **When**: timestamp
- **Who**: actor type (`user`, `api_token`, `oidc`, `service`, `system`) and optional actor ID/display
- **Action**: canonical `action_type`
- **Target**: optional target type/ID/display
- **Outcome**: `success`, `denied`, `validation_failed`, `failed`, or `partial`
- **Context**: optional request ID and structured details metadata

## Two log tables

| Log | Contents | Who can view |
| --- | --- | --- |
| Tenant Logs | Tenant-scoped actions | Users with `view_audit_logs` |
| System Logs | Global/system actions | Users with `view_system_audit_logs` |

## Viewing audit logs in the UI

Navigate to **Audit Logs** in the sidebar. The link is visible to users with the
`view_audit_logs` or `view_system_audit_logs` permission.

### Tab bar

Users with access to both logs see a tab bar at the top:

- **Tenant Logs**
- **System Logs**

Users with access to only one log see that log directly with no tab bar.

### Filters

Apply filters to narrow results:

| Filter | Description |
| --- | --- |
| Actor Type | `user`, `api_token`, `oidc`, `service`, `system` |
| Action | Exact semantic action (for example `plugin_config.create`) |
| Outcome | `success`, `denied`, `validation_failed`, `failed`, `partial` |
| Target Type | Semantic target category (for example `plugin_config`, `host`) |
| Target ID | Exact target identifier |
| From | Lower time bound (inclusive). Use the date-time picker. |
| To | Upper time bound (inclusive). Use the date-time picker. |

Click **Apply** to load results with the current filters. Click **Clear** to reset all filters.

### Table columns

| Column | Description |
| --- | --- |
| Occurred At | Event timestamp (local display). |
| Action | Semantic action name. |
| Target | Target display or target type/ID fallback. |
| Outcome | Action result badge. |
| Actor | Actor display or actor type/ID fallback. |

Results are shown newest first and support pagination.

## Viewing audit logs via the CLI

```sh
# List tenant audit log entries
uptrakit audit-logs list

# Filter by actor type
uptrakit audit-logs list --actor-type api_token

# Filter by action and outcome
uptrakit audit-logs list --action-type plugin_config.update --outcome success

# Filter by target
uptrakit audit-logs list --target-type host --target-id 0193c9c5-4b3e-7b11-8ab2-7860a9f2f1ad

# Filter by time range (RFC 3339 format)
uptrakit audit-logs list --from 2026-03-01T00:00:00Z --to 2026-03-03T23:59:59Z

# List system audit log entries (requires view_system_audit_logs)
uptrakit audit-logs system list
uptrakit audit-logs system list --action-type system.service.update_freeze.apply
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

- [Audit Logs API Reference](https://github.com/worried-networking/uptrakit/tree/main/docs/api/)
- [Audit Logs Security](../security/audit-logs.md) — what is and is not logged, retention
