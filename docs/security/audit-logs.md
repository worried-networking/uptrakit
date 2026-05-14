---
title: Audit Log Security
weight: 130
description: Uptrakit uses semantic audit logs with durable, mutation-first records of security-relevant actions and outcomes.
---

# Audit Log Security

## Overview

Uptrakit uses semantic audit logs: durable records of security-relevant actions and outcomes.
The system is mutation-first, with explicit action names such as `plugin_config.update`,
`service_config.store`, or `system.service.update_gate`.

## Logged fields

Each row stores:

- Scope: `tenant_id` (tenant table only) or system scope (system table).
- Actor: `actor_type` (`user`, `api_token`, `oidc`, `service`, `system`), optional `actor_id`,
  optional `actor_display`.
- Action: `action_type`, `action_kind` (`stateful` or `event`).
- Target: optional `target_type`, `target_id`, `target_display`.
- Result: `outcome` (`success`, `denied`, `validation_failed`, `failed`, `partial`).
- Context: optional `details_json`, optional `before_snapshot`, optional `after_snapshot`,
  optional `correlation_id`, optional `request_id`, `occurred_at`.

## Data minimization

The audit contract intentionally does not require full HTTP payload capture.

- No request body snapshots.
- No response body snapshots.
- No credential material (tokens, passwords, private keys).
- `details_json` is curated and bounded metadata, not free-form dumps.
- `before_snapshot` and `after_snapshot` are present only on `action_kind = "stateful"` rows and
  are always filtered through the `AuditView` projection, which excludes secret-bearing types.

## Tenant and system isolation

Two immutable tables separate scopes:

- `audit_logs`: tenant-scoped events.
- `system_audit_logs`: global/system events.

`audit_logs.tenant_id` has no FK by design so records survive tenant deletion for compliance.

Access control:

- `view_audit_logs` -> `GET /api/v1/audit-logs`
- `view_system_audit_logs` -> `GET /api/v1/system-audit-logs`

## Trust boundaries for forwarded runtime events

Services may forward `AuditEventPayload` over the internal wire. The controller does not trust
payloads blindly:

- Re-validates `action_type` and `outcome` against canonical types.
- Enforces scope allowlists (tenant-only, service-bound, system-only actions).
- Validates `tenant_id` consistency against enrolled service identity.
- Validates sizes and JSON parseability before writing.
- Rejects any service-forwarded `AuditEventPayload` whose `action_type` resolves to
  `AuditActionKind::Stateful`. Snapshots are always sourced from the controller's authoritative
  DB read; a service-supplied before/after pair could fabricate a plausible audit record that
  contradicts the underlying DB state.

Invalid or out-of-scope events are dropped.

## Backends and reliability

- `DatabaseBackend`: primary durable store.
- `JournaldBackend` (feature-gated): structured mirror to `target: "uptrakit_audit"`.
- `MultiplexBackend`: concurrent fan-out.
- `AuditLogDispatcher`: unbounded fire-and-forget channel.

Security tradeoff: request/mutation paths are not blocked by backend latency, so dropped writes
on shutdown/backend failure are possible. For stronger operational resilience, run DB and journald
together.

## Evidence-integrity properties (V2)

V2 introduces two distinct reliability tiers:

**Stateful rows** (`action_kind = "stateful"`):

- Committed-or-not-present. The mutation and the audit row are written in the same database
  transaction.
- A mutation whose audit cannot be captured does not happen (the transaction is rolled back).
- The DB row is the audit-of-record.

**Event rows** (`action_kind = "event"`):

- Best-effort. Fire-and-forget through the async dispatcher.
- May be delayed or missing on crash; no rollback coupling with any mutation.

**Journald is a mirror, not canonical.** Both Stateful and Event rows are also flushed to
journald when `--audit-log-backend journald` is active; the journald copy is a supplementary
mirror. The DB row is always the authoritative audit record.

## Snapshot storage sizing

V2 adds `before_snapshot` and `after_snapshot` JSON columns to stateful rows. Each column is
capped at 16 KB.

**Worst-case row size:** ~32 KB (two 16 KB snapshots). V1 rows were ~1–2 KB; event-class rows
remain V1-sized.

**Worked retention example:**

> 100 stateful mutations/day × 16 KB average × 90 days = ~144 MB/tenant for stateful rows

Use the `audit_log.retention_days` setting to dial this down if storage pressure surfaces. Lower
values trade audit history depth for storage. See `docs/development/audit-logs.md` for how to
configure the setting.

## V1→V2 cutover note

The V2 migration **drops V1 audit rows**. No transformation is applied; V1 history is not
retained in V2 tables.

**Deployments with a compliance posture** (SOC 2, ISO 27001, or customer DPA referencing audit
history) should export V1 rows before running the migration and store the dump outside the
database for the retention period required by their compliance obligations.

Optional pre-migration export:

Postgres:

```sh
pg_dump --table audit_logs --table system_audit_logs <db-name> > audit-v1-backup.sql
```

SQLite:

```sh
sqlite3 <path-to-db> ".dump audit_logs" ".dump system_audit_logs" > audit-v1-backup.sql
```

Store the dump in a location with the retention period your compliance posture requires.

## What's excluded from V2 audit

The `audit-catalog.toml` file at `crates/shared/audit-log/audit-catalog.toml` lists every
catalogued site with either an `action` entry or a `skip` entry. Review `skip` entries for the
complete exclusion rationale. The intentionally-excluded categories are:

- GET handlers and read paths (covered by transport access log)
- Heartbeats, ping/pong, telemetry counters
- Connection lifecycle bookkeeping (WS open/close, reconnect, keepalive)
- Internal cache writes and denormalization side effects driven by observed state
- Schema migrations themselves

## Retention

`AuditLogCleanupExecutor` deletes old rows from both tables (default policy: 90 days).
Per-tenant enforcement is active via the `audit_log.retention_days` setting.

## V3 deferred

**V3 deferred:**

- Per-action-kind or per-action retention policy (compliance scope).
- Compliance-driven legal-hold and immutable archive.
- Analytics dashboards, workflow timeline view, per-entity audit history view.

The `audit_log.retention_days` setting applies globally; per-tenant enforcement is active.

## See also

- [Audit Logs Development](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
- [Audit Logs API Reference](https://github.com/worried-networking/uptrakit/tree/main/docs/api/)
- [Audit Logs End-User Guide](../end-user/audit-logs.md)
- [Auth and Authorization](auth-and-authorization.md)
