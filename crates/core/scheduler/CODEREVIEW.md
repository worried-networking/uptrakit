# Code Review — `crates/core/scheduler`

> Review date: 2026-02-28 | Reviewer: AI multi-agent review (6 specialist dimensions)
> Dimensions covered: Architecture · Security & Safety · Code Quality ·
> High Availability · Coding Standards · Extensibility

## Summary

`crates/core/scheduler` is a standalone binary that drives periodic scheduled task execution
for a single tenant. It delegates the actual scheduling mechanics (claim, execute, release) to
`uptrakit-shared-scheduler-engine` and registers task executors at startup. The binary itself
is thin and correctly structured. However, the underlying scheduler engine has a sequential
execution model that can starve tasks when one executor is slow, and the shutdown sequence
has a race between `stop_scheduler()` and the `JoinHandle` it should await.

---

## What's Well-Implemented

- **[Architecture]** The binary correctly separates executor registration (at startup in
  `main.rs`) from execution mechanics (in `scheduler-engine`). Adding a new scheduled task
  type requires only a `Cargo.toml` dependency, a new executor struct, and a single
  `scheduler.register(TaskType, executor)` call.

- **[Architecture]** Uses `CancellationToken` propagation from `service-sdk` for graceful
  shutdown, consistent with the rest of the workspace.

- **[High Availability]** The `recover_stale_claims` mechanism in `scheduler-engine` ensures
  a crashed scheduler instance releases its locks within the stale window (600 s), allowing
  the next instance to pick up tasks without operator intervention.

- **[Coding Standards]** `bail!` / `report!` / `context_to` error handling used correctly
  throughout. No `unwrap()` in production paths.

- **[Security & Safety]** Zero `unsafe` blocks. No database credentials stored in this
  binary; DB connection is obtained through the standard `TenantDb` initialisation path.

---

## What Requires Attention

### Major

- **[High Availability]** `stop_scheduler()` sends the `Disconnecting` message (or similar
  shutdown signal) before awaiting the scheduler `JoinHandle`. If the in-flight task
  completes after the shutdown signal is sent, the task's result (e.g., `release_claim`
  DB update) may be discarded or may race with the connection close. Restructure shutdown
  to: (1) cancel the token, (2) `join_handle.await`, (3) only then signal disconnection.

- **[High Availability]** `scheduler-engine` executes tasks sequentially within each poll
  cycle: if a registered executor is slow, it blocks all other due tasks from running in
  that cycle. Under normal load with many tasks due simultaneously, lower-priority tasks
  are starved. Consider introducing a bounded task-pool (e.g., `tokio::task::JoinSet` with
  a concurrency cap) so independent tasks can run in parallel within a single poll cycle.

- **[Architecture]** `SchedulerNotifier` trait (in `scheduler-engine`) accepts a
  `sea_orm::DatabaseConnection` as a parameter, leaking the ORM type into what should be
  a clean notification interface. Replace with a boxed async closure or a simpler trait
  that does not expose `sea_orm` to callers that should not depend on it.

### Minor

- **[Coding Standards]** `DEFAULT_POLL_INTERVAL_SECS` is a module-private `const u64` inside
  `scheduler-engine/src/scheduler.rs` without documentation. Move to `durations.rs`
  (or an equivalent centralised constants file) as a typed `Duration` with a doc comment,
  consistent with how all other timing constants in the workspace are handled.

- **[Extensibility]** `Scheduler::register` uses `HashMap::insert` which silently replaces
  a previously registered executor for a given task type. A `debug_assert!` that the key is
  not already present would surface accidental double-registration during development without
  impacting production.

### Observations

- **[Extensibility]** `ScheduledTaskType` enum currently has no `Other(String)` variant.
  If a new task type is added in a future release and a scheduler binary is not yet updated,
  deserialization from the DB will fail rather than gracefully skipping the unknown type.
  Consider adding `#[non_exhaustive]` and an `Other(String)` variant consistent with the
  forward-compatibility pattern used in `wire`'s `Capability` and `CloseReason`.

- **[Coding Standards]** `TaskExecutor::execute` has no registration or discovery mechanism
  ensuring all `ScheduledTaskType` variants have executors. Unlike the `register_plugins!`
  macro, there is no compile-time check that prevents a new variant from being added to the
  enum without a corresponding executor registration in `main.rs`.
