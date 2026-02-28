# Code Review — `crates/shared/scheduler-engine`

> Review date: 2026-02-28 | Reviewer: AI multi-agent review (6 specialist dimensions)
> Dimensions covered: Architecture · Security & Safety · Code Quality ·
> High Availability · Coding Standards · Extensibility

## Summary

`uptrakit-shared-scheduler-engine` (~2,172 LOC) implements the database-backed distributed
task scheduler used by both `uptrakit-controller` (embedded) and `uptrakit-scheduler`
(standalone binary). The core claim/release cycle uses TOCTOU-free optimistic locking and
stale-claim recovery, which are strong correctness properties. The primary concerns are an
architectural coupling issue (the `SchedulerNotifier` trait leaks `sea_orm::DatabaseConnection`
into what should be a clean notification boundary) and a sequential execution model that can
starve tasks under concurrent load.

---

## What's Well-Implemented

- **[Architecture]** `try_claim` in `claim.rs` uses a single `UPDATE WHERE locked_by IS NULL`
  atomic operation. Two concurrent scheduler instances racing to claim the same task are
  handled correctly by the DB engine — exactly one wins, no separate read or TOCTOU window.

- **[High Availability]** `recover_stale_claims` releases locks held for more than 600 s,
  providing automatic recovery after a scheduler crash without manual intervention. The
  stale window is a named constant with a doc comment.

- **[High Availability]** `release_all_claims` is called in the `cancelled()` arm of the
  shutdown loop, ensuring a clean shutdown does not leave locks that block the 600 s stale
  window.

- **[Code Quality]** `claim.rs` unit tests are thorough: claim acquisition, double-claim
  rejection, release with success, release with error (run_count not incremented),
  `find_due_tasks` filtering, `release_all_claims` scoped to controller, and
  `trigger_immediate`. All use in-memory SQLite with no inter-test leakage.

- **[Coding Standards]** `thiserror`-derived error types with typed variants; no
  `Result<T, String>` at library boundaries.

- **[Security & Safety]** Zero `unsafe` blocks. The optimistic locking design means no
  application-level mutex is needed for concurrent access.

---

## What Requires Attention

### Major

- **[Architecture]** `src/notifier.rs` — `SchedulerNotifier` trait accepts
  `sea_orm::DatabaseConnection` as a parameter. This leaks the ORM into what should be a
  pure notification interface, and forces every implementor to take on a `sea-orm` dependency
  even if they only want to send a channel message or call a simple async function. Replace
  with a boxed async closure (`Box<dyn Fn(...) -> BoxFuture<...> + Send + Sync>`) or a
  simpler trait that does not reference `sea_orm` types.

- **[High Availability]** `scheduler.rs` — Tasks are executed sequentially within each poll
  cycle. If a registered executor takes 10 seconds, all other due tasks are blocked for the
  full 10 seconds. For deployments with many concurrent due tasks this causes head-of-line
  blocking. Introduce a `tokio::task::JoinSet` with a configurable concurrency limit
  (defaulting to, e.g., 8) so independent tasks can run in parallel while the per-task-type
  executor still provides serialisation at the type level.

- **[Extensibility]** `ScheduledTaskType` enum lacks `#[non_exhaustive]` and an
  `Other(String)` variant. When a new task type is seeded in the DB by a newer migration
  and the scheduler binary has not yet been updated, `find_due_tasks` will attempt to
  deserialise the unknown string and fail, causing the scheduler to crash or skip the row
  with a logged error. Adding `#[non_exhaustive]` and graceful skip-on-unknown handling
  (consistent with `Capability::Other(String)` in the wire crate) would make rolling
  upgrades safe.

### Minor

- **[Coding Standards]** `DEFAULT_POLL_INTERVAL_SECS` is a bare `const u64` inside
  `scheduler.rs` without a doc comment. Promote to a typed `Duration` constant with
  documentation, matching the `durations.rs` pattern used in the controller.

- **[High Availability]** The shutdown sequence in `scheduler.rs` sends the shutdown signal
  before awaiting the in-progress task's `JoinHandle`. This creates a window where the task's
  `release_claim` DB write may be abandoned mid-execution. Await the handle before
  propagating shutdown to callers.

- **[Code Quality]** `Scheduler::register` uses `HashMap::insert` which silently overwrites
  a previously registered executor with no warning. Add a `debug_assert!` that the key is
  absent to surface accidental double-registration during development.

### Observations

- **[Architecture]** The scheduler is currently single-tenant by design: `SchedulerConfig`
  carries a `tenant_id` and all `find_due_tasks` queries filter by it. Any future multi-tenant
  extension requires a redesign of task discovery and assignment. This limitation is not
  documented at the `Scheduler` struct definition site; adding a `// Single-tenant: scoped
  to one tenant_id; multi-tenant requires redesign` comment would make the constraint visible
  to future contributors.

- **[Extensibility]** `TaskExecutor` trait has no compile-time check ensuring all
  `ScheduledTaskType` variants have registered executors. Unlike `register_plugins!`, there
  is no mechanism to catch a missing executor at startup — the scheduler will silently skip
  tasks of an unregistered type.
