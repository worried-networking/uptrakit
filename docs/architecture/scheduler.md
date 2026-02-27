# Scheduler

The centralised DB-backed scheduler coordinates periodic background tasks across controller instances. It ensures exactly-once execution in HA
deployments via optimistic locking on the `scheduled_tasks` table.

## Overview

Before the scheduler, periodic tasks ran as independent `tokio::time::interval` loops inside each controller. In a multi-controller HA deployment this
caused duplicate work (every controller ran every task) and provided no coordination for tasks that should execute once globally. The scheduler fixes
this by moving periodic work into the database and claiming tasks with optimistic locking.

Key properties:

- **HA-safe**: only one controller executes a given task at a time (optimistic lock via `locked_by`/`locked_at` columns).
- **Stale recovery**: tasks locked longer than 10 minutes are automatically released.
- **Cron-based**: standard 5-field cron expressions control scheduling; 6/7-field extended expressions are also accepted.
- **REST-manageable**: administrators can view, update schedules, and trigger immediate execution via the REST API.

## Database Schema

### `scheduled_tasks` table

| Column | Type | Description | | --- | --- | --- | | `id` | UUID (PK, v7) | Task identifier | | `tenant_id` | UUID FK | References `tenants.id` | |
`task_type` | TEXT | Enum discriminant (see below) | | `cron_expression` | TEXT | Standard 5-field cron expression | | `enabled` | BOOLEAN | Whether
the task is active | | `task_config` | JSON (nullable) | Per-task configuration | | `last_run_at` | TIMESTAMP (nullable) | Last successful execution |
| `next_run_at` | TIMESTAMP | Next scheduled execution | | `locked_by` | UUID (nullable) | Controller ID holding the claim | | `locked_at` | TIMESTAMP
(nullable) | When the claim was acquired | | `last_error` | TEXT (nullable) | Error message from last failed run | | `run_count` | BIGINT | Total
successful executions | | `created_at` | TIMESTAMP | Row creation time | | `updated_at` | TIMESTAMP | Last modification time |

### Indexes

| Name | Columns | Purpose | | --- | --- | --- | | `idx_scheduled_tasks_next_run` | `next_run_at` | Efficient due-task lookup | |
`idx_scheduled_tasks_tenant_id` | `tenant_id` | Tenant-scoped queries | | `uq_scheduled_tasks_tenant_task_type` | `tenant_id, task_type` (UNIQUE) |
One task per type per tenant |

### Task types

| Value | Default cron | Description | | --- | --- | --- | | `auth_cleanup` | `*/5 * * * *` | Clean expired auth flow state from DB | |
`stale_lease_cleanup` | `*/5 * * * *` | Release stale MQTT client leases | | `event_cleanup` | `0 * * * *` | Legacy (no executor registered; kept for DB compat) |
| `ca_rotation_check` | `0 3 * * *` | Check if managed CA needs rotation | | `version_check` | `0 */6 * * *` | Trigger version detection on agents | |
`service_cert_check` | `0 */12 * * *` | Proactive certificate renewal for services |

All six rows are seeded during the migration with `next_run_at = now`. The `event_cleanup` task type exists in
the database for backward compatibility but has no registered executor — the `controller_events` table has been
dropped in favour of [NATS JetStream](../development/cross-controller-comm.md).

## HA Claim Mechanism

The claim pattern mirrors `MqttLeaseCoordinator` (see [Cross-Controller Communication](../development/cross-controller-comm.md)):

1. **Poll**: every 15 seconds, each controller polls for due tasks (`next_run_at <= now`, `enabled = true`, `locked_by IS NULL`).
1. **Claim**: `UPDATE SET locked_by = $me, locked_at = $now WHERE id = $id AND locked_by IS NULL` -- succeeds only if `rows_affected == 1`.
1. **Execute**: run the task's executor.
1. **Release**: clear `locked_by`/`locked_at`, update `last_run_at`, `next_run_at` (computed from cron), `run_count`, and optionally `last_error`.
1. **Stale recovery**: on each poll cycle, tasks with `locked_at < now - 10 min` are released (the controller may have crashed).
1. **Shutdown**: all claims held by the stopping controller are released.

Manual trigger via the REST API sets `next_run_at = now`, making the task immediately eligible on the next poll cycle.

## Module Structure

The scheduler lives in the controller crate:

```text
crates/core/controller/src/scheduler/
    mod.rs              -- Scheduler struct, SchedulerConfig, poll loop
    cron_utils.rs       -- Cron parsing (chrono<->time bridge), next_run_after()
    claim.rs            -- try_claim, release_claim, recover_stale, release_all, find_due_tasks
    executor.rs         -- TaskExecutor trait
    executors/
        mod.rs
        auth_cleanup.rs
        stale_lease_cleanup.rs
        ca_rotation_check.rs
        version_check.rs
        service_cert_check.rs
```

### TaskExecutor trait

```rust
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(&self, task: &scheduled_task::Model) -> Result<(), String>;
}
```

Each executor implements this trait with the task-specific logic.

### Cron handling

The `cron` crate requires 6 or 7 fields (with a seconds field). Standard 5-field expressions are normalized by prepending `0` (fire at second 0). The
chrono-to-time boundary is bridged via unix timestamps in `cron_utils::next_run_after()`.

## Executor Details

### AuthCleanupExecutor

Calls `cleanup_expired()` on all DB-backed auth flow stores: `OidcFlowStore`, `AccountLinkStore`, `OidcTokenExchangeStore`, `OidcRegistrationStore`,
`DeviceFlowStore`, and `RateLimitStore`. The in-memory `TokenDenylist` purge remains a per-controller interval (not scheduled here).

### StaleLeaseCleanupExecutor

Creates a `MqttLeaseCoordinator` and calls `cleanup_stale_leases()` to release MQTT client leases that have been held without heartbeat for longer
than the stale threshold.

### CaRotationCheckExecutor

Checks `pki::should_rotate_ca()` against the current CA snapshot. If rotation is needed, fires the existing `ca_rotation_trigger` (`Arc<Notify>`). The
per-controller CA rotation loop still handles the actual key generation and certificate signing.

### VersionCheckExecutor

Queries all enabled software items for the tenant (joined through `plugin_config`, `host_software_item`, `service_host`, and `service`), groups them
by agent, and sends `CheckVersionsPayload` messages via `NotificationService`. Agent responses arrive asynchronously through the existing
`VersionCheckResults` wire message handler.

### ServiceCertCheckExecutor

Queries `service_certificates` for non-revoked certificates approaching their renewal window (within 30 days of expiry) and sends `RequestCertRenewal`
messages to the owning services via `NotificationService`.

## What Moved to the Scheduler vs. What Stayed

### Moved to the scheduler (runs on exactly one controller at a time)

| Task | Previously | Now | | --- | --- | --- | | Auth state cleanup | Inline 5-min interval in `main.rs` | `AuthCleanupExecutor` | | MQTT stale lease
cleanup | No dedicated loop | `StaleLeaseCleanupExecutor` | | CA
rotation check | Inline 24h interval in `main.rs` | `CaRotationCheckExecutor` | | Version checking | Not implemented | `VersionCheckExecutor` | |
Service cert renewal check | Not implemented | `ServiceCertCheckExecutor` |

### Stays as per-controller intervals (must run on every controller)

| Task | Reason | | --- | --- | | Settings version check (30s) | In-memory cache sync | | CA state reload (30s) | CA cache sync | | CRL manager (60s +
event-driven) | Security-critical TLS config | | NATS consumer (when configured) | Cross-controller messaging backbone | | Server cert renewal (24h) |
Per-controller disk cert | | Token denylist purge (5min) | In-memory, per-instance | | Ping/pong (agent/MQTT) | Connection keepalive | | CA rotation
execution | Listens on `ca_rotation_trigger`, needs local key store |

## REST API

See [HTTP Web API](../api/http-web-api.md#scheduler-endpoints) for endpoint details.

| Method | Path | Description | | --- | --- | --- | | GET | `/api/v1/scheduler/tasks` | List all tasks for the tenant | | GET |
`/api/v1/scheduler/tasks/{id}` | Get task details | | PUT | `/api/v1/scheduler/tasks/{id}` | Update cron/enabled/config | | POST |
`/api/v1/scheduler/tasks/{id}/trigger` | Trigger immediate execution |

All endpoints require the `ManageSoftware` permission.

## Security Considerations

- The scheduler never runs automatic updates. It triggers version *checks* and certificate *renewal requests* only. Update execution always requires
  explicit user action.
- Optimistic locking prevents concurrent execution of the same task across controllers.
- Task claims have a 10-minute stale timeout to prevent permanent locking if a controller crashes.
- REST API endpoints are protected by JWT authentication and the `ManageSoftware` permission.
- Cron expressions are validated before persistence.

## Related Documentation

- [Cross-Controller Communication](../development/cross-controller-comm.md) -- NATS-based cross-controller messaging
- [HTTP Web API](../api/http-web-api.md) -- REST endpoint documentation
- [Services and Operations](../api/services-operations.md) -- version check and cert renewal flows
- [Security Architecture](../security/security-architecture.md) -- defense-in-depth principles
