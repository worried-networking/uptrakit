# Audit log subsystem

Development guide for the audit log subsystem. This document covers the crate structure, backend
selection, filter configuration, separate database setup, retention, and testing.

## Architecture overview

The audit log subsystem captures all authenticated HTTP requests and persists them through
pluggable backends. It follows the same fire-and-forget dispatcher pattern as the
[notification subsystem](notifications.md).

```text
HTTP Request
     │
┌────▼──────────────────┐
│  resolve_ip            │  (outer middleware)
├────────────────────────┤
│  require_auth          │  sets AuthenticatedUser in extensions
├────────────────────────┤
│  audit_log             │  captures method/path/user/IP, calls next, records status/duration
├────────────────────────┤
│  route handler         │
└────────────────────────┘
     │
     ▼ dispatcher.dispatch(AuditEntry)
┌────────────────────────┐
│ AuditLogDispatcher     │  mpsc::UnboundedSender (fire-and-forget)
│ (background loop)      │
└──────┬─────────────────┘
       ▼
┌────────────────────────┐
│ Backend                │  one of:
├────────────────────────┤
│ DatabaseBackend        │  → audit_logs / system_audit_logs tables
│ JournaldBackend        │  → structured tracing events → journald layer (feature-gated)
│ MultiplexBackend       │  → fans out to 1..N backends concurrently
│ NoopBackend            │  → discard (used when no backends selected, or in tests)
└────────────────────────┘
```

Key properties:

- **Zero handler latency**: the middleware dispatches asynchronously via an mpsc channel.
- **Failure isolation**: backend write failures are logged at `warn` level but never propagate
  to request handlers.
- **Multiple backends**: the `--audit-log-backend` CLI flag is repeatable. All selected backends
  receive every entry concurrently via `MultiplexBackend`.

## Crate structure

| Crate | Path | Purpose |
| --- | --- | --- |
| `uptrakit-audit-log` | `crates/shared/audit-log/` | Core types, backends, dispatcher, filter logic |
| `uptrakit-shared-db` | `crates/shared/db/` | SeaORM entities (`audit_log`, `system_audit_log`) |
| `uptrakit-web-api` | `crates/ui/web-api/src/middleware/audit_log.rs` | Axum middleware |
| `uptrakit-scheduler-engine` | `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs` | Retention cleanup executor |
| `uptrakit-controller` | `crates/core/controller/` | CLI flags, backend wiring, AppState integration |

## Feature flags

| Feature | Crate | Default | Description |
| --- | --- | --- | --- |
| `db` | `audit-log` | no | Enables `DatabaseBackend` (requires `sea-orm` + `uptrakit-shared-db`) |
| `journald` | `audit-log` | no | Enables `JournaldBackend` (requires `tracing-journald`) |
| `journald` | `controller` | no | Propagates to `audit-log/journald` + adds `tracing-journald` dependency |

Feature flags are additive and chain through the dependency graph:

```text
controller/Cargo.toml          audit-log/Cargo.toml
  journald  ───────────────>     journald  ──> tracing-journald
                                 db        ──> sea-orm + uptrakit-shared-db
```

The `web-api` crate always depends on `audit-log` with `features = ["db"]`.

## Core types

### `AuditEntry`

Defined in `crates/shared/audit-log/src/entry.rs`:

```rust
pub struct AuditEntry {
    pub id: Uuid,                    // UUIDv7
    pub tenant_id: Option<Uuid>,     // None → system_audit_logs, Some → audit_logs
    pub actor_id: Uuid,
    pub actor_type: AuditActorType,  // User | ApiToken | Oidc
    pub auth_method: String,         // "password" | "oidc" | "api_token"
    pub http_method: String,
    pub http_path: String,
    pub route_pattern: Option<String>, // Axum MatchedPath
    pub http_status: u16,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub duration_ms: u64,
    pub occurred_at: OffsetDateTime,
}
```

### `AuditActorType`

Internal-only typed enum following the `ActorType`/`BatchType` pattern: `Copy`, `as_str()` +
`Display`, not `#[non_exhaustive]`, no `Other(String)`.

### `FilterMode`

Controls which requests are logged:

| Mode | Behaviour |
| --- | --- |
| `All` (default) | Log all authenticated requests |
| `Mutations` | Log only `POST`, `PUT`, `PATCH`, `DELETE` |
| `None` | Disable audit logging entirely |

### `AuditFilter`

Combines a global `FilterMode` (set via CLI) with an optional per-tenant override (loaded from
the `audit_log.filter` setting key). Per-tenant overrides take precedence over the global mode.

### `AuditLogBackend` trait

```rust
#[async_trait]
pub trait AuditLogBackend: Send + Sync {
    async fn write(&self, entry: &AuditEntry) -> Result<(), AuditLogError>;
}
```

Implementations:

- **`NoopBackend`** -- discards all entries (default in tests, used when `--audit-log-backend none`).
- **`DatabaseBackend`** (cfg `db`) -- inserts into `audit_logs` or `system_audit_logs` based on
  `tenant_id`.
- **`JournaldBackend`** (cfg `journald`) -- emits structured tracing events at `info` level with
  target `uptrakit_audit`. Requires a `tracing-journald` subscriber layer.
- **`MultiplexBackend`** -- wraps `Vec<Arc<dyn AuditLogBackend>>`, calls all backends concurrently
  via `futures_util::future::join_all`. Errors from one backend do not affect others.

### `AuditLogDispatcher`

Fire-and-forget dispatcher using `mpsc::UnboundedSender<AuditEntry>`. The background loop reads
entries and calls `backend.write()`. Write failures are logged at `warn` level.

`dispatch()` never blocks and never returns an error. If the channel is closed, the entry is
silently dropped with a `tracing::warn!`.

## CLI configuration

The controller exposes three audit-log-related CLI flags:

| Flag | Type | Default | Description |
| --- | --- | --- | --- |
| `--audit-log-backend` | Repeatable enum | `db` | Backend selection: `db`, `journald` (cfg-gated), `none` |
| `--audit-log-db-url` | Optional string | (none) | Separate database URL for audit logs |
| `--audit-log-filter` | Enum | `all` | Global filter mode: `all`, `mutations`, `none` |

### Multiple backends

Pass the flag multiple times to enable concurrent fan-out:

```sh
uptrakit-controller --audit-log-backend db --audit-log-backend journald
```

All selected backends receive every entry. `none` disables all backends and is mutually
exclusive with other values.

### Separate audit database

When `--audit-log-db-url` is provided, the controller opens a second database connection and
runs the standard migrations on it. The `DatabaseBackend` uses this separate connection for
audit log writes. Extra empty tables in the audit database are harmless.

```sh
uptrakit-controller --audit-log-db-url "postgres://audit:pass@db:5432/audit_logs"
```

### Journald backend

The `journald` CLI variant is only available when the controller is compiled with the `journald`
feature. When enabled, the controller configures a layered tracing subscriber with both `fmt`
and `tracing-journald` layers. The journald layer is filtered to the `uptrakit_audit` target.

Entries can be queried with `journalctl`:

```sh
journalctl UPTRAKIT_AUDIT=1 -o json
```

## Per-tenant settings

Two setting keys control per-tenant audit log behaviour:

| Setting key | DB key | Type | Description |
| --- | --- | --- | --- |
| `AuditLogFilter` | `audit_log.filter` | `all` / `mutations` / `none` | Overrides the global `--audit-log-filter` for this tenant |
| `AuditLogRetentionDays` | `audit_log.retention_days` | Integer | Per-tenant retention period (future use) |

Per-tenant overrides take precedence over the global CLI flag. When not set, the global mode
applies.

## Database schema

Two tables store audit entries:

### `audit_logs` (tenant-scoped)

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID (PK) | UUIDv7, not auto-increment |
| `tenant_id` | UUID | **No FK constraint** (compliance: survives tenant deletion) |
| `actor_id` | UUID | User or API token ID |
| `actor_type` | TEXT | `"user"`, `"api_token"`, `"oidc"` |
| `auth_method` | TEXT | `"password"`, `"oidc"`, `"api_token"` |
| `http_method` | TEXT | `"GET"`, `"POST"`, etc. |
| `http_path` | TEXT | Raw request path |
| `route_pattern` | TEXT (nullable) | Axum `MatchedPath` |
| `http_status` | INTEGER | HTTP response status code |
| `client_ip` | TEXT (nullable) | From `resolve_ip` middleware |
| `user_agent` | TEXT (nullable) | `User-Agent` header |
| `duration_ms` | BIGINT | Request duration in milliseconds |
| `occurred_at` | TIMESTAMP | When the request occurred |

Indexes: `(tenant_id, occurred_at)`, `(actor_id)`.

### `system_audit_logs` (global)

Same columns as `audit_logs` minus `tenant_id`. Used for system-level (non-tenant-scoped)
requests.

Indexes: `(occurred_at)`, `(actor_id)`.

### Routing

`DatabaseBackend` routes entries based on `AuditEntry.tenant_id`:

- `Some(tenant_id)` → `audit_logs`
- `None` → `system_audit_logs`

Currently, all authenticated API routes dispatch to `audit_logs` with
`tenant_id = Some(default_tenant_id)`. System audit routing will be refined when multi-tenancy
boundaries are clearer.

## Middleware

The `audit_log` middleware (`crates/ui/web-api/src/middleware/audit_log.rs`) runs **inside**
`require_auth` (declared as inner `route_layer`, meaning it executes after auth):

```rust
let auth_routes = auth_routes
    .route_layer(axum_mw::from_fn_with_state(
        Arc::clone(&state),
        crate::middleware::audit_log::audit_log,
    ))
    .route_layer(axum_mw::from_fn_with_state(
        Arc::clone(&state),
        crate::middleware::require_auth::require_auth,
    ));
```

The middleware:

1. Captures `MatchedPath`, `AuthenticatedUser`, `ClientIp`, `User-Agent` before the handler.
2. Calls `next.run(req).await`.
3. Maps `AuthMethod` to `AuditActorType` and auth method string.
4. Checks `state.audit_log_filter.should_log(method, per_tenant_override)`.
5. Dispatches `AuditEntry` via `state.audit_log_dispatcher`.

## Retention cleanup

The `AuditLogCleanupExecutor` (`crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs`)
runs as a scheduled task (cron `0 3 * * *`, disabled by default). It deletes entries older than
the retention period (default: 90 days) from both `audit_logs` and `system_audit_logs` in a
single database transaction.

The scheduled task type is `AuditLogCleanup` (variant of `ScheduledTaskType`).

Future: per-tenant retention overrides will be read from the `audit_log.retention_days` setting
key.

## Testing

### Default `NoopBackend` in tests

`AppState` uses `unwrap_or_else` defaults for audit log fields:

- `audit_log_filter`: defaults to `AuditFilter::default()` (`FilterMode::All`)
- `audit_log_dispatcher`: defaults to `AuditLogDispatcher::new(Arc::new(NoopBackend))`

This means existing tests require **zero changes** to their `AppState` construction. The
`NoopBackend` silently discards all entries.

### Testing audit log capture

To verify audit logging in tests:

1. Use a `DatabaseBackend` with an in-memory SQLite database.
2. Make authenticated requests via the test router.
3. Query `audit_log::Entity` to verify entries were written.

### CLI tests

Five CLI argument tests are included in `crates/core/controller/src/cli.rs`:

- Default backend is `db`
- `--audit-log-backend none` disables logging
- `--audit-log-filter mutations` sets mutation-only mode
- `--audit-log-db-url` accepts a separate database URL
- Multiple `--audit-log-backend` values are accepted

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/audit-log/src/lib.rs` | Crate root + re-exports |
| `crates/shared/audit-log/src/entry.rs` | `AuditEntry` + `AuditActorType` |
| `crates/shared/audit-log/src/error.rs` | `AuditLogError` + `Result<T>` |
| `crates/shared/audit-log/src/backend.rs` | `AuditLogBackend` trait + `NoopBackend` + `MultiplexBackend` + `DatabaseBackend` + `JournaldBackend` |
| `crates/shared/audit-log/src/filter.rs` | `FilterMode` + `AuditFilter` + tests |
| `crates/shared/audit-log/src/dispatcher.rs` | `AuditLogDispatcher` (fire-and-forget background loop) |
| `crates/shared/db/src/entity/audit_log.rs` | SeaORM entity for `audit_logs` table |
| `crates/shared/db/src/entity/system_audit_log.rs` | SeaORM entity for `system_audit_logs` table |
| `crates/ui/web-api/src/middleware/audit_log.rs` | Axum middleware |
| `crates/ui/web-api/src/setting_key.rs` | `AuditLogFilter` + `AuditLogRetentionDays` setting keys |
| `crates/ui/web-api/src/app_state.rs` | `AppState` fields: `audit_log_filter`, `audit_log_dispatcher` |
| `crates/core/controller/src/cli.rs` | `AuditLogBackendArg`, `AuditLogFilterArg` enums + CLI flags |
| `crates/core/controller/src/main.rs` | Backend construction + AppState wiring |
| `crates/core/controller/src/startup.rs` | `init_audit_database()` for separate DB |
| `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs` | Retention cleanup executor |

## Cross-references

- [Audit Logs Security](../security/audit-logs.md)
- [Notifications Development](notifications.md) (similar dispatcher pattern)
- [Scheduler Engine](scheduler-engine.md) (cleanup executor registration)
- [Coding Standards](coding-standards.md)
- [Error Handling](error-handling.md)
- [Testing](testing.md)
