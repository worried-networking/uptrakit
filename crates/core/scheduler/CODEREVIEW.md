# Code Review: uptrakit-scheduler

- **Review date**: 2026-03-02
- **Parallel review date**: 2026-03-06
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

The external scheduler binary (~753 LoC) is a standalone service that enrolls as a service and
receives infrastructure credentials via WebSocket. It delegates all scheduling logic to the
shared `uptrakit-shared-scheduler-engine` library and registers task executors at startup. The
binary itself is thin and correctly structured, with proper feature flag pass-through and clean
`ServiceHandler` implementation.

The main concerns are inherited issues from the underlying `scheduler-engine` library (sequential
execution model, `SchedulerNotifier` leaking `sea_orm`).

## Architecture

### Strengths

- `Cargo.toml:1-47` -- Clean separation: standalone scheduler binary using shared
  `scheduler-engine` library. At 753 LoC, appropriately thin.
- Uses `CancellationToken` propagation from `service-sdk` for graceful shutdown, consistent
  with the rest of the workspace.
- Adding a new scheduled task type requires only a `Cargo.toml` dependency, a new executor
  struct, and a single `scheduler.register(TaskType, executor)` call.
- Feature flag pass-through for `oidc` and database backends demonstrates proper cascading.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- Receives credentials (DB URL, NATS URL, master key) exclusively over authenticated
  WebSocket, never stored on disk.
- Zero `unsafe` blocks. No database credentials stored in this binary.
- DB connection obtained through the standard `TenantDb` initialization path.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/handler.rs:31-34` -- Clean `SchedulerRuntime` RAII with `CancellationToken`.
- `src/handler.rs:67` -- `type ServiceEvent = std::convert::Infallible` correctly models the
  scheduler's lack of custom events.
- `bail!` / `report!` / `context_to` error handling used correctly throughout. No `unwrap()` in
  production paths.

### Issues

No quality issues found.

## High Availability

### Strengths

- `src/handler.rs:94` -- Properly handles credential re-delivery by stopping the existing
  scheduler first.
- `src/handler.rs:212-221` -- Uses `CancellationToken` for cooperative shutdown of the spawned
  scheduler engine.
- `src/handler.rs:259-278` -- On shutdown, cancels the scheduler (which releases claims) before
  sending disconnect message.
- `recover_stale_claims` mechanism in `scheduler-engine` ensures a crashed scheduler instance
  releases its locks within the stale window (600s), allowing the next instance to pick up
  tasks without operator intervention.

### Issues

**[MEDIUM]** Inherited from `scheduler-engine` -- Sequential task execution within each poll
cycle. If a registered executor is slow, it blocks all other due tasks from running. Under
normal load with many tasks due simultaneously, lower-priority tasks are starved. Consider
a bounded task-pool (`tokio::task::JoinSet` with concurrency cap) for parallel execution.

**[MEDIUM]** Inherited from `scheduler-engine` -- `SchedulerNotifier` trait accepts a
`sea_orm::DatabaseConnection` as a parameter, leaking the ORM type into what should be a clean
notification interface. Replace with a boxed async closure or a simpler trait.

**[MEDIUM]** (2026-03-06 parallel review, HA-9) Race window between external scheduler
disconnect and embedded scheduler noticing. The `external_scheduler_connected` flag is an
`AtomicBool` set by the WebSocket connection handler. If the external scheduler disconnects
between poll cycles (15-second default interval), there is up to a 15-second window where
external tasks are neither executed by the (now-disconnected) external scheduler nor by the
embedded scheduler (which still sees the flag as `true`). In practice the flag is likely
cleared promptly on disconnect, but the worst case is a single missed poll cycle.

**[LOW]** (2026-03-06 parallel review, HA-2) Scheduler claim release on shutdown is
best-effort. In `scheduler-engine/src/scheduler.rs:113-122`, the `release_all_claims` call
during shutdown only logs a warning on failure. If the DB connection is lost at shutdown time,
claims remain locked until the 10-minute stale claim recovery window passes. The stale claim
recovery mechanism adequately handles this, but the window is longer than ideal.

## Coding Standards

### Strengths

- Properly uses workspace dependency inheritance.
- Uses `Backoff` from the service SDK for lifecycle management.

### Issues

**[LOW]** Inherited from `scheduler-engine` -- `DEFAULT_POLL_INTERVAL_SECS` is a
module-private `const u64` inside `scheduler-engine/src/scheduler.rs` without documentation.
Move to a centralized constants file as a typed `Duration` with a doc-comment, consistent with
how all other timing constants in the workspace are handled.

**[LOW]** (2026-03-06 parallel review) Inherited from `scheduler-engine` --
`std::sync::Mutex` used in scheduler-engine tests at `scheduler.rs:539-540` for a
`BlockingExecutor` struct. While technically in `#[cfg(test)]` code, the project standard
mandates `parking_lot::Mutex` everywhere. The practical risk is low, but this is inconsistent
with the stated convention.

## Extensibility

### Strengths

- Feature flag pass-through allows different deployment configurations.
- `TaskExecutor` trait is minimal and object-safe, making it straightforward to add new
  executors.

### Issues

**[MEDIUM]** Inherited from `scheduler-engine` -- `Scheduler::register` uses
`HashMap::insert`, which silently replaces a previously registered executor for a given task
type with no warning. A `debug_assert!` would surface accidental double-registration during
development without impacting production.

**[LOW]** Inherited from `scheduler-engine` -- `TaskExecutor` has no registration or discovery
mechanism ensuring all `ScheduledTaskType` variants have executors. Unlike `register_plugins!`,
there is no compile-time check that prevents a new variant from being added without a
corresponding executor registration in `main.rs`.

## Tests

### Strengths

- `src/cli.rs:17-80` -- 4 tests covering CLI defaults, custom `--poll-interval`, version flag
  parsing without other flags, and directory resolution. Adequate for the narrow argument surface.
- `src/handler.rs:308-319` -- `scheduler_handler_capabilities` verifies the advertised
  capability set (`Scheduler`, `DatabaseAccess`, `NatsAccess`, `MasterKeyAccess`,
  `CaManagement`, `GracefulShutdown`) and asserts the count is exactly 6, guarding against
  accidental removals.
- `src/ca_rotation.rs:162-260` -- Three tests for `ExternalCaRotationCheckExecutor` using
  in-memory SQLite and a `TrackingNotifier` mock: empty DB skips rotation, long-lived cert does
  not trigger, and near-expiry cert triggers. Tests use plain `#[tokio::test]` without
  `start_paused` -- correct because they connect to SQLite (AGENTS.md rule 2).
- `src/ca_rotation.rs:88-114` -- `TrackingNotifier` is a minimal `SchedulerNotifier` mock using
  `Arc<AtomicBool>` to record whether `signal_ca_rotation` was called. Self-contained, no
  mocking framework required, mirrors the `TrackingExecutor` pattern in the controller.
- `src/nats_notifier.rs:83-90` -- `nats_scheduler_notifier_new` verifies construction does not
  panic and that `scheduler_id` is stored. Minimal but meaningful smoke test.

### Issues

**[HIGH]** `src/handler.rs` -- `on_connected` (lines 88-210) and `on_message` (lines 213-254)
have no test coverage. `on_connected` performs the critical credential-delivery sequence:
receives `SchedulerCredentials`, initializes the database connection, initializes the master
key, and starts the scheduler engine. `on_message` handles re-delivery and the `Disconnect`
case. Both the success path and the failure path (e.g., invalid DB URL, master key already
initialized with a different key) are exercised only by live integration. A stub
`ServiceIdentityState` and a mock `ControllerConnection` would allow unit testing of the
credential parsing, startup sequencing, and error-return logic without a running controller.

**[MEDIUM]** `src/ca_rotation.rs` -- The boundary between the rotation threshold (33% of
lifetime remaining) and "no rotation needed" is tested with 1825-day and 30-day certificates.
The exact threshold (cert lifetime / 3) is not directly tested at its boundary value. A
certificate at precisely 33.3% remaining lifetime should trigger; one at 33.4% remaining should
not. An off-by-one in the threshold formula would not be caught by the current tests.

**[MEDIUM]** `src/handler.rs:94-96` -- `stop_scheduler` is called on credential re-delivery to
tear down the previous runtime before starting a new one. This path — receiving credentials a
second time while a scheduler is already running — has no test. A regression here could cause
a double-start or a leaked `CancellationToken` child task.

**[LOW]** `src/nats_notifier.rs` -- `send_to_service`, `broadcast`, `send_by_capability`,
`signal_ca_rotation`, `signal_software_states_changed`, and `signal_crl_renewal` all have no
tests. These are thin NATS publish wrappers, but the topic construction and payload serialization
inside each method are exercised only by integration. At minimum, the `signal_ca_rotation`
topic format should be asserted with a mock NATS connection to guard against breaking the
topic contract between the external scheduler and the controller's subscription.

---

## Test Coverage Analysis (2026-03-05)

Overall crate coverage: 83 / 274 lines (30.3%).

### Per-File Coverage

| File | Coverage | Lines |
| --- | ---: | ---: |
| `main.rs` | 0.0% | 26 |
| `handler.rs` | 17.6% | 182 |
| `nats_notifier.rs` | 25.0% | 20 |
| `cli.rs` | 97.8% | 46 |

### Critical Uncovered Paths

**[BUSINESS] `handler.rs` — credential delivery + scheduler lifecycle (17.6% coverage)**

`SchedulerHandler` processes `ServiceSettings`, `DatabaseCredentials`, `NatsCredentials`, and
`MasterKey` messages. On receiving all credentials, it starts the scheduler engine. On
credential re-delivery, it stops and restarts the scheduler.

Key untested paths:

- Credential accumulation: all four credential types must be received before starting
- `stop_scheduler` on re-delivery: must cleanly tear down before restart
- Error handling when scheduler engine fails to start

Recommended tests:

- Mock `ControllerConnection` to deliver credentials sequentially
- Verify scheduler starts only after all credentials are received
- Verify `stop_scheduler` + restart on credential re-delivery

**[BUSINESS] `nats_notifier.rs` — topic construction (25.0% coverage)**

NATS publish wrappers with topic construction. The topic format is a contract between the
scheduler and the controller. At minimum, topic string formatting should be unit-tested.

---

## Review — 2026-03-10

- **Reviewer**: AI code review (HA|consistency|tests)
- **Scope**: Scheduler-specific findings from HA, code quality, and consistency passes

### High Availability

#### Strengths

- **(H5, confirmed)** Backoff implementation is correct and complete. `Backoff::new(2s, 60s)`
  with jitter, interruptible via `tokio::select!`, resets on success. This is a well-designed
  resilience pattern used appropriately in the scheduler's reconnect loop.

#### Issues

**[MEDIUM]** (H1) `src/handler.rs` (`on_message`) — A transient DB or NATS failure while
handling a `ServiceCredentials` message causes `std::process::exit(1)`, permanently killing
the scheduler process. A transient database unavailability at credential delivery time becomes
an unrecoverable outage requiring operator intervention to restart the binary. Recommendation:
for transient infrastructure errors (`DbErr::Conn`, NATS timeout), return `Ok(None)` or map
to a reconnect-triggering error variant so the event loop can retry. Reserve
`std::process::exit(1)` for configuration errors (invalid key material, wrong key length)
where retrying would not help.

**[MEDIUM]** (H2) `crates/shared/scheduler-engine/src/scheduler.rs:110-128` —
`poll_cycle` drains all due tasks synchronously before returning to the interval tick.
During this drain, the `token.cancelled()` arm of the surrounding `tokio::select!` is not
evaluated, so a cooperative shutdown signal can be delayed by up to `TASK_EXECUTION_TIMEOUT`
(2 hours) if a task happens to be running at shutdown time. Recommendation: wrap the
`poll_cycle` call in a nested `tokio::select!` that also monitors `token.cancelled()`, so
shutdown is responsive even while a long-running task is in progress.

**[LOW]** (H3) `crates/shared/scheduler-engine/src/claim.rs:19` — Stale claim recovery is
logged at `info` level when `recovered > 0`. Because stale claims indicate a previous
instance crashed or lost connectivity without releasing its locks, this event is operationally
significant and warrants `warn` level. An application metric counter (e.g., via `metrics::counter!`)
would also enable post-incident analysis of crash frequency without requiring log triage.

**[LOW]** (H4) `src/handler.rs:278-297` — `on_shutdown` ignores the `shutdown_timeout`
parameter (shadowed as `_shutdown_timeout`) and instead applies a hard-coded
`STOP_SCHEDULER_TIMEOUT = 30s` regardless of the controller-configured value. If the controller
sends a shutdown with a different timeout expectation, the scheduler may stop its task loop
prematurely or linger longer than intended. Recommendation: use the parameter value as the
timeout and document `STOP_SCHEDULER_TIMEOUT` as a safety upper bound to prevent infinite
waits.

### Consistency

#### Issues

**[MEDIUM]** (C1) `src/nats_notifier.rs` — `nats_scheduler_notifier_new` test (lines 83-90)
is a no-op stub that asserts `std::mem::size_of::<Uuid>() == 16`. This assertion tests the
Rust standard library, not any application behavior. The test provides no isolation-level
value for the scheduler crate and would pass even if the NATS subject construction were
completely broken. Already noted under the Tests section above (`[LOW] src/nats_notifier.rs`)
as lacking coverage for topic construction; this finding confirms the sole existing test in
that module is not meaningful. Recommendation: remove the stub test and replace it with a
mock-NATS test that verifies the subject format produced by `signal_ca_rotation` (and at
minimum one other method) matches the string the controller subscribes to.

## 2026-03-10 12-Dimension Review Update

Comprehensive 12-dimension review covering architecture, security, code quality, tests, HA,
database, coding standards, extensibility, consistency, idiomatic Rust, references & heap,
and maintainability.

### Dimension: High Availability (D5)

#### Strengths

- `crates/shared/scheduler-engine/src/claim.rs` -- Robust optimistic locking. `try_claim`
  uses a single atomic `UPDATE WHERE locked_by IS NULL` statement, making task claim
  acquisition TOCTOU-free. Two concurrent schedulers racing for the same task have exactly
  one winner determined by the database engine.
- `crates/shared/scheduler-engine/src/scheduler.rs` -- Claims released on shutdown. The
  `token.cancelled()` arm in the scheduler loop calls `release_all_claims` before exiting,
  ensuring a clean shutdown does not leave stale locks that would block task execution for
  the 10-minute recovery window.
- `crates/shared/scheduler-engine/src/scheduler.rs` -- Concurrent execution within each poll
  cycle. Due tasks discovered by `find_due_tasks` are dispatched to their registered executors
  and awaited, allowing multiple independent tasks to make progress within a single poll
  interval rather than being strictly serialized.
- `crates/shared/scheduler-engine/src/scheduler.rs` -- Unknown task types are skipped during
  rolling upgrades. When a new scheduler version adds a `ScheduledTaskType` variant but an
  older instance is still running, the older instance encounters the unknown type in
  `find_due_tasks` results and skips it with a log warning rather than panicking. This enables
  safe rolling deployments where new task types are seeded in the database before all scheduler
  instances are upgraded.

## Review — 2026-03-15

- **Reviewer**: AI code review (HA)
- **Branch**: docs/codereview-backend

### High Availability

#### Strengths

- Optimistic locking (`LockedBy` column with `UPDATE WHERE locked_by IS NULL`) makes task
  claims TOCTOU-free. Two concurrent scheduler instances racing for the same task have exactly
  one winner determined by the database engine.
- Stale claim recovery runs at the start of each poll cycle: claims older than 10 minutes are
  released automatically, so a crashed instance's locks are recovered within one poll window
  without operator intervention.
- Shutdown cancellation has priority in the biased `select!`: the `token.cancelled()` arm is
  evaluated before the poll-cycle arm, ensuring a shutdown signal is never missed due to a
  runaway task occupying the loop.
- External vs. embedded scheduler is controlled by the `external_scheduler_connected` flag.
  When the external scheduler disconnects, the embedded scheduler takes over within one poll
  cycle.

#### Issues

**[LOW]** No fairness guarantee across task types. The fastest DB responder claims any due task
regardless of priority or type. Under sustained load with many tasks due simultaneously, a
fast-executing high-frequency task type could starve slower or lower-frequency types. Acceptable
given DB-level duplicate prevention, but worth noting for future scheduler evolution if priority
lanes are introduced.
