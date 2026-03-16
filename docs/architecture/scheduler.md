# Scheduler

The centralised DB-backed scheduler coordinates periodic background tasks using optimistic locking on
the `scheduled_tasks` table for HA-safe exactly-once execution.

## Overview

The scheduler ensures that periodic work (release fetching, installed-version detection, cleanup,
CA rotation, certificate renewal, CRL renewal) runs exactly once, regardless of how many
controller instances are deployed.

Key properties:

- **HA-safe**: only one instance executes a given task at a time (optimistic lock via `locked_by`/`locked_at` columns).
- **Stale recovery**: tasks locked longer than 10 minutes are automatically released.
- **Interval+jitter**: each task runs on a fixed interval (seconds) with configurable random jitter to spread load.
- **REST-manageable**: administrators can view, update schedules, and trigger immediate execution via the REST API.

## Deployment modes

The scheduler engine (`uptrakit-scheduler-engine`) is a shared library crate used in two modes:

| Mode | Binary | Feature | How it runs |
| --- | --- | --- | --- |
| Embedded | `uptrakit-controller` | `embedded-scheduler` (not default) | Spawned inside the controller process |
| External | `uptrakit-scheduler` | Always | Standalone binary, enrolls as a service via WebSocket |

### Embedded scheduler

When built with `--features embedded-scheduler`, the controller spawns the scheduler loop internally.
Tasks are categorised as **internal** or **external** (see below). When an external scheduler connects
(detected via `Capability::Scheduler` in `ServiceConnectionRegistry`), the embedded scheduler
automatically defers external tasks (the external scheduler handles them) while continuing to execute
internal tasks that require in-process controller resources. When the last external scheduler
disconnects, the embedded scheduler resumes all tasks. This provides seamless failover.

### External scheduler

The `uptrakit-scheduler` binary enrolls as a **system service** with capabilities `system_service`,
`scheduler`, `database_access`, `nats_access`, `master_key_access`, and `graceful_shutdown`. The
`system_service` capability causes the controller to route enrollment through the system-service
path, which grants the scheduler infrastructure credentials (database URL, NATS URL, master
encryption key) without any tenant-scoped restrictions.

The external scheduler is **tenant-agnostic**: it polls the `scheduled_tasks` table across **all
tenants** on every cycle. No tenant filter is applied in `find_due_tasks()`; each returned row
carries its own `tenant_id` and executors that require it (e.g. `FetchReleasesExecutor`,
`DetectVersionExecutor`) read it directly from the task model.

The external scheduler registers only the 4 external task types. Internal tasks are not registered
because they require in-process controller resources.

### Internal vs external task categorisation

Tasks are categorised based on whether they require direct in-process access to controller resources:

| Task | Category | Rationale |
| --- | --- | --- |
| `CrlRenewal` | Internal | Direct `revocation_notify` + NATS publish via `ControllerSchedulerNotifier` |
| `CaRotationCheck` | Internal | In-process `watch::Receiver<CaSnapshot>` + `ca_rotation_trigger` |
| `ServiceCertCheck` | Internal | `RequestCertRenewal` via `NotificationService` to connected services |
| `AuthCleanup` | External | Pure DB cleanup |
| `StaleLeaseCleanup` | External | Pure DB cleanup |
| `FetchReleases` | External | HTTP + DB + agent dispatch |
| `DetectVersion` | External | DB + agent dispatch |
| `DiscoverSoftware` | External | DB + agent dispatch (periodic software rediscovery) |

The `ScheduledTaskType::is_internal()` method encodes this categorisation. The `Scheduler` struct
holds an `external_scheduler_connected: Arc<AtomicBool>` flag that is set/cleared by the WebSocket
handler when an external scheduler connects/disconnects. During each poll cycle, the embedded
scheduler skips non-internal tasks when the flag is `true`.

See [External Scheduler Deployment](../end-user/deployment/external-scheduler.md) for production guidance
and [Scheduler Engine (Development)](../development/scheduler-engine.md) for engine internals.

## Database Schema

### `scheduled_tasks` table

| Column | Type | Description |
| --- | --- | --- |
| `id` | UUID (PK, v7) | Task identifier |
| `tenant_id` | UUID FK | References `tenants.id` — scopes the *configuration* (interval, enabled flag, task config) to a specific tenant. Does **not** restrict which scheduler instance processes the task; the external scheduler queries across all tenants. |
| `task_type` | TEXT | Enum discriminant (see below) |
| `interval_seconds` | INTEGER | How often the task runs (in seconds). Must be > 0. |
| `jitter_seconds` | INTEGER | Random jitter added to the interval to spread load (in seconds). Must be >= 0. |
| `enabled` | BOOLEAN | Whether the task is active |
| `task_config` | JSON (nullable) | Per-task configuration |
| `last_run_at` | TIMESTAMP (nullable) | Last successful execution |
| `next_run_at` | TIMESTAMP | Next scheduled execution |
| `locked_by` | UUID (nullable) | Controller ID holding the claim |
| `locked_at` | TIMESTAMP (nullable) | When the claim was acquired |
| `last_error` | TEXT (nullable) | Error message from last failed run |
| `run_count` | BIGINT | Total successful executions |
| `created_at` | TIMESTAMP | Row creation time |
| `updated_at` | TIMESTAMP | Last modification time |

### Indexes

| Name | Columns | Purpose |
| --- | --- | --- |
| `idx_scheduled_tasks_next_run` | `next_run_at` | Efficient due-task lookup |
| `idx_scheduled_tasks_tenant_id` | `tenant_id` | Tenant-scoped queries |
| `uq_scheduled_tasks_tenant_task_type` | `tenant_id, task_type` (UNIQUE) | One task per type per tenant |

### Task types

| Value | Default interval | Default jitter | Description |
| --- | --- | --- | --- |
| `auth_cleanup` | 300 s (5 min) | 30 s | Clean expired auth flow state from DB |
| `stale_lease_cleanup` | 300 s (5 min) | 30 s | Release stale MQTT client leases |
| `ca_rotation_check` | 86 400 s (24 h) | 300 s | Check if managed CA needs rotation |
| `fetch_releases` | 21 600 s (6 h) | 300 s | Fetch latest available versions (controller-side API calls + agent-side package index queries). Replaces the old `version_check` task. |
| `detect_version` | 86 400 s (24 h) | 300 s | Detect currently installed versions on all agent hosts. |
| `service_cert_check` | 43 200 s (12 h) | 300 s | Proactive certificate renewal for services |
| `crl_renewal` | 14 400 s (4 h) | 120 s | Trigger CRL rebuild on all controller instances |
| `audit_log_cleanup` | 86 400 s (24 h) | 300 s | Delete audit log entries older than the retention period (disabled by default) |
| `discover_software` | 21 600 s (6 h) | 300 s | Periodically rediscover installed software on all active hosts. Items that disappear from the agent's report are automatically soft-deleted. |

All rows are seeded during migrations with `next_run_at = now`. The `detect_version` row is
seeded by migration `m20260307_000001_split_version_check` (one per tenant), which also renames
any existing `version_check` rows to `fetch_releases`. The `discover_software` row is seeded
by migration `m20260312_000002_discover_software_task`.

## HA Claim Mechanism

The claim pattern mirrors `MqttLeaseCoordinator` (see [Cross-Controller Communication](../development/cross-controller-comm.md)):

1. **Poll**: every 15 seconds, each controller polls for due tasks (`next_run_at <= now`, `enabled = true`, `locked_by IS NULL`).
1. **Claim**: `UPDATE SET locked_by = $me, locked_at = $now WHERE id = $id AND locked_by IS NULL` -- succeeds only if `rows_affected == 1`.
1. **Execute**: run the task's executor.
1. **Release**: clear `locked_by`/`locked_at`, update `last_run_at`, `run_count`, and optionally `last_error`.
   Compute `next_run_at = now + interval_seconds + rand(0..=jitter_seconds)`.
1. **Stale recovery**: on each poll cycle, tasks with `locked_at < now - 10 min` are released (the controller may have crashed).
1. **Shutdown**: all claims held by the stopping controller are released.

Manual trigger via the REST API sets `next_run_at = now`, making the task immediately eligible on the next poll cycle.

## Task Execution Timeout and Cancellation

### Per-task execution timeout

Each task execution is wrapped in `tokio::time::timeout` using the constant:

```rust
const TASK_EXECUTION_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60); // 2 hours
```

If a task runs longer than this limit, the timeout fires and the task receives
`SchedulerError::TaskTimedOut`. The claim is then released with the timeout recorded so that
the next poll cycle can re-claim and retry the task. This prevents a runaway task from holding
its claim indefinitely, which would otherwise block future executions until the 10-minute stale
recovery fires.

The timeout is also exposed as `SchedulerConfig.task_execution_timeout` (`Duration`, default 2 hours)
so that integration tests and unusual deployments can override it without recompiling.

### Cancellation awareness

`poll_cycle` accepts a `CancellationToken`. The scheduler checks `token.is_cancelled()` before
attempting to claim each due task, skipping any remaining work if shutdown has been requested.

During task execution the scheduler uses a `biased` `tokio::select!` block that gives the
cancellation branch priority:

```rust
tokio::select! {
    biased;
    _ = token.cancelled() => { /* release claim, stop */ }
    result = tokio::time::timeout(config.task_execution_timeout, executor.execute(task)) => { /* handle result */ }
}
```

The `biased` qualifier means Tokio always checks the cancellation branch first, regardless of
readiness order. This ensures that a shutdown signal is honoured promptly even while a task is
mid-execution, rather than waiting for the task to finish or time out.

## Module Structure

The scheduler engine is a shared library crate:

```text
crates/shared/scheduler-engine/src/
    lib.rs              -- Re-exports
    scheduler.rs        -- Scheduler struct, SchedulerConfig, poll loop
    interval.rs         -- compute_next_run_at(now, interval_seconds, jitter_seconds)
    claim.rs            -- try_claim, release_claim, recover_stale, release_all, find_due_tasks
    executor.rs         -- TaskExecutor trait
    notifier.rs         -- SchedulerNotifier trait
    ca_utils.rs         -- should_rotate_ca() utility
    software_states.rs  -- load_software_states_for_tenant() query
    executors/
        mod.rs
        auth_cleanup.rs
        stale_lease_cleanup.rs
        queries.rs                  -- Shared agent-assignment query helpers (AgentAssignmentRow, merge_config, …)
        fetch_releases.rs           -- FetchReleasesExecutor (was version_check.rs)
        detect_version.rs           -- DetectVersionExecutor
        discover_software.rs   -- DiscoverSoftwareExecutor
        service_cert_check.rs
```

The CA rotation check executor lives in the controller:

- `EmbeddedCaRotationCheckExecutor` in `crates/core/controller/src/scheduler/` — uses the in-process
  CA watch channel and `ca_rotation_trigger` directly. It is an internal task and does not run on the
  external scheduler.

### TaskExecutor trait

```rust
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(&self, task: &scheduled_task::Model) -> Result<(), String>;
}
```

Each executor implements this trait with the task-specific logic.

### Interval handling

The `interval.rs` module provides `compute_next_run_at(now, interval_seconds, jitter_seconds)`. It
adds `interval_seconds` to `now`, then adds a random value in `[0, jitter_seconds]` to spread task
executions across instances and avoid thundering-herd effects. No external crate dependencies are
required (no `cron` or `chrono`).

## Executor Details

### AuthCleanupExecutor

Calls `cleanup_expired()` on all DB-backed auth flow stores: `OidcFlowStore`, `AccountLinkStore`, `OidcTokenExchangeStore`, `OidcRegistrationStore`,
`DeviceFlowStore`, and `RateLimitStore`. The in-memory `TokenDenylist` purge remains a per-controller interval (not scheduled here).

### StaleLeaseCleanupExecutor

Creates a `MqttLeaseCoordinator` and calls `cleanup_stale_leases()` to release MQTT client leases that have been held without heartbeat for longer
than the stale threshold.

### CaRotationCheckExecutor (internal)

`EmbeddedCaRotationCheckExecutor` checks `should_rotate_ca()` against the in-process CA snapshot
(`watch::Receiver<CaSnapshot>`). If rotation is needed, fires the `ca_rotation_trigger`
(`Arc<Notify>`) directly. This is an internal task — it runs exclusively on the embedded scheduler
because it requires in-process access to the CA watch channel.

### FetchReleasesExecutor

Handles the **fetch_releases** scheduled task. Runs two phases concurrently via `tokio::join!`:

**Phase A — Controller-side fetch:** Queries `host_software_item_plugins` rows with
`role = 'fetch_releases'` that target the controller (`execution_site = 'controller'` or `'auto'`
with `ControllerSideFetchReleases` capability). Groups by `(plugin_config_id, package_identifier)`,
instantiates each plugin, then spawns all fetch calls into a `JoinSet` bounded by a `Semaphore`
(max [`MAX_CONCURRENT_CONTROLLER_FETCHES`] = 10). After all fetches complete, stores
`latest_version` in `host_software_items`, batch-updates `software_item.last_checked_at`, and
signals `SoftwareStatesChanged` so the controller pushes updated states to update-tracking services.

**Phase B — Agent-side dispatch:** Builds `VersionCheckAssignment` per
`(service_id, host_machine_id)` with only `fetch_releases` set (no `detect_version`, no host
packages) and sends `CheckVersions` messages to agents that run package-index plugins
(APT, Homebrew, npm).

### DetectVersionExecutor

Handles the **detect_version** scheduled task. Queries `host_software_item_plugins` rows with
`role = 'detect_version'`, groups by agent, and sends `CheckVersions` messages
with only `detect_version` set. Agent responses arrive asynchronously through the existing
`VersionCheckResults` wire message handler.

This executor was introduced when the old `version_check` task was split in two (migration
`m20260307_000001_split_version_check`). Running release fetching and installed-version detection
on independent schedules lets operators tune each cadence independently — for example, fetch
releases every 6 hours but detect installed versions once daily.

### ServiceCertCheckExecutor

Queries `service_certificates` for non-revoked certificates approaching their renewal window (within 30 days of expiry) and sends `RequestCertRenewal`
messages to the owning services via `NotificationService`.

### DiscoverSoftwareExecutor

Handles the **discover_software** scheduled task. On each cycle:

1. Queries all active, non-deactivated hosts that have a connected agent service
   (`host → service_host → service`, filtering enabled/non-deactivated rows).
2. Loads per-tenant and per-host discovery allowlists, plus all enabled plugin configs for
   discovery-capable plugin types, in a single `tokio::try_join!` call.
3. For each host, applies the allowlist precedence rule (host-specific → tenant-wide → all
   discovery types) to compute the effective set of plugin types.
4. Builds a `Vec<DiscoveryPluginAssignment>` — one entry per plugin config for each effective
   type (or one empty-config default if no config exists for a type).
5. Sends `ControllerMessage::DiscoverSoftware` to the host's agent service via
   `notifier.send_to_service()`.

Agent responses arrive via the existing `DiscoverSoftwareResult` wire message handler, which
routes each `DiscoveryPluginResult` through `process_plugin_result()`. The handler also calls
`deactivate_missing_items()` to soft-delete host-software junction rows for items absent
from the latest discovery snapshot.

Casks with `"auto_updates": true` are excluded by the Homebrew plugin before the result is
sent, so they never appear in the software list.

## What Moved to the Scheduler vs. What Stayed

### Moved to the scheduler (runs on exactly one controller at a time)

| Task | Previously | Now |
| --- | --- | --- |
| Auth state cleanup | Inline 5-min interval in `main.rs` | `AuthCleanupExecutor` |
| MQTT stale lease cleanup | No dedicated loop | `StaleLeaseCleanupExecutor` |
| CA rotation check | Inline 24h interval in `main.rs` | `CaRotationCheckExecutor` |
| Release fetching | Not implemented | `FetchReleasesExecutor` |
| Installed-version detection | Not implemented | `DetectVersionExecutor` |
| Service cert renewal check | Not implemented | `ServiceCertCheckExecutor` |
| CRL periodic renewal | 60-second poll loop in `CrlManager::run()` | `CrlRenewalExecutor` (default every 4 h) |
| Periodic software rediscovery | Not implemented | `DiscoverSoftwareExecutor` |

### Stays as per-controller intervals (must run on every controller)

| Task | Reason |
| --- | --- |
| Settings version check (30s) | In-memory cache sync |
| CA state reload (30s) | CA cache sync |
| CRL rebuild (event-driven) | Fires on `revocation_notify` — triggered by local revocation, NATS `RequestCrlRenewal`, or `CrlRenewalExecutor` |
| NATS consumer (when configured) | Cross-controller messaging backbone |
| Server cert renewal (24h) | Per-controller disk cert |
| Token denylist purge (5min) | In-memory, per-instance |
| Ping/pong (agent/MQTT) | Connection keepalive |
| CA rotation execution | Listens on `ca_rotation_trigger`, needs local key store |

## REST API

See [HTTP Web API](../api/http-web-api.md#scheduler-endpoints) for endpoint details.

| Method | Path | Description |
| --- | --- | --- |
| GET | `/api/v1/scheduler/tasks` | List all tasks for the tenant |
| GET | `/api/v1/scheduler/tasks/{id}` | Get task details |
| PUT | `/api/v1/scheduler/tasks/{id}` | Update interval/jitter/enabled/config |
| POST | `/api/v1/scheduler/tasks/{id}/trigger` | Trigger immediate execution |

All endpoints require the `ManageSoftware` permission.

## Security Considerations

- The scheduler never runs automatic updates. It triggers release fetching, installed-version
  detection, and certificate *renewal requests* only. Update execution always requires explicit
  user action.
- Optimistic locking prevents concurrent execution of the same task across controllers.
- Task claims have a 10-minute stale timeout to prevent permanent locking if a controller crashes.
- Each task execution is bounded by a 2-hour per-task timeout (`TASK_EXECUTION_TIMEOUT`). A task that exceeds
  this receives `SchedulerError::TaskTimedOut` and its claim is released immediately.
- REST API endpoints are protected by JWT authentication and the `ManageSoftware` permission.
- Interval and jitter values are validated before persistence (`interval_seconds > 0`, `jitter_seconds >= 0`).

## Related Documentation

- [Scheduler Engine (Development)](../development/scheduler-engine.md) -- engine crate internals
- [External Scheduler Deployment](../end-user/deployment/external-scheduler.md) -- production deployment
- [Cross-Controller Communication](../development/cross-controller-comm.md) -- NATS-based cross-controller messaging
- [HTTP Web API](../api/http-web-api.md) -- REST endpoint documentation
- [Services and Operations](../api/services-operations.md) -- version check and cert renewal flows
- [Security Architecture](../security/security-architecture.md) -- defense-in-depth principles
