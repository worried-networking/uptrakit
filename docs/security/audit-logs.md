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
- Action: `action_type`.
- Target: optional `target_type`, `target_id`, `target_display`.
- Result: `outcome` (`success`, `denied`, `validation_failed`, `failed`, `partial`).
- Context: optional `details_json`, optional `request_id`, `occurred_at`.

## Data minimization

The audit contract intentionally does not require full HTTP payload capture.

- No request body snapshots.
- No response body snapshots.
- No credential material (tokens, passwords, private keys).
- `details_json` is curated and bounded metadata, not free-form dumps.

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

Invalid or out-of-scope events are dropped.

## Backends and reliability

- `DatabaseBackend`: primary durable store.
- `JournaldBackend` (feature-gated): structured mirror to `target: "uptrakit_audit"`.
- `MultiplexBackend`: concurrent fan-out.
- `AuditLogDispatcher`: unbounded fire-and-forget channel.

Security tradeoff: request/mutation paths are not blocked by backend latency, so dropped writes
on shutdown/backend failure are possible. For stronger operational resilience, run DB and journald
together.

## Retention

`AuditLogCleanupExecutor` deletes old rows from both tables (default policy: 90 days).

## V2 Deferrals

- Per-tenant retention (`audit_log.retention_days`) exists as a setting key but is not yet applied
  by cleanup.
- Global/per-tenant `audit_log.filter` policy remains in config/state but is not yet the
  centralized enforcement point for semantic producers.

## See also

- [Audit Logs Development](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
- [Audit Logs API Reference](https://github.com/worried-networking/uptrakit/tree/main/docs/api/)
- [Audit Logs End-User Guide](../end-user/audit-logs.md)
- [Auth and Authorization](auth-and-authorization.md)
