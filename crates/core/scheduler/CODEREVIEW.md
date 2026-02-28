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

The main concerns are the `stop_scheduler` race condition that can allow two scheduler instances
to operate concurrently during credential re-delivery, and inherited issues from the underlying
`scheduler-engine` library (sequential execution model, `SchedulerNotifier` leaking `sea_orm`).

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

**[HIGH]** `src/handler.rs:51-59` -- `stop_scheduler` does not await the scheduler
`JoinHandle`. If called from `on_message` (line 94, when credentials are re-sent), the old
scheduler loop may still be running while the new one is constructed, creating a brief window
where two scheduler instances race on claim operations. Restructure shutdown to: (1) cancel the
token, (2) `join_handle.await`, (3) only then proceed with new scheduler construction.

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

**[LOW]** Inherited from `scheduler-engine` -- `ScheduledTaskType` enum has no
`#[non_exhaustive]` attribute or `Other(String)` variant. If a new task type is added and a
scheduler binary is not yet updated, deserialization from the DB will fail rather than
gracefully skipping the unknown type. Inconsistent with the forward-compatibility pattern used
in `wire`'s `Capability` and `CloseReason`.

**[LOW]** Inherited from `scheduler-engine` -- `TaskExecutor` has no registration or discovery
mechanism ensuring all `ScheduledTaskType` variants have executors. Unlike `register_plugins!`,
there is no compile-time check that prevents a new variant from being added without a
corresponding executor registration in `main.rs`.
