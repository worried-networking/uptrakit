# Code Review: uptrakit-scheduler

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
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

## Coding Standards

### Strengths

- Properly uses workspace dependency inheritance.
- Uses `Backoff` from the service SDK for lifecycle management.

### Issues

**[LOW]** Inherited from `scheduler-engine` -- `DEFAULT_POLL_INTERVAL_SECS` is a
module-private `const u64` inside `scheduler-engine/src/scheduler.rs` without documentation.
Move to a centralized constants file as a typed `Duration` with a doc-comment, consistent with
how all other timing constants in the workspace are handled.

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
`signal_ca_rotation`, `push_software_states_for_tenant`, and `signal_crl_renewal` all have no
tests. These are thin NATS publish wrappers, but the topic construction and payload serialization
inside each method are exercised only by integration. At minimum, the `signal_ca_rotation`
topic format should be asserted with a mock NATS connection to guard against breaking the
topic contract between the external scheduler and the controller's subscription.
