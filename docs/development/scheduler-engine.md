# Scheduler Engine (Development Guide)

The `uptrakit-scheduler-engine` crate (`crates/shared/scheduler-engine/`) contains the core scheduler
logic extracted from the controller. It is used by both the embedded scheduler (inside the controller,
feature-gated behind `embedded-scheduler`) and the external scheduler binary (`uptrakit-scheduler`).

## Crate purpose

The engine provides:

- **Scheduler loop** — polls for due tasks, claims them with optimistic locking, executes, and releases.
- **Claim mechanism** — `try_claim`, `release_claim`, `recover_stale`, `release_all`, `find_due_tasks`.
- **Interval utilities** — `compute_next_run_at(now, interval_seconds, jitter_seconds)` for next-run computation with random jitter.
- **`TaskExecutor` trait** — implemented by each scheduled task type.
- **`SchedulerNotifier` trait** — abstracts push notification delivery (local vs NATS).
- **Shared query helpers** — `load_software_states_for_tenant()`, `should_rotate_ca()`.
- **Six built-in executors** — `AuthCleanupExecutor`, `StaleLeaseCleanupExecutor`,
  `FetchReleasesExecutor`, `DetectVersionExecutor`, `ServiceCertCheckExecutor`, `CrlRenewalExecutor`.
  Of these, `CrlRenewalExecutor` and `ServiceCertCheckExecutor` are **internal** tasks registered
  only by the embedded scheduler (they require in-process controller resources).
- **Shared query helpers** — `queries.rs` contains `AgentAssignmentRow`,
  `merge_config`, and `query_agent_assignment_rows`, shared
  by `FetchReleasesExecutor` and `DetectVersionExecutor`.

The CA rotation check executor (`EmbeddedCaRotationCheckExecutor`) is **not** in the engine — it
lives in `crates/core/controller/src/scheduler/` and uses in-process `watch::Receiver<CaSnapshot>`
and `Arc<Notify>`. It is an internal task registered only by the embedded scheduler.

## Module structure

```text
crates/shared/scheduler-engine/src/
    lib.rs              — Re-exports
    scheduler.rs        — Scheduler struct, SchedulerConfig, poll loop
    claim.rs            — try_claim, release_claim, recover_stale, release_all, find_due_tasks
    interval.rs         — compute_next_run_at(now, interval_seconds, jitter_seconds)
    error.rs            — SchedulerError, Result<T>
    executor.rs         — TaskExecutor trait
    notifier.rs         — SchedulerNotifier trait (+ NoopSchedulerNotifier for tests)
    ca_utils.rs         — should_rotate_ca() utility (x509 cert expiry check)
    software_states.rs  — load_software_states_for_tenant() query
    executors/
        mod.rs
        auth_cleanup.rs
        stale_lease_cleanup.rs
        queries.rs          — Shared: AgentAssignmentRow, merge_config,
                              query_agent_assignment_rows
        fetch_releases.rs   — FetchReleasesExecutor (Phase A parallel + Phase B agent dispatch)
        detect_version.rs   — DetectVersionExecutor (agent-side installed-version detection)
        service_cert_check.rs
        crl_renewal.rs
```

## Key traits

### TaskExecutor

```rust
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(&self, task: &scheduled_task::Model) -> Result<(), String>;
}
```

Each executor receives the full `scheduled_task::Model` and returns `Ok(())` on success or
`Err(message)` on failure. The scheduler stores the error message in `last_error`.

### SchedulerNotifier

```rust
#[async_trait]
pub trait SchedulerNotifier: Send + Sync {
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage);
    async fn broadcast(&self, msg: ControllerMessage);
    async fn send_by_capability(&self, capability: &str, msg: ControllerMessage);
    async fn signal_ca_rotation(&self, reason: &str);
    async fn signal_software_states_changed(&self, tenant_id: Uuid);
    /// Trigger an immediate CRL rebuild on all controller instances.
    async fn signal_crl_renewal(&self);
}
```

Two implementations exist:

| Implementation | Location | Transport |
| --- | --- | --- |
| `ControllerSchedulerNotifier` | `crates/core/controller/src/scheduler/mod.rs` | `NotificationService` (local + optional NATS) |
| `NatsSchedulerNotifier` | `crates/core/scheduler/src/nats_notifier.rs` | `NatsConnection` (NATS only) |

## SchedulerConfig

```rust
pub struct SchedulerConfig {
    pub poll_interval: Duration,
    pub controller_id: Uuid,
    pub tenant_id: Uuid,
}
```

- `poll_interval`: how often the scheduler polls for due tasks (default: 15s).
- `controller_id`: unique identifier for this scheduler instance (used for claim locking).
- `tenant_id`: the tenant scope for task queries.

## Scheduler fields

The `Scheduler` struct accepts a `should_yield_external: Box<dyn Fn() -> bool + Send + Sync>`
closure in its constructor. When the closure returns `true`, the poll cycle skips non-internal
tasks (tasks where `ScheduledTaskType::is_internal()` returns `false`), deferring them to the
external scheduler. Internal tasks (CRL renewal, CA rotation check, service cert check) always
execute regardless of the closure's return value.

In the embedded scheduler, this closure queries the `EmbeddedServiceNotifier` trait to check
whether the `Scheduler` capability is currently yielded. In the external scheduler binary, the
closure always returns `false` (the external scheduler never skips its own tasks).

See [Embedded Services Architecture](../architecture/embedded-services.md) for how the controller
manages the yield lifecycle.

## Feature flags

| Feature | Default | Description |
| --- | --- | --- |
| `oidc` | No | Enables OIDC-related cleanup in `AuthCleanupExecutor` |

When `oidc` is disabled, `AuthCleanupExecutor` skips the OIDC flow store cleanup calls
(`OidcFlowStore`, `AccountLinkStore`, `OidcTokenExchangeStore`, `OidcRegistrationStore`).

## Executor details

### AuthCleanupExecutor

Cleans expired authentication flow state from the database. With the `oidc` feature enabled, it cleans:
`OidcFlowStore`, `AccountLinkStore`, `OidcTokenExchangeStore`, `OidcRegistrationStore`. Always cleans:
`DeviceFlowStore`, `RateLimitStore`.

### StaleLeaseCleanupExecutor

Releases stale MQTT client leases by running a direct DELETE query against the `mqtt_leases` table
for entries whose `last_heartbeat` exceeds the stale threshold.

### FetchReleasesExecutor

Handles the `fetch_releases` task. Phase A and Phase B run concurrently via `tokio::join!`.

**Phase A — Controller-side fetch (parallel):** Queries `host_software_item_plugins` with
`role = 'fetch_releases'` targeting the controller. Groups by
`(plugin_config_id, assignment_config)`, instantiates one plugin per group, and runs
`batch_fetch_releases` per group in a `JoinSet` bounded by a
`Semaphore(MAX_CONCURRENT_CONTROLLER_FETCHES = 10)`. After all spawned tasks complete, the
DB update loop runs sequentially: updates successful `host_software_items.latest_version` rows,
batch-updates `software_item.last_checked_at`, and signals that software states have changed via
`SchedulerNotifier::signal_software_states_changed(tenant_id)`. When a group-level batch call
returns `Err` or a per-item result contains `BatchFetchResult.error`, Phase A skips DB writes for
the affected rows and preserves their previously stored `host_software_items.latest_version`.
The controller's event delivery layer handles loading and pushing states to update-tracking
services.

**Phase B — Agent-side dispatch:** Calls `query_agent_assignment_rows` with `roles = ["fetch_releases"]`,
builds `VersionCheckAssignment` with only `fetch_releases` set, and sends `CheckVersions` messages
to agents.

### DetectVersionExecutor

Handles the `detect_version` task. Calls `query_agent_assignment_rows` with
`roles = ["detect_version"]` for software items. Builds `VersionCheckAssignment` with only
`detect_version` set and sends
`CheckVersions` messages. Agent responses arrive asynchronously via the existing
`VersionCheckResults` wire message handler.

The `detect_version` task was introduced by migration `m20260307_000001_split_version_check`, which
split the old single `version_check` task into `fetch_releases` (every 6 hours, heavier API work)
and `detect_version` (daily, agent-side only). Running them on independent schedules lets operators
tune each cadence independently.

### ServiceCertCheckExecutor

Queries `service_certificates` for non-revoked certificates within 30 days of expiry and sends
`RequestCertRenewal` messages to the owning services via `SchedulerNotifier::send_to_service()`.

### CrlRenewalExecutor

Triggers a CRL rebuild on all controller instances by calling `SchedulerNotifier::signal_crl_renewal()`.

- **Embedded mode** (`ControllerSchedulerNotifier`): fires `revocation_notify.notify_one()` and publishes
  `ControllerMessage::RequestCrlRenewal` to NATS for remote controllers.
- **External mode** (`NatsSchedulerNotifier`): publishes `ControllerMessage::RequestCrlRenewal` to the
  NATS `controller` capability subject; each receiving controller fires its own `revocation_notify`.

Default interval: 14 400 seconds (every 4 hours) with 120 seconds jitter. The interval is configurable at
runtime via the scheduler task management API (`PUT /api/v1/scheduler/tasks/{id}`) without a restart.

The `CrlRenewal` task row is seeded for all tenants by migration `m20260305_000001_crl_cache`.

## Shared utilities

### `should_rotate_ca(cert_pem: &str) -> bool`

Parses an X.509 PEM certificate and returns `true` if the certificate expires within 6 months.
Used by both the embedded and external CA rotation check executors.

### `load_software_states_for_tenant(db, tenant_id) -> Vec<SoftwareStateEntry>`

Loads all enabled software items with per-host version data for a tenant. The canonical
implementation now lives in `web-api-queries/software_states.rs`. Used by the controller's
event delivery layer when handling `SoftwareStatesChanged` signals.

## Testing

```bash
# Unit tests (default features)
cargo test -p uptrakit-scheduler-engine

# With OIDC feature
cargo test -p uptrakit-scheduler-engine --features oidc
```

## Dependencies

The engine depends on `uptrakit-shared-db`, `uptrakit-internal-wire`, `uptrakit-shared-types`,
`uptrakit-command`, `uptrakit-plugin-infrastructure-core`, and `uptrakit-plugin-infrastructure-registry`.
It does **not** depend on `uptrakit-web-api`, `uptrakit-nats`, or any controller-specific crate.

## Related documentation

- [Scheduler Architecture](../architecture/scheduler.md) — database schema, HA claim mechanism, REST API
- [Cross-Controller Communication](cross-controller-comm.md) — NATS-based messaging
- [NATS Integration](nats-integration.md) — NATS development guide
- [External Scheduler Deployment](../end-user/deployment/external-scheduler.md) — production deployment
- [Service Lifecycle](service-lifecycle.md) — `ServiceHandler` trait used by the external scheduler binary
