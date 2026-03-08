# Audit Log Security

## Overview

The audit log subsystem records all authenticated HTTP requests to provide a compliance-ready
audit trail for security investigations, incident response, and operational visibility. This
document covers what is logged, what is excluded, the tenant isolation model, retention
policies, and GDPR considerations.

## What is logged

Every authenticated HTTP request generates an `AuditEntry` containing:

| Field | Description | Example |
| --- | --- | --- |
| `id` | UUIDv7 identifier | `019…` |
| `tenant_id` | Tenant scope (nullable) | `019…` |
| `actor_id` | User or API token UUID | `019…` |
| `actor_type` | `"user"`, `"api_token"`, `"oidc"` | `"user"` |
| `auth_method` | How the actor authenticated | `"password"` |
| `http_method` | HTTP method | `"POST"` |
| `http_path` | Raw request path | `"/api/v1/hosts/abc"` |
| `route_pattern` | Axum matched route pattern | `"/api/v1/hosts/{id}"` |
| `http_status` | Response status code | `200` |
| `client_ip` | Client IP from `resolve_ip` middleware | `"192.168.1.10"` |
| `user_agent` | `User-Agent` header | `"curl/8.0"` |
| `duration_ms` | Request processing time | `42` |
| `occurred_at` | UTC timestamp | `2026-03-03T12:00:00Z` |

## What is NOT logged

The audit log **never** captures:

- **Request bodies** -- no passwords, secrets, API tokens, or configuration payloads.
- **Response bodies** -- no query results, error details, or sensitive data.
- **Authentication credentials** -- only the authentication method and actor ID are recorded,
  not the JWT token, API token value, or OIDC tokens.
- **Failed authentication attempts** -- the audit middleware runs inside `require_auth`, so
  only successfully authenticated requests are logged. Failed login attempts are handled
  separately by rate limiting and login logging.

This ensures that even if the audit log database is compromised, no secrets or credentials
are exposed.

## Tenant isolation

### Two-table design

Audit entries are stored in two separate tables:

| Table | Scope | `tenant_id` column |
| --- | --- | --- |
| `audit_logs` | Tenant-scoped requests | Required (UUID) |
| `system_audit_logs` | Global/system requests | Not present |

The `audit_logs` table implements `TenantScoped`, ensuring queries through `TenantDb`
automatically filter by the authenticated user's tenant.

`system_audit_logs` has no tenant column and no `TenantScoped` implementation. It stores
entries for system-level operations that are not tied to any tenant.

### No FK constraint on `tenant_id`

The `audit_logs.tenant_id` column intentionally has **no foreign key constraint** referencing
the `tenants` table. This design decision ensures:

- **Compliance**: Audit records survive tenant deletion. Even if a tenant is removed, their
  audit trail remains intact for regulatory and forensic purposes.
- **Immutability**: Audit log entries are write-once, never updated or cascaded.
- **Independence**: Tenant lifecycle operations (deletion, migration) never affect the audit
  trail.

This mirrors the same rationale used for immutable log tables in compliance-oriented systems.

### Routing

The `audit_log` middleware detects system-scoped routes by URL prefix:

- Routes under `/api/v1/global-settings/` (global settings management, CA rotation) →
  `system_audit_logs` (`tenant_id = None`)
- Routes under `/api/v1/system-services/` (system service management) →
  `system_audit_logs` (`tenant_id = None`)
- All other authenticated routes → `audit_logs` (`tenant_id = Some(default_tenant_id)`)

This ensures infrastructure-level operations are always recorded in the global log, regardless
of which tenant is active.

### Access permissions

| Permission | DB name | Grants access to |
| --- | --- | --- |
| `ViewAuditLogs` | `view_audit_logs` | `GET /api/v1/audit-logs` (tenant log) |
| `ViewSystemAuditLogs` | `view_system_audit_logs` | `GET /api/v1/system-audit-logs` (global log) |

Default assignments:

| Role | `view_audit_logs` | `view_system_audit_logs` |
| --- | --- | --- |
| `owner` | ✓ | ✓ |
| `admin` | ✓ | ✗ |
| `user` | ✗ | ✗ |

The `user` role has no access to audit logs by default. Operators with `admin` role can view
what happened in the tenant but cannot access system-level operations. Only `owner` can
view the system audit log, limiting exposure of global infrastructure changes.

## Backend security properties

### Database backend

- Entries are written to the same database as the application by default.
- A separate audit database can be configured via `--audit-log-db-url`, providing physical
  separation of audit data from application data.
- The separate audit database runs the full migration set. Extra empty tables are harmless.
- Database credentials for the audit database follow the same security model as the main
  database connection.

### Journald backend

- Entries are emitted as structured tracing events with target `uptrakit_audit`.
- Journald provides its own integrity guarantees (forward-secure sealing, tamper detection).
- Entries include all `AuditEntry` fields as structured key-value pairs.
- The journald backend is feature-gated (`journald` Cargo feature) and is never pulled in
  unconditionally.

### NoopBackend

- Silently discards all entries. Used when `--audit-log-backend none` is set or in tests.
- No data leaves the process.

### MultiplexBackend

- When multiple backends are selected, `MultiplexBackend` fans out to all backends concurrently.
- Errors from one backend do not affect others -- each backend's failure is logged independently.
- This allows writing to both the database and journald simultaneously for defense-in-depth.

## Filter configuration

The filter determines which requests are logged:

| Mode | Logged requests |
| --- | --- |
| `all` (default) | All authenticated requests |
| `mutations` | Only `POST`, `PUT`, `PATCH`, `DELETE` |
| `none` | No requests (audit logging disabled) |

### Global filter

Set via `--audit-log-filter` CLI flag. Applies to all tenants unless overridden.

### Per-tenant override

The `audit_log.filter` setting key allows per-tenant override of the global filter mode.
Per-tenant overrides take precedence over the global setting. This allows:

- A global `mutations` mode with specific tenants set to `all` for enhanced monitoring.
- A global `all` mode with specific tenants set to `none` for privacy compliance.

## Retention and GDPR

### Automatic cleanup

The `AuditLogCleanupExecutor` scheduled task deletes entries older than the configured retention
period (default: 90 days). Both `audit_logs` and `system_audit_logs` are cleaned in a single
database transaction.

The cleanup task is seeded as a scheduled task row (disabled by default, interval: 86 400 seconds / 24 hours).
Enable it via the scheduler management API or database.

### Data minimisation

The audit log captures the minimum data needed for security investigation:

- No request/response bodies.
- No authentication credentials.
- IP addresses and user agents are stored for security purposes (identifying compromised
  accounts, tracing unauthorized access).

### Right to erasure

Audit log entries are compliance records. Under GDPR Article 17(3)(e), the right to erasure
does not apply when processing is necessary for the establishment, exercise, or defence of
legal claims. Audit logs fall under this exemption when used for security and compliance
purposes.

However, the retention cleanup mechanism ensures data is not kept indefinitely. Adjust the
retention period to match your organisation's data retention policy.

### Per-tenant retention

Future: the `audit_log.retention_days` setting key will allow per-tenant retention periods,
enabling tenants with different regulatory requirements to have different retention windows.

## Dispatcher reliability

The `AuditLogDispatcher` uses an unbounded mpsc channel. Key security properties:

- **Non-blocking**: `dispatch()` never blocks the HTTP request handler. Audit logging cannot
  be used as a denial-of-service vector against the API.
- **Fail-open**: if the dispatcher channel is closed or the backend fails, entries are dropped
  with a `tracing::warn!` log. The API continues to serve requests.
- **No data loss guarantee**: the fire-and-forget pattern prioritises availability over
  guaranteed delivery. For guaranteed audit trail delivery, use both the database and journald
  backends simultaneously.

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/audit-log/src/backend.rs` | Backend trait and implementations |
| `crates/shared/audit-log/src/dispatcher.rs` | Fire-and-forget dispatcher |
| `crates/shared/audit-log/src/entry.rs` | `AuditEntry` -- defines what is logged |
| `crates/shared/audit-log/src/filter.rs` | `FilterMode` + `AuditFilter` |
| `crates/shared/db/src/entity/audit_log.rs` | `audit_logs` entity (no FK on `tenant_id`) |
| `crates/shared/db/src/entity/system_audit_log.rs` | `system_audit_logs` entity |
| `crates/ui/web-api/src/middleware/audit_log.rs` | Middleware (captures but never logs secrets) |
| `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs` | Retention cleanup |

## See also

- [Audit Logs Development](../development/audit-logs.md) — crate structure, backend selection,
  testing, REST API query module
- [Audit Logs API Reference](../api/audit-logs.md) — endpoint reference, filter parameters,
  response schema
- [Audit Logs End-User Guide](../end-user/audit-logs.md) — UI walkthrough, filter usage
- [Secrets and Encryption](secrets-and-encryption.md) — encryption-at-rest, master key handling
- [Auth and Authorization](auth-and-authorization.md) — authentication methods, RBAC
- [Secure Development](secure-development.md) — secure coding expectations
