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
- **Action**: canonical `action_type` and `action_kind` (`stateful` or `event`)
- **Target**: optional target type/ID/display
- **Outcome**: `success`, `denied`, `validation_failed`, `failed`, or `partial`
- **Context**: optional `correlation_id`, request ID, and structured details metadata

## Two log tables

| Log         | Contents              | Who can view                   |
| ----------- | --------------------- | ------------------------------ |
| Tenant Logs | Tenant-scoped actions | Users with `audit:read`        |
| System Logs | Global/system actions | Users with `system.audit:read` |

## Viewing audit logs in the UI

Navigate to **Audit Logs** in the sidebar. The link is visible to users with the
`audit:read` or `system.audit:read` action.

### Tab bar

Users with access to both logs see a tab bar at the top:

- **Tenant Logs**
- **System Logs**

Users with access to only one log see that log directly with no tab bar.

### Filters

Apply filters to narrow results:

| Filter         | Description                                                    |
| -------------- | -------------------------------------------------------------- |
| Actor Type     | `user`, `api_token`, `oidc`, `service`, `system`               |
| Action         | Exact semantic action (for example `plugin_config.create`)     |
| Outcome        | `success`, `denied`, `validation_failed`, `failed`, `partial`  |
| Target Type    | Semantic target category (for example `plugin_config`, `host`) |
| Target ID      | Exact target identifier                                        |
| From           | Lower time bound (inclusive). Use the date-time picker.        |
| To             | Upper time bound (inclusive). Use the date-time picker.        |
| Correlation ID | UUID shared by all events in a multi-step workflow             |
| Action Kind    | `stateful` or `event`                                          |

Click **Apply** to load results with the current filters. Click **Clear** to reset all filters.

### Table columns

| Column      | Description                                |
| ----------- | ------------------------------------------ |
| Occurred At | Event timestamp (local display).           |
| Action      | Semantic action name.                      |
| Target      | Target display or target type/ID fallback. |
| Outcome     | Action result badge.                       |
| Actor       | Actor display or actor type/ID fallback.   |

Results are shown newest first and support pagination.

## State changes

Audit rows for stateful actions (service configuration changes, user updates, plugin config
mutations, and others) include a **State tab** in the detail drawer.

**When it appears:** Only on stateful rows. Event rows (auth events, workflow triggers,
completions) do not have a State tab.

**What it shows:** Side-by-side "Before" and "After" tables derived from the entity's snapshot
at the time of the change. Fields are shown as key-value rows. The diff is computed client-side:

| Highlight         | Meaning                                             |
| ----------------- | --------------------------------------------------- |
| Green background  | Field was added (present in After, not in Before)   |
| Red background    | Field was removed (present in Before, not in After) |
| Yellow background | Field value changed                                 |
| Dimmed            | Field unchanged                                     |

Nested object values are collapsible sub-tables. Primitive values display inline.

**Reading Stateful vs Event rows:** In the audit log list, rows with a State tab have
`action_kind = "stateful"`. Rows without a State tab are event-class entries. The detail drawer
header shows the action kind.

## Correlation ID

Some audit entries are linked by a **correlation ID** — a shared UUID that ties all events in a
multi-step workflow together (for example, a batch update trigger and the individual update
completions it spawned, or an OIDC authorization flow and its callback).

**Copy button:** Open the row detail drawer, then click the copy icon next to the correlation ID
to copy the UUID to your clipboard. The copy button is in the detail drawer, not the list row.

**Filtering by correlation ID:** Paste the UUID into the **Correlation ID** filter field in the
filter bar and click **Apply**. The log list narrows to all rows that share that correlation ID.

Single-step actions leave the correlation ID empty; it only appears on rows that are part of a
multi-step workflow.

## What's not shown

V2 audit logs show state changes and workflow events for the current audit period. Some items are
intentionally excluded:

**By design:**

- Read operations (viewing lists, fetching config, downloading files) are not audited.
- Internal background bookkeeping (keepalive, cache refresh) is not audited.

**Deferred to V3 (not yet available):**

- Workflow timeline view: a grouped view of all events sharing a correlation ID, showing sequence
  and timing.
- Per-entity audit history: "show every audit row that ever touched this plugin config."
- Analytics dashboards and time-series aggregation.

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

# Filter by correlation ID
uptrakit audit-logs list --correlation-id <uuid>

# Filter by action kind
uptrakit audit-logs list --action-kind stateful
uptrakit audit-logs list --action-kind event

# List system audit log entries (requires system.audit:read)
uptrakit audit-logs system list
uptrakit audit-logs system list --action-type system.service.update_freeze.apply
```

Use `--output json` to get machine-readable output:

```sh
uptrakit --output json audit-logs list --per-page 50
```

For stateful entries, `uptrakit audit-logs show <id>` displays a compact diff (changed keys only,
before → after on each line).

## Required Actions

| Action              | Role             | Endpoint   |
| ------------------- | ---------------- | ---------- |
| `audit:read`        | `owner`, `admin` | Tenant log |
| `system.audit:read` | `owner` only     | System log |

A user without either grant will see a "You do not have permission" message and the
Audit Logs nav link will not appear in the sidebar.

## See also

- [Audit Logs API Reference](../api/audit-logs.md)
- [Audit Logs Security](../security/audit-logs.md) — what is and is not logged, retention
