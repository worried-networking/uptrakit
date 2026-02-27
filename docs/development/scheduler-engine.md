# Scheduler Engine (Development Guide)

The `uptrakit-scheduler-engine` crate (`crates/shared/scheduler-engine/`) contains the core scheduler
logic extracted from the controller. It is used by both the embedded scheduler (inside the controller,
feature-gated behind `embedded-scheduler`) and the external scheduler binary (`uptrakit-scheduler`).

## Crate purpose

The engine provides:

- **Scheduler loop** — polls for due tasks, claims them with optimistic locking, executes, and releases.
- **Claim mechanism** — `try_claim`, `release_claim`, `recover_stale`, `release_all`, `find_due_tasks`.
- **Cron utilities** — 5-field → 6-field normalization, `next_run_after()`, `validate_cron()`.
- **`TaskExecutor` trait** — implemented by each scheduled task type.
- **`SchedulerNotifier` trait** — abstracts push notification delivery (local vs NATS).
- **Shared query helpers** — `load_software_states_for_tenant()`, `should_rotate_ca()`.
- **Four built-in executors** — `AuthCleanupExecutor`, `StaleLeaseCleanupExecutor`,
  `VersionCheckExecutor`, `ServiceCertCheckExecutor`.

The CA rotation check executor is **not** in the engine — it is mode-specific:

- **Embedded**: `EmbeddedCaRotationCheckExecutor` in `crates/core/controller/` uses
  `watch::Receiver<CaSnapshot>` and `Arc<Notify>`.
- **External**: `ExternalCaRotationCheckExecutor` in `crates/core/scheduler/` reads the CA cert from
  the database and signals via `SchedulerNotifier::signal_ca_rotation()`.

## Module structure

```text
crates/shared/scheduler-engine/src/
    lib.rs              — Re-exports
    scheduler.rs        — Scheduler struct, SchedulerConfig, poll loop
    claim.rs            — try_claim, release_claim, recover_stale, release_all, find_due_tasks
    cron_utils.rs       — Cron parsing (chrono↔time bridge), next_run_after(), validate_cron()
    error.rs            — SchedulerError, Result<T>
    executor.rs         — TaskExecutor trait
    notifier.rs         — SchedulerNotifier trait
    ca_utils.rs         — should_rotate_ca() utility (x509 cert expiry check)
    software_states.rs  — load_software_states_for_tenant() query
    executors/
        mod.rs
        auth_cleanup.rs
        stale_lease_cleanup.rs
        version_check.rs
        service_cert_check.rs
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
    async fn signal_ca_rotation(&self);
    async fn push_software_states_for_tenant(&self, db: &DatabaseConnection, tenant_id: Uuid);
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

### VersionCheckExecutor

Queries enabled software items for the tenant, groups them by agent service, and sends
`CheckVersionsPayload` messages via `SchedulerNotifier::send_to_service()`. Also triggers
controller-side fetch for items with `ControllerSideFetchReleases` plugins and pushes
`SoftwareStates` to MQTT services via `SchedulerNotifier::push_software_states_for_tenant()`.

### ServiceCertCheckExecutor

Queries `service_certificates` for non-revoked certificates within 30 days of expiry and sends
`RequestCertRenewal` messages to the owning services via `SchedulerNotifier::send_to_service()`.

## Shared utilities

### `should_rotate_ca(cert_pem: &str) -> bool`

Parses an X.509 PEM certificate and returns `true` if the certificate expires within 6 months.
Used by both the embedded and external CA rotation check executors.

### `load_software_states_for_tenant(db, tenant_id) -> Vec<SoftwareStateEntry>`

Loads all enabled software items with per-host version data for a tenant. Used by
`VersionCheckExecutor` and `SchedulerNotifier::push_software_states_for_tenant()`.

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
